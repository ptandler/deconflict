"""Media & office content extraction for diffing. UI-free; external tools are helpers."""

from __future__ import annotations

import datetime
import json
import re
import subprocess
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

_AUDIO_EXT = {".mp3", ".flac", ".ogg", ".oga", ".m4a", ".opus", ".wav", ".aiff", ".ape"}
_IMAGE_EXT = {".jpg", ".jpeg", ".png", ".gif", ".bmp", ".webp", ".tif", ".tiff"}
_VIDEO_EXT = {".mp4", ".mkv", ".avi", ".mov", ".webm", ".m4v", ".flv", ".wmv"}
_OFFICE_EXT = {".docx", ".xlsx", ".pptx", ".ods", ".odt", ".odp"}
_OFFICE_TEXT_PARTS = (
    "word/document.xml",
    "xl/sharedStrings.xml",
    "xl/worksheets/sheet",
    "ppt/slides/slide",
    "content.xml",
)


def _has_ext(path: Path, exts: set[str]) -> bool:
    return path.suffix.lower() in exts


def is_audio(path: Path) -> bool:
    return _has_ext(path, _AUDIO_EXT)


def is_image(path: Path) -> bool:
    return _has_ext(path, _IMAGE_EXT)


def is_video(path: Path) -> bool:
    return _has_ext(path, _VIDEO_EXT)


def is_office(path: Path) -> bool:
    return _has_ext(path, _OFFICE_EXT)


def is_kdbx(path: Path) -> bool:
    return path.suffix.lower() == ".kdbx"


KINDS = ("audio", "image", "video", "office", "kdbx", "text", "other")
"""File kinds the CLI understands; each maps to a suggested tool."""


def file_kind(path: Path, is_text: bool | None = None) -> str:
    """Classify `path` into one of KINDS for tool selection.

    Extension-based kinds win; anything else follows the content sniff
    (`is_text`) and unknown-but-likely-text files fall back to "text".
    """
    if is_kdbx(path):
        return "kdbx"
    if is_audio(path):
        return "audio"
    if is_image(path):
        return "image"
    if is_video(path):
        return "video"
    if is_office(path):
        return "office"
    if is_text is False:
        return "other"
    return "text"


_ID3_FIELDS = {
    "TIT2": "title",
    "TPE1": "artist",
    "TALB": "album",
    "TDRC": "date",
    "TCON": "genre",
    "TRCK": "tracknumber",
}


def audio_summary(path: Path) -> str:
    """Return a metadata text summary (ID3/Vorbis/etc.) — best-effort, for diffing."""
    try:
        from mutagen import File

        audio = File(str(path))
    except Exception:
        return "<unreadable audio>"
    if audio is None:
        return "<no audio tags>"
    tags = getattr(audio, "tags", None)
    fields = []
    if tags is not None:
        for frame, label in _ID3_FIELDS.items():
            value = _get_tag(tags, frame)
            if value is not None:
                fields.append(f"{label}={value}")
    if not fields:
        length = getattr(getattr(audio, "info", None), "length", None)
        return f"length={length}" if length is not None else "<no tags>"
    return ", ".join(fields)


def _get_tag(tags, frame: str):
    """Read an ID3 frame, tolerating missing / list-valued frames."""
    try:
        value = tags.get(frame)
    except Exception:
        return None
    if value is None:
        return None
    if isinstance(value, str):
        return value
    text = getattr(value, "text", value)
    if isinstance(text, (list, tuple)):
        return " / ".join(str(t) for t in text)
    return str(text)


def image_summary(path: Path) -> str:
    """Return image dimensions + EXIF snippet (best-effort)."""
    try:
        from PIL import Image
        from PIL.ExifTags import TAGS

        with Image.open(path) as im:
            dims = f"{im.width}x{im.height} {im.format or ''}".strip()
            exif = im.getexif()
            tags = {TAGS.get(k, k): v for k, v in exif.items()}
            if tags:
                return f"{dims}; exif[{len(tags)}] {tags}"
            return dims
    except Exception:
        return "<unreadable image>"


