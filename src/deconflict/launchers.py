"""External tool launching, coupled to file type. Blocking: run tool, wait for user to continue."""

from __future__ import annotations

import subprocess
import sys
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

from .media import is_audio, is_image, is_kdbx, is_office, is_video
from .tools import Tools


class ToolType(str, Enum):
    DIFF = "diff"
    EDITOR = "editor"
    VIEWER = "viewer"
    OFFICE = "office"
    KEEPASS = "keepass"


@dataclass
class Launcher:
    tools: Tools

    def suggested(self, path: Path) -> ToolType:
        """The tool most appropriate for `path`'s type."""
        if is_kdbx(path):
            return ToolType.KEEPASS
        if is_office(path):
            return ToolType.OFFICE
        if is_image(path) or is_video(path):
            return ToolType.VIEWER
        if is_audio(path):
            return ToolType.VIEWER
        return ToolType.EDITOR

    def run(self, group, ga, tool: ToolType, target: Path) -> None:
        if not target.exists():
            raise FileNotFoundError(target)
        cmd = self._command(group, tool, target)
        if not cmd:
            raise RuntimeError(f"no tool configured for {tool.value}")
        print(f"launching: {cmd[0]} … (close it to continue)")
        subprocess.run(cmd, check=False)

    def _command(self, group, tool: ToolType, target: Path) -> list[str] | None:
        if tool is ToolType.KEEPASS:
            return self._keepass_cmd(group, target)
        if tool is ToolType.DIFF:
            other = next((c for c in group.files if c.resolve() != target.resolve()), None)
            exe = self.tools.get("diff")
            if not exe:
                return None
            if other is None:
                return [exe, str(target)]
            if self._is_graphical_diff(exe):
                return [exe, str(other), str(target)]
            return [exe, str(other), str(target)]
        if tool is ToolType.EDITOR:
            exe = self.tools.get("editor")
            return [exe, str(target)] if exe else None
        if tool is ToolType.OFFICE:
            exe = self.tools.get("office")
            return [exe, str(target)] if exe else None
        if tool is ToolType.VIEWER:
            return self._viewer_cmd(target)
        return None

    @staticmethod
    def _is_graphical_diff(exe: str) -> bool:
        return "meld" in exe or "WinMerge" in exe

    def _viewer_cmd(self, target: Path) -> list[str] | None:
        exe = self.tools.get("image_viewer") or self.tools.get("viewer")
        if not exe:
            return None
        if exe == "start":  # windows shell keyword
            return None
        if sys.platform == "darwin":
            return ["open", str(target)]
        if sys.platform.startswith("win"):
            return [exe, str(target)]
        if exe == "xdg-open":
            return [exe, str(target)]
        return [exe, str(target)]

    def _keepass_cmd(self, group, target: Path) -> list[str] | None:
        exe = self.tools.get("keepass")
        if not exe:
            return None
        other = next((c for c in group.files if c.resolve() != target.resolve()), None)
        if other is None:
            return None
        return [exe, "merge", str(other), str(target)]
