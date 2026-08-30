"""Media & office content extraction for diffing. UI-free; external tools are helpers."""

from __future__ import annotations

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