def video_summary(path: Path, ffprobe: str | None) -> str:
    """Return ffprobe stream summary or best-effort dims."""
    if ffprobe:
        try:
            import json
            import subprocess

            out = subprocess.run(
                [ffprobe, "-v", "error", "-print_format", "json", "-show_streams", str(path)],
                capture_output=True,
                text=True,
                check=True,
            ).stdout
            data = json.loads(out)
            parts = []
            for s in data.get("streams", []):
                dims = s.get("width") and f" {s.get('width')}x{s.get('height')}"
                parts.append(f"{s.get('codec_type')}:{s.get('codec_name')}{dims}")
            return ", ".join(parts) if parts else "<no streams>"
        except Exception:
            return "<ffprobe failed>"
    try:
        from PIL import Image

        with Image.open(path) as im:
            return f"{im.width}x{im.height} (thumbnail)"
    except Exception:
        return "<unknown video>"


def office_summary(path: Path) -> str:
    """Extract text from zip-based office docs (docx/xlsx/pptx/ods...) via stdlib."""
    try:
        with zipfile.ZipFile(path) as z:
            text_parts: list[str] = []
            for member in z.namelist():
                if not member.startswith(_OFFICE_TEXT_PARTS):
                    continue
                try:
                    content = z.read(member)
                except Exception:
                    continue
                text_parts.append(content.decode("utf-8", errors="replace"))
        if not text_parts:
            return "<no text parts>"
        joined = "\n".join(text_parts)
        return joined[:4000]
    except Exception:
        return "<unreadable office>"


def summary(path: Path, tools: dict[str, str] | None = None) -> str:
    """Choose the right extractor for `path`. returns a text summary for diffing."""
    tools = tools or {}
    if is_audio(path):
        return audio_summary(path)
    if is_image(path):
        return image_summary(path)
    if is_video(path):
        return video_summary(path, tools.get("video_probe"))
    if is_office(path):
        return office_summary(path)
    return "<no media extractor>"


def media_supported(path: Path) -> bool:
    return any((is_audio(path), is_image(path), is_video(path), is_office(path)))


# Field names for the generic file-attribute block shown for EVERY file kind.
# These are filesystem-level and intentionally excluded from *content* equality.
ATTR_SIZE = "size"
ATTR_CREATED = "created"
ATTR_MODIFIED = "modified"
META_ATTR_FIELDS = (ATTR_SIZE, ATTR_CREATED, ATTR_MODIFIED)

# Kinds that carry content metadata (tags/EXIF/streams/office text) beyond the
# generic file attributes.
_META_KINDS = ("audio", "image", "video", "office", "kdbx", "text", "other")


def _fmt_ts(ts: float) -> str:
    return datetime.datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M:%S")


def fs_attrs(path: Path) -> dict[str, str]:
    """Generic file attributes (size + create + modify) for every file kind."""
    st = path.stat()
    # st_birthtime is the true creation time on macOS and some BSDs; POSIX has no
    # true birthtime, so fall back to ctime (metadata-change / often ≈ created).
    birth = getattr(st, "st_birthtime", None)
    return {
        ATTR_SIZE: str(st.st_size),
        ATTR_CREATED: _fmt_ts(birth if birth is not None else st.st_ctime),
        ATTR_MODIFIED: _fmt_ts(st.st_mtime),
    }


def _audio_tags(path: Path) -> dict[str, str]:
    """Return the audio tag summary as an ordered dict (empty on failure)."""
    sm = audio_summary(path)
    if sm.startswith("<"):
        return {}
    out: dict[str, str] = {}
    for part in sm.split(", "):
        if "=" not in part:
            continue
        key, _, value = part.partition("=")
        out[key.strip()] = value.strip()
    return out


