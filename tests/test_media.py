"""Media extraction tests against the real mp3/jpg/office fixtures."""

from __future__ import annotations

from pathlib import Path

from deconflict import media
from deconflict.patterns import build_patterns
from deconflict.scan import scan

PATTERNS = build_patterns()

MP3 = "Fröhlicher Kreis - Track12 Scottish Circassian, Irish Washerwoman, My Old Man.mp3"


def _pair(sample_dir, base_name):
    result = scan([sample_dir], PATTERNS)
    g = next(g for g in result.groups if g.base.name == base_name)
    return g.base, g.conflicts[0]


def test_audio_is_audio(sample_dir):
    base, _copy = _pair(sample_dir, MP3)
    assert media.is_audio(base)
    assert media.is_audio(_copy)


def test_audio_summary_differs(sample_dir):
    base, copy = _pair(sample_dir, MP3)
    b = media.audio_summary(base)
    c = media.audio_summary(copy)
    assert "title" in b or "album" in b
    assert b != c  # real tag-only diff proves metadata value


def test_image_summary_has_dims(sample_dir):
    base, _copy = _pair(sample_dir, "IMG-20181122-WA0004.jpg")
    s = media.image_summary(base)
    assert "x" in s
    assert media.is_image(base)


def test_office_summary_extracts_text(sample_dir):
    for name, _ext in [("Kostenauflistung 2023.xlsx", ".xlsx")]:
        base, _copy = _pair(sample_dir, name)
        s = media.office_summary(base)
        assert len(s) > 0
        assert media.is_office(base)


def test_office_and_content_differs(sample_dir):
    base, copy = _pair(sample_dir, "Kostenauflistung 2023.xlsx")
    assert media.office_summary(base) != media.office_summary(copy)


def test_kdbx_detection():
    from pathlib import Path

    assert media.is_kdbx(Path("db.kdbx"))
    assert not media.is_kdbx(Path("db.txt"))


def test_media_supported_dispatch(sample_dir):
    base, _ = _pair(sample_dir, "Readme.md")
    assert media.media_supported(base) is False
    img, _ = _pair(sample_dir, "IMG-20181122-WA0004.jpg")
    assert media.media_supported(img) is True


def test_file_kind_classification():
    from pathlib import Path

    assert media.file_kind(Path("a.md")) == "text"
    assert media.file_kind(Path("a.mp3")) == "audio"
    assert media.file_kind(Path("a.jpg")) == "image"
    assert media.file_kind(Path("a.ods")) == "office"
    assert media.file_kind(Path("a.kdbx")) == "kdbx"
    assert media.file_kind(Path("a.bin"), is_text=False) == "other"
    assert media.file_kind(Path("a.bin"), is_text=True) == "text"


def test_metadata_equal_on_tag_diff(sample_dir):
    base, copy = _pair(sample_dir, MP3)
    assert media.metadata_equal(base, copy, "audio") is False
    same = media.metadata_equal(base, base, "audio")
    assert same is True


def test_metadata_diff_mentions_tags(sample_dir):
    base, copy = _pair(sample_dir, MP3)
    diff = media.metadata_diff(base, copy, "audio")
    assert diff is not None
    assert "tags differ" in diff


def test_metadata_diff_none_for_text(sample_dir):
    base, copy = _pair(sample_dir, "Readme.md")
    assert media.metadata_diff(base, copy, "text") is None


def test_metadata_fields_audio(sample_dir):
    base, copy = _pair(sample_dir, MP3)
    bf = media.metadata_fields(base, "audio")
    cf = media.metadata_fields(copy, "audio")
    assert isinstance(bf, dict) and bf  # at least one extractable tag
    assert isinstance(cf, dict) and cf
    # some field must differ (real tag-only diff) for the menu to make sense
    assert any(bf.get(k) != cf.get(k) for k in dict.fromkeys([*bf, *cf]))


