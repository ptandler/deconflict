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
    "office": ["soffice", "libreoffice"],
    "keepass": ["keepassxc-cli"],
    "mp3_editor": ["kid3-cli", "Picard"],
    "video_probe": ["ffprobe"],
    "exiftool": ["exiftool"],
    "viewer": ["xdg-open", "start"],
}


@dataclass(frozen=True)
class Tools:
    """Resolved external tools present on this system."""

    found: dict[str, str]

    def __bool__(self) -> bool:
        return bool(self.found)

    def get(self, name: str) -> str | None:
        return self.found.get(name)

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