def _image_fields(path: Path) -> dict[str, str]:
    """Image metadata as a dict: dimensions plus best-effort EXIF tags."""
    from PIL import Image
    from PIL.ExifTags import TAGS

    fields: dict[str, str] = {}
    with Image.open(path) as im:
        fields["dimensions"] = f"{im.width}x{im.height}"
        fmt = im.format or "unknown"
        fields["format"] = fmt
        exif = im.getexif()
        for k, v in exif.items():
            label = TAGS.get(k, f"tag{k}")
            if isinstance(v, (bytes, bytearray)):
                v = bytes(v).decode("latin-1", errors="replace")
            elif isinstance(v, (tuple, list)):
                v = ", ".join(
                    str(x.decode("latin-1", errors="replace") if isinstance(x, bytes) else x)
                    for x in v
                )
            fields[str(label)] = str(v)
    return fields


def _office_core_props(path: Path) -> dict[str, str]:
    """Read docProps/core.xml (docx/xlsx/pptx) into real per-field metadata."""
    try:
        with zipfile.ZipFile(path) as z:
            if "docProps/core.xml" not in z.namelist():
                return {}
            root = ET.fromstring(z.read("docProps/core.xml"))
    except Exception:
        return {}
    out: dict[str, str] = {}
    # dc: / cp: namespaces, keys kept short
    for tag, label in (
        ("dc:title", "title"),
        ("dc:subject", "subject"),
        ("dc:creator", "creator"),
        ("dc:description", "description"),
        ("cp:lastModifiedBy", "last_modified_by"),
        ("dcterms:created", "created"),
        ("dcterms:modified", "modified"),
    ):
        for el in root.iter():
            if el.tag.endswith(tag.split(":")[1]) and el.text and el.text.strip():
                out.setdefault(label, el.text.strip())
    return out


_W_NS = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
_OD_NS = "urn:oasis:names:tc:opendocument:xmlns:office:1.0"


def _extract_office_text(path: Path, limit: int = 4000) -> str:
    """Extract readable text from office docs, stripping markup (docx/xlsx/pptx/ods)."""
    try:
        with zipfile.ZipFile(path) as z:
            names = z.namelist()
            if "content.xml" in names:  # ODS/ODT (OpenDocument)
                root = ET.fromstring(z.read("content.xml"))
                paras = ["".join(el.itertext()) for el in root.iter() if _is_od_text_p(el)]
                text = "\n".join(p for p in paras if p.strip())
                return text[:limit] if text else "<no text>"
            # OOXML: docx paragraphs, xlsx shared strings + cells, pptx slides
            parts: list[str] = []
            if "word/document.xml" in names:  # docx
                root = ET.fromstring(z.read("word/document.xml"))
                paras = ["".join(el.itertext()) for el in root.iter(_W_NS + "p")]
                parts.append("\n".join(p for p in paras if p.strip()))
            for slide in sorted(n for n in names if n.startswith("ppt/slides/slide")):
                try:
                    root = ET.fromstring(z.read(slide))
                    paras = ["".join(el.itertext()) for el in root.iter(_W_NS + "p")]
                except Exception:
                    continue
                parts.append("\n".join(p for p in paras if p.strip()))
            # xlsx: shared strings (map by index) then worksheet cell values
            if "xl/sharedStrings.xml" in names:
                root = ET.fromstring(z.read("xl/sharedStrings.xml"))
                strs: list[str] = []
                for si in root.iter(
                    "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}si"
                ):
                    strs.append("".join(si.itertext()))
                sst = dict(enumerate(strs))
                for sheet in sorted(
                    n for n in names if re.match(r"xl/worksheets/sheet\d+\.xml", n)
                ):
                    try:
                        root = ET.fromstring(z.read(sheet))
                    except Exception:
                        continue
                    vals = []
                    xml = z.read(sheet).decode("utf-8", "replace")
                    for m in re.finditer(r'<c[^>]*?(?:t="s")?[^>]*>\s*<v>([^<]*)</v>', xml):
                        raw = m.group(1)
                        vals.append(str(sst.get(int(raw))) if raw.isdigit() else raw)
                    if vals:
                        parts.append(", ".join(v for v in vals if v))
            text = "\n".join(p for p in parts if p.strip())
            return text[:limit] if text else "<no text>"
    except Exception:
        return "<unreadable office>"