def test_metadata_fields_image_has_dims_and_format(sample_dir):
    base, _copy = _pair(sample_dir, "IMG-20181122-WA0004.jpg")
    fields = media.metadata_fields(base, "image")
    assert fields.get("dimensions") is not None
    assert fields.get("format") is not None


def test_metadata_fields_generic_attrs_for_unsupported(sample_dir):
    """Even kinds without content metadata (text/other) get the generic file attrs,
    so (m)etadata is always available (size/created/modified)."""
    base, copy = _pair(sample_dir, "Readme.md")
    fields = media.metadata_fields(base, "text")
    assert set(fields) == set(media.META_ATTR_FIELDS)
    assert fields.get("size") is not None


def test_metadata_equal_and_diff_share_extraction_for_images(sample_dir):
    """item: overview must not say 'metadata same' while the diff shows differences —
    both derive from the same `_content_fields` extractor."""
    base, copy = _pair(sample_dir, "IMG-20181122-WA0004.jpg")
    bf = media._content_fields(base, "image", {})
    cf = media._content_fields(copy, "image", {})
    equal = all(bf.get(k) == cf.get(k) for k in dict.fromkeys([*bf, *cf]))
    assert equal == media.metadata_equal(base, copy, "image")


def test_office_metadata_has_core_props_and_clean_text(sample_dir):
    """item: office metadata should be real fields (core props + cleaned text), not raw XML."""
    base, _copy = _pair(sample_dir, "Vorlage_Transkription.docx")
    fields = media.metadata_fields(base, "office")
    assert "title" in fields and "creator" in fields
    assert "last_modified_by" in fields
    content = fields.get("content", "")
    # cleaned text: readable words, no XML markup angle-brackets
    assert "<" not in content and ">" not in content
    assert content.strip()


def test_metadata_fields_include_generic_attrs(sample_dir):
    """item: the metadata table always shows size + created + modified, even for media."""
    base, _copy = _pair(sample_dir, MP3)
    fields = media.metadata_fields(base, "audio")
    for attr in media.META_ATTR_FIELDS:
        assert fields.get(attr) is not None
    # content tags still present alongside the generic attrs
    assert any(k not in media.META_ATTR_FIELDS for k in fields)


def test_office_text_falls_back_to_zip_without_converter(sample_dir):
    """item: office_text uses our zip walk when no external converter is configured."""
    base, _copy = _pair(sample_dir, "Kostenauflistung 2023.xlsx")
    text = media.office_text(base, {})
    assert text.strip()
    # content is a readable cell dump, not raw XML markup
    assert "Summe" in text


def test_office_text_uses_converter_when_available(sample_dir, monkeypatch, tmp_path):
    """item: when [tools].office_text is set, its output wins over the zip walk."""
    base, _copy = _pair(sample_dir, "Kostenauflistung 2023.xlsx")
    sentinel = "FAKE CONVERTER OUTPUT"
    script = tmp_path / "fake_conv"
    script.write_text(f"#!/bin/sh\nprintf '%s\\n' '{sentinel}'\n")
    script.chmod(0o755)
    monkeypatch.setattr(media, "_OFFICE_TEXT_CACHE", {})
    assert media.office_text(base, {"office_text": str(script)}) == sentinel
    monkeypatch.setattr(media, "_OFFICE_TEXT_CACHE", {})


def test_office_text_converter_fallback_on_error(sample_dir, tmp_path, monkeypatch):
    """item: a failing converter returns to the zip-based extraction."""
    base, _copy = _pair(sample_dir, "Kostenauflistung 2023.xlsx")
    bad = tmp_path / "bad_conv"
    bad.write_text("#!/bin/sh\nexit 1\n")
    bad.chmod(0o755)
    monkeypatch.setattr(media, "_OFFICE_TEXT_CACHE", {})
    assert media.office_text(base, {"office_text": str(bad)})


