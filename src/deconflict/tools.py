"""External tool detection. Never hard-require; per-OS defaults via shutil.which."""

from __future__ import annotations

import os
import shutil
from collections.abc import Mapping
from dataclasses import dataclass

DEFAULTS: dict[str, list[str]] = {
    "diff": ["meld", "WinMergeU.exe"],
    "editor": [os.environ.get("EDITOR", "vi") or "vi"],
    "image_viewer": (["eog", "xdg-open"] if os.name != "nt" else ["mspaint", "start"]),
    "image_editor": ["gimp", "pinta", "kolourpaint"],
    "office": ["soffice", "libreoffice"],
    "keepass": ["keepassxc-cli"],
    # Prefer GUI tag editors over the CLI; kid3-cli stays a last-resort fallback.
    "mp3_editor": ["kid3", "easytag", "picard", "kid3-cli"],
    "audio_player": ["mpv", "vlc", "ffplay", "totem", "xdg-open"],
    "video_probe": ["ffprobe"],
    "exiftool": ["exiftool"],
    "viewer": ["xdg-open", "start"],
}

INSTALL_HINTS: dict[str, str] = {
    "diff": "meld: apt install meld  ·  brew install meld",
    "editor": "set $EDITOR, or override [tools] editor in config",
    "image_viewer": "eog: apt install eog  ·  brew install eog",
    "image_editor": "gimp: apt install gimp  ·  brew install --cask gimp",
    "office": "soffice: apt install libreoffice  ·  brew install --cask libreoffice",
    "keepass": "keepassxc-cli: apt install keepassxc  ·  brew install keepassxc",
    "mp3_editor": "kid3/easytag: apt install kid3 easytag  ·  brew install --cask kid3 easytag",
    "audio_player": "mpv/vlc: apt install mpv vlc  ·  brew install --cask vlc",
    "video_probe": "ffprobe: apt install ffmpeg  ·  brew install ffmpeg",
    "exiftool": "exiftool -diff: apt install libimage-exiftool-perl  ·  brew install exiftool",
    "viewer": "xdg-open: your system default opener, no install needed",
}


@dataclass(frozen=True)
class Tools:
    """Resolved external tools present on this system."""

    found: dict[str, str]

    def __bool__(self) -> bool:
        return bool(self.found)

    def get(self, name: str) -> str | None:
        return self.found.get(name)

    def names(self) -> list[str]:
        return list(DEFAULTS)

    def candidates(self, name: str) -> list[str]:
        return _candidates(name)

    def hint(self, name: str) -> str | None:
        return INSTALL_HINTS.get(name)

    def require(self, name: str, action: str) -> str:
        tool = self.get(name)
        if not tool:
            raise ToolMissing(name, action)
        return tool


class ToolMissing(RuntimeError):
    def __init__(self, name: str, action: str) -> None:
        msg = f"{action} requires '{name}', which was not found. Install it or set it in config."
        super().__init__(msg)
        self.name = name


def _candidates(name: str) -> list[str]:
    return DEFAULTS.get(name, [name])


def detect(overrides: Mapping[str, str] | None = None) -> Tools:
    """Detect available external tools. `overrides` map config tool -> path."""
    found: dict[str, str] = {}
    overrides = overrides or {}
    for name in DEFAULTS:
        override = overrides.get(name)
        if override:
            found[name] = override
            continue
        for cand in _candidates(name):
            path = shutil.which(cand)
            if path:
                found[name] = path
                break
    return Tools(found)