def _is_od_text_p(el) -> bool:
    return el.tag == f"{{{_OD_NS}}}p" or el.tag.endswith("}p")


def _office_fields(path: Path, tools: dict[str, str] | None = None) -> dict[str, str]:
    """Office metadata: core props first, then the cleaned document text as `content`."""
    fields = dict(_office_core_props(path))
    fields["content"] = office_text(path, tools or {})
    return fields


def office_text(path: Path, tools: dict[str, str] | None = None) -> str:
    """Extract faithful plain text from an office doc.

    Prefer an external converter when available (`[tools].office_text`, e.g.
    `odt2txt`/`pandoc` read the file directly and give fuller output than our
    zip walk), falling back to the dependency-free `_extract_office_text` so the
    feature never hard-requires a tool. `soffice` is not used here because it
    needs `--headless --convert-to txt` (slow, writes temp files).
    """
    conv = (tools or {}).get("office_text")
    if conv:
        _OFFICE_TEXT_CACHE.setdefault(path, {})
        cache = _OFFICE_TEXT_CACHE[path]
        if "value" not in cache:
            cache["value"] = _convert_via(conv, path) or _extract_office_text(path)
        return cache["value"]
    return _extract_office_text(path)


_OFFICE_TEXT_CACHE: dict[Path, dict] = {}


def _convert_via(conv: str, path: Path) -> str | None:
    """Run a CLI text extractor that reads the file directly; None on any error."""
    base = Path(conv).name
    try:
        # pandoc reads plain text to stdout; odt2txt and friends take the file.
        args = [conv, "-t", "plain", str(path)] if base.startswith("pandoc") else [conv, str(path)]
        proc = subprocess.run(args, capture_output=True, text=True, timeout=30, check=False)
        if proc.returncode != 0 or not proc.stdout.strip():
            return None
        return proc.stdout[:4000].strip()
    except Exception:
        return None


def _content_fields(path: Path, kind: str, tools: dict[str, str]) -> dict[str, str]:
    """Kind-specific content metadata (EXCLUDES the generic file attributes).

    This is the canonical extractor for BOTH the overview same/diff verdict and
    the metadata diff table, so the two never disagree (see IMG same-but-differs bug).

    For audio/image the rich metadata comes from ExifTool (`[tools].exiftool`)
    when available, falling back to the lighter mutagen/PIL extractors so the
    feature never hard-requires an external tool.
    """
    if kind == "audio":
        return _exiftool_fields(path, tools.get("exiftool")) or _audio_tags(path)
    if kind == "image":
        return _exiftool_fields(path, tools.get("exiftool"), image=True) or _image_fields(path)
    if kind == "video":
        return {"streams": video_summary(path, tools.get("video_probe"))}
    if kind == "office":
        return _office_fields(path, tools)
    # text / other / kdbx: no content metadata (still get generic file attrs)
    return {}


# ExifTool fields that are pure filesystem noise, redundant with our generic
# attrs, or binary blobs — all excluded so they never cause spurious diffs.
_EXIFTOOL_SKIP = frozenset(
    {
        "SourceFile",
        "ExifToolVersion",
        "FileName",
        "Directory",
        "FileSize",
        "FileModifyDate",
        "FileAccessDate",
        "FileInodeChangeDate",
        "FilePermissions",
        "FileTypeExtension",
        # image redundancy / binary payloads / maker-note blobs
        "ImageSize",
        "Megapixels",
        "FileType",
        "ImageWidth",
        "ImageHeight",
        "ThumbnailImage",
        "ThumbnailLength",
        "ThumbnailOffset",
        "PreviewImage",
        "PreviewImageLength",
        "PreviewImageStart",
        "MakerNoteApple",
        "MakerNoteCanon",
        "MakerNoteNikon",
        "MakerNoteSony",
        "MakerNotes",
    }
)
_EXIFTOOL_MAX_VALUE = 200  # cap individual exiftool tag values in the table