class _FakeProc:
    def __init__(self, returncode: int, stdout: str):
        self.returncode = returncode
        self.stdout = stdout


_FAKE_EXIF = """[{
  "SourceFile": "a.mp3",
  "ExifToolVersion": 13.55,
  "FileName": "a.mp3",
  "FileSize": "3.6 MB",
  "MIMEType": "audio/mpeg",
  "MPEGAudioVersion": 1,
  "SampleRate": 44100,
  "CopyrightFlag": false,
  "Title": "A Song",
  "Artist": "Someone",
  "Genre": "Volkstanz",
  "Comment": "",
  "Track": 12
}]"""


def test_exiftool_fields_parses_and_normalizes(monkeypatch):
    """item: exiftool enriches audio/image metadata; noise is skipped, acronym keys
    normalized (MIMEType->mimeType, MPEGAudioVersion->mpegAudioVersion), empty and
    file-system fields excluded. Mocked subprocess keeps the test fast/deterministic."""
    monkeypatch.setattr(media.subprocess, "run", lambda *a, **k: _FakeProc(0, _FAKE_EXIF))
    fields = media._exiftool_fields(Path("a.mp3"), "/usr/bin/exiftool")
    assert fields is not None
    # normalized acronym keys
    assert fields.get("mimeType") == "audio/mpeg"
    assert fields.get("mpegAudioVersion") == "1"
    # scalar/string tags present
    assert fields.get("title") == "A Song"
    assert fields.get("artist") == "Someone"
    assert fields.get("genre") == "Volkstanz"
    assert fields.get("track") == "12"
    assert fields.get("copyrightFlag") == "false"
    # noise + empty values excluded
    for noisy in ("SourceFile", "FileName", "FileSize", "ExifToolVersion", "comment"):
        assert noisy not in fields


def test_exiftool_fields_dims_merged_for_image(monkeypatch):
    """ImageWidth/Height are folded into a single `dimensions` field for images."""
    exif = """[{"ImageWidth": 1200, "ImageHeight": 1600, "FileType": "JPEG",
                "ImageSize": "1200x1600", "Megapixels": 1.9, "JFIFVersion": "1.02"}]"""
    monkeypatch.setattr(media.subprocess, "run", lambda *a, **k: _FakeProc(0, exif))
    fields = media._exiftool_fields(Path("a.jpg"), "/usr/bin/exiftool", image=True)
    assert fields is not None
    assert fields.get("dimensions") == "1200x1600"
    assert fields.get("jfifVersion") == "1.02"
    # redundancy dropped, file-type noise dropped
    assert "imageWidth" not in fields and "imageHeight" not in fields
    assert "imageSize" not in fields and "megapixels" not in fields
    assert "fileType" not in fields


def test_exiftool_fields_error_falls_back_to_none(monkeypatch):
    """A failing/missing exiftool returns None so callers fall back to abrupt extractors."""
    monkeypatch.setattr(media.subprocess, "run", lambda *a, **k: _FakeProc(1, ""))
    assert media._exiftool_fields(Path("a.mp3"), "/usr/bin/exiftool") is None
    assert media._exiftool_fields(Path("a.mp3"), "") is None
    assert media._exiftool_fields(Path("a.mp3"), None) is None


def test_content_fields_uses_exiftool_or_falls_back(sample_dir, monkeypatch):
    """audio/image content extraction uses exiftool when available, mutagen/PIL otherwise."""
    base, _copy = _pair(sample_dir, MP3)
    tools = {"exiftool": "/usr/bin/exiftool"}
    monkeypatch.setattr(media.subprocess, "run", lambda *a, **k: _FakeProc(0, _FAKE_EXIF))
    rich = media._content_fields(base, "audio", tools)
    assert rich.get("artist") == "Someone"  # from mocked exiftool
    # without exiftool the mutagen fallback still yields tags
    fallback = media._content_fields(base, "audio", {})
    assert fallback.get("artist") == "Freds Folks"
