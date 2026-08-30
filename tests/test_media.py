"""Media extraction tests against the real mp3/jpg/office fixtures."""

from __future__ import annotations

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