def _exif_value(v) -> str | None:
    """Stringify an exiftool JSON value; None for values we can't show or empties."""
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    if isinstance(v, str):
        s = v.strip()
        return s or None
    if isinstance(v, (list, tuple)):
        parts = [_exif_value(x) for x in v]
        parts = [p for p in parts if p]
        return ", ".join(parts) if parts else None
    return None  # dicts/binary refuse to display


def _exif_key(raw: str) -> str:
    """Title->title, MIMEType->mimeType, MPEGAudioVersion->mpegAudioVersion,
    ID3Size->id3Size (lowercase the leading-acronym block, keep the boundary)."""
    i = 0
    n = len(raw)
    while i < n and raw[i].isupper():
        i += 1
    if i > 1:
        return raw[: i - 1].lower() + raw[i - 1 :]
    return raw[:1].lower() + raw[1:]


def _exiftool_fields(
    path: Path, exiftool: str | None, image: bool = False
) -> dict[str, str] | None:
    """Rich audio/image metadata via ExifTool (`-s -j`). None when unavailable/failed."""
    if not exiftool:
        return None
    try:
        proc = subprocess.run(
            [exiftool, "-s", "-j", str(path)],
            capture_output=True,
            text=True,
            timeout=15,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if proc.returncode != 0:
        return None
    try:
        data = json.loads(proc.stdout or "[]")
    except ValueError:
        return None
    if not (isinstance(data, list) and data) or not isinstance(data[0], dict):
        return None
    obj = data[0]
    fields: dict[str, str] = {}
    width = height = None
    for raw, v in obj.items():
        if raw == "ImageWidth":
            width = _exif_value(v)
            continue
        if raw == "ImageHeight":
            height = _exif_value(v)
            continue
        if raw in _EXIFTOOL_SKIP:
            continue
        sv = _exif_value(v)
        if sv is None or len(sv) > _EXIFTOOL_MAX_VALUE:
            continue
        fields[_exif_key(raw)] = sv
    if image and width and height:
        fields["dimensions"] = f"{width}x{height}"
    return fields or None


def metadata_fields(path: Path, kind: str, tools: dict[str, str] | None = None) -> dict[str, str]:
    """Return `path`'s metadata as an ordered dict of field→value (never empty).

    The generic file attributes (size/created/modified) are always present for
    every kind; content metadata (tags/EXIF/streams/office text) is added for the
    media kinds. This is what the (m)etadata table renders.
    """
    tools = tools or {}
    try:
        fields = dict(fs_attrs(path))
        fields.update(_content_fields(path, kind, tools))
        return fields
    except Exception:
        return {ATTR_SIZE: "", ATTR_CREATED: "", ATTR_MODIFIED: ""}


def metadata_equal(a: Path, b: Path, kind: str, tools: dict[str, str] | None = None) -> bool | None:
    """True when both files carry identical CONTENT metadata (fs attrs excluded)."""
    if kind not in _META_KINDS:
        return None
    return _content_fields(a, kind, tools or {}) == _content_fields(b, kind, tools or {})


def metadata_diff_fields(
    a: Path, b: Path, kind: str, tools: dict[str, str] | None = None
) -> tuple[dict[str, str], dict[str, str]]:
    """Content + fs metadata dicts for (a, b), for the (m)etadata table."""
    tools = tools or {}
    return metadata_fields(a, kind, tools), metadata_fields(b, kind, tools)


def metadata_diff(a: Path, b: Path, kind: str, tools: dict[str, str] | None = None) -> str | None:
    """A short human string of what CONTENT metadata differs (None when equal)."""
    if kind not in _META_KINDS:
        return None
    tools = tools or {}
    ca, cb = _content_fields(a, kind, tools), _content_fields(b, kind, tools)
    changed = [k for k in dict.fromkeys([*ca, *cb]) if ca.get(k) != cb.get(k)]
    if not changed:
        return None
    if kind == "audio":
        bits = ", ".join(f"{k}: {ca.get(k, '∅')} → {cb.get(k, '∅')}" for k in changed)
        return f"tags differ ({bits})"
    if kind in ("image", "video", "office"):
        bits = ", ".join(changed)
        return f"metadata differs ({bits})"
    return "metadata differs"
