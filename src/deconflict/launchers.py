"""External tool launching, coupled to file type. Blocking: run tool, wait for user to continue."""

from __future__ import annotations

import subprocess
import sys
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path

from .media import file_kind, is_audio, is_image, is_kdbx, is_office, is_video
from .tools import Tools


class ToolType(str, Enum):
    DIFF = "diff"
    EDITOR = "editor"
    VIEW = "view"  # open both files in the kind's suggested app
    VIEWER = "viewer"  # single-file open in a generic viewer
    OFFICE = "office"
    KEEPASS = "keepass"


# Which [tools] key opens/edits each file kind; overridable via config [file_types].
KIND_TOOL: dict[str, str] = {
    "text": "editor",
    "other": "editor",
    "audio": "mp3_editor",
    "image": "image_viewer",
    "video": "viewer",
    "office": "office",
    "kdbx": "keepass",
}


def _launch(cmd: list[str]) -> None:
    print(f"launching: {cmd[0]} …")
    subprocess.run(cmd, check=False)


@dataclass
class Launcher:
    tools: Tools
    file_type_tools: dict[str, str] = field(default_factory=dict)

    def kind_tool(self, kind: str) -> str:
        """The [tools] key that opens/edits `kind` (config override wins)."""
        return self.file_type_tools.get(kind) or KIND_TOOL.get(kind, "editor")

    def suggested(self, path: Path) -> ToolType:
        """The tool most appropriate for `path`'s type (menu 'suggest' hint)."""
        if is_kdbx(path):
            return ToolType.KEEPASS
        if is_office(path):
            return ToolType.OFFICE
        if is_image(path) or is_video(path) or is_audio(path):
            return ToolType.VIEWER
        return ToolType.EDITOR

    def run(self, group, ga, tool: ToolType, target: Path) -> None:
        if not target.exists():
            raise FileNotFoundError(target)
        if tool is ToolType.VIEW:
            self._run_view(group, target)
            return
        cmd = self.command(group, tool, target)
        if not cmd:
            raise RuntimeError(f"no tool configured for {tool.value}")
        _launch(cmd)

    def _run_view(self, group, target: Path) -> None:
        cmds = self.view_commands(group, target)
        if not cmds:
            raise RuntimeError("no viewer/editor available for this file type")
        for cmd in cmds:
            _launch(cmd)

    def view_commands(self, group, target: Path) -> list[list[str]]:
        """Commands opening BOTH files (target + sibling) in the suggested app.

        Editor/office tools take both paths in one command; viewers get one
        command per file (xdg-open/open detach immediately).
        """
        kind = file_kind(target)
        tool_name = self.kind_tool(kind)
        exe = self.tools.get(tool_name)
        if exe is None and kind == "audio":
            exe = self.tools.get("viewer")  # no tag editor -> system audio player
        if exe is None:
            return []
        sibling = next((f for f in group.files if f.resolve() != target.resolve()), None)
        if kind == "kdbx":
            # keepass merge recipe: sibling (base) absorbs the copy's entries
            return [[exe, "merge", str(sibling), str(target)]] if sibling else []
        if kind in ("text", "other", "office"):
            return [[exe, str(target), str(sibling)]] if sibling else [[exe, str(target)]]
        cmds = [[exe, str(target)]]
        if sibling is not None:
            cmds.append([exe, str(sibling)])
        return cmds

    def command(self, group, tool: ToolType, target: Path) -> list[str] | None:
        if tool is ToolType.KEEPASS:
            return self._keepass_cmd(group, target)
        if tool is ToolType.DIFF:
            other = next((c for c in group.files if c.resolve() != target.resolve()), None)
            exe = self.tools.get("diff")
            if not exe:
                return None
            if other is None:
                return [exe, str(target)]
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
