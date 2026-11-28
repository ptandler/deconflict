"""External tool launching, coupled to file type. Blocking: run tool, wait for user to continue."""

from __future__ import annotations

import subprocess
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path

from .media import file_kind, is_kdbx, is_office
from .tools import Tools


class ToolType(str, Enum):
    VIEW = "view"  # open file(s) in a viewer/player/office for comparison
    EDIT = "edit"  # open target in an editor/creator (tag editor, image editor, text editor)
    DIFF = "diff"  # graphical/universal diff (meld for text, exiftool -diff for media)
    VIEW_META = "view_meta"  # show the built-in metadata diff table (CLI-rendered)
    EDIT_META = "edit_meta"  # open a metadata/tag editor (tag editor / exiftool)


# Which [tools] key opens each file kind for VIEWING (config [file_types] overrides).
KIND_VIEW_TOOL: dict[str, str] = {
    "text": "editor",
    "other": "editor",
    "audio": "audio_player",
    "image": "image_viewer",
    "video": "viewer",
    "office": "office",
    "kdbx": "keepass",
}

# Which [tools] key opens each file kind for EDITING (config [file_types_edit] overrides).
KIND_EDIT_TOOL: dict[str, str] = {
    "text": "editor",
    "other": "editor",
    "audio": "mp3_editor",  # tag editor
    "image": "image_editor",
    "video": "editor",
    "office": "office",
    "kdbx": "keepass",
}

# Kinds that support editing their metadata, and the [tools] key that edits it.
KIND_META_EDIT_TOOL: dict[str, str] = {
    "audio": "mp3_editor",
    "image": "exiftool",
}


def _launch(cmd: list[str], block: bool = False) -> None:
    # Show the full command (including the file paths being opened) so the user
    # knows exactly what tools act on which files — not just the executable.
    parts = [a if " " not in a and "'" not in a else f'"{a}"' for a in cmd]
    print("launching: " + " ".join(parts))
    if block:
        # Blocking: wait for the tool to exit (e.g. a headless merge recipe like
        # keepassxc-cli merge, whose result the user decides on next).
        subprocess.run(cmd, check=False)
    else:
        # Non-blocking GUI/player launch: detach so the app stays open and the
        # interactive menu is free again (the user goes straight back to actions).
        subprocess.Popen(cmd, start_new_session=True)


def _sibling(group, target: Path) -> Path | None:
    for f in group.files:
        if f.resolve() != target.resolve():
            return f
    return None


@dataclass
class Launcher:
    tools: Tools
    file_type_tools: dict[str, str] = field(default_factory=dict)  # kind -> view tool key
    file_type_edit_tools: dict[str, str] = field(default_factory=dict)  # kind -> edit tool key

    # -- tool key resolution ------------------------------------------------
    def kind_tool(self, kind: str) -> str:
        """The [tools] key that opens/views `kind` (config override wins)."""
        return self.file_type_tools.get(kind) or KIND_VIEW_TOOL.get(kind, "editor")

    def edit_tool(self, kind: str) -> str:
        """The [tools] key that edits `kind` (config override wins)."""
        return self.file_type_edit_tools.get(kind) or KIND_EDIT_TOOL.get(kind, "editor")

    # -- which actions are possible for a kind ------------------------------
    def view_available(self, kind: str) -> bool:
        return self._kind_exe(kind, self.kind_tool) is not None or (
            self._fallback_viewer(kind) is not None
        )

    def edit_available(self, kind: str) -> bool:
        return self._kind_exe(kind, self.edit_tool) is not None

    def diff_available(self, kind: str) -> bool:
        if kind in ("text", "other"):
            return self.tools.get("diff") is not None
        if kind == "office" and self.tools.get("office"):
            return True
        return self.tools.get("exiftool") is not None

    def graphical_diff(self, kind: str) -> bool:
        """True when the configured diff tool is a GUI diff (meld/WinMerge).

        Frontends then launch it as an external tool instead of rendering the
        inline terminal diff.
        """
        if kind not in ("text", "other"):
            return False
        exe = self.tools.get("diff")
        return exe is not None and self._is_graphical_diff(exe)

    def runs_in_terminal(self, tool: ToolType, kind: str) -> bool:
        """True when `tool` launches a foreground terminal program for `kind`.

        Text/other files open in the configured editor, which is typically a
        terminal editor (vi/nano/fresh) that needs the terminal to itself — the
        TUI suspends around those. GUI launches (meld, LibreOffice, players) stay
        detached.
        """
        return kind in ("text", "other") and tool in (ToolType.VIEW, ToolType.EDIT)

    def edit_meta_available(self, kind: str) -> bool:
        return kind in KIND_META_EDIT_TOOL and self._kind_exe(kind, self.edit_tool) is not None

    def view_edit_collapse(self, kind: str) -> bool:
        """True when VIEW and EDIT resolve to the same executable, so showing both
        is meaningless (e.g. LibreOffice opens the doc either way). The CLI then
        offers only (e)dit."""
        v = self._kind_exe(kind, self.kind_tool) or self._fallback_viewer(kind)
        e = self._kind_exe(kind, self.edit_tool)
        return v is not None and e is not None and v == e

    def _kind_exe(self, kind: str, resolver) -> str | None:
        key = resolver(kind)
        return self.tools.get(key)

    def _fallback_viewer(self, kind: str) -> str | None:
        """System-default opener when a kind has no dedicated viewer (video/audio)."""
        if kind in ("image", "video", "audio"):
            return self.tools.get("viewer")
        return None

    def tools_for(self, kind: str) -> list[str]:
        """The [tools] keys that back this kind's actions, in menu relevance order.

        Used by (?)tools so it lists only what is actually run for this file, not
        every configured tool.
        """
        keys: list[str] = []
        view_key = self.kind_tool(kind)
        keys.append(view_key)
        edit_key = self.edit_tool(kind)
        if edit_key != view_key:
            keys.append(edit_key)
        meta_key = KIND_META_EDIT_TOOL.get(kind)
        if meta_key and meta_key not in keys:
            keys.append(meta_key)
        if kind in ("text", "other"):
            keys.append("diff")
        elif kind != "office" and "exiftool" not in keys:
            # media kinds diff via the universal exiftool
            keys.append("exiftool")
        return list(dict.fromkeys(keys))

    # -- command construction -----------------------------------------------
    def run(self, group, ga, tool: ToolType, target: Path, blocking: bool | None = None) -> None:
        """Run the tool(s) for `tool`.

        GUI launches are NON-blocking (detached) so the app stays open and the
        interactive menu returns immediately; the only default blocking case is
        the kdbx keepass merge recipe, whose result the user must see before
        deciding. Pass `blocking=True` to force a foreground run (e.g. a terminal
        editor that needs the terminal to itself) or `False` to force a detach.
        """
        if tool is ToolType.VIEW:
            cmds = self.view_commands(group, target)
            if not cmds:
                raise RuntimeError("no viewer/player available for this file type")
            merge = file_kind(target) == "kdbx"
            block = merge if blocking is None else blocking
            for cmd in cmds:
                _launch(cmd, block=block)
            return
        cmd = self.command(group, tool, target)
        if not cmd:
            raise RuntimeError(f"no tool configured for {tool.value}")
        _launch(cmd, block=False if blocking is None else blocking)

    def view_commands(self, group, target: Path) -> list[list[str]]:
        """Commands opening BOTH files (target + sibling) for comparison.

        Editor/office take both paths in one command; players/viewers get one
        command per file (they detach or block until closed).
        """
        kind = file_kind(target)
        exe = self._kind_exe(kind, self.kind_tool) or self._fallback_viewer(kind)
        if exe is None:
            return []
        sibling = _sibling(group, target)
        if kind == "kdbx":
            return [[exe, "merge", str(sibling), str(target)]] if sibling else []
        if kind in ("text", "other", "office"):
            return [[exe, str(target), str(sibling)]] if sibling else [[exe, str(target)]]
        cmds = [[exe, str(target)]]
        if sibling is not None:
            cmds.append([exe, str(sibling)])
        return cmds

    def command(self, group, tool: ToolType, target: Path) -> list[str] | None:
        if tool is ToolType.DIFF:
            return self._diff_cmd(group, target)
        if tool is ToolType.EDIT:
            return self._edit_cmd(group, target)
        if tool is ToolType.EDIT_META:
            return self._edit_meta_cmd(group, target)
        if tool is ToolType.VIEW:
            first = self.view_commands(group, target)
            return first[0] if first else None
        return None

    def _diff_cmd(self, group, target: Path) -> list[str] | None:
        """Type-aware diff: meld for text, LibreOffice compare for office, and
        ExifTool `-diff` (universal metadata diff) for audio/image/video/office."""
        kind = file_kind(target)
        other = _sibling(group, target)
        if kind in ("text", "other"):
            exe = self.tools.get("diff")
            if not exe or other is None:
                return None
            return [exe, str(other), str(target)]
        if kind == "office":
            exe = self.tools.get("office")
            if exe and other is not None:  # LibreOffice two-doc compare
                return [exe, "--compare", str(target), str(other)]
            exe = self.tools.get("exiftool")
            if exe and other is not None:
                return [exe, "-diff", str(other), str(target), "--system:all", "-s", "-a"]
            return None
        # audio / image / video (and office meta): universal metadata diff
        exe = self.tools.get("exiftool")
        if not exe or other is None:
            return None
        return [exe, "-diff", str(other), str(target), "--system:all", "-s", "-a"]

    def _edit_cmd(self, group, target: Path) -> list[str] | None:
        kind = file_kind(target)
        exe = self._kind_exe(kind, self.edit_tool)
        if not exe:
            return None
        # For single-app kinds (text/other/office) the same exe IS the viewer, so
        # edit should open BOTH files side-by-side for comparison (not just the
        # one copy) — fixes "edit opens only the copy" for ODT/doc same-app case.
        if kind in ("text", "other", "office"):
            sibling = _sibling(group, target)
            return [exe, str(target), str(sibling)] if sibling else [exe, str(target)]
        return [exe, str(target)]

    def _edit_meta_cmd(self, group, target: Path) -> list[str] | None:
        kind = file_kind(target)
        key = KIND_META_EDIT_TOOL.get(kind)
        if not key:
            return None
        exe = self.tools.get(key)
        return [exe, str(target)] if exe else None

    # -- retained helpers ---------------------------------------------------
    def suggested(self, path: Path) -> ToolType:
        """The tool most appropriate for `path`'s type (menu 'suggest' hint)."""
        if is_kdbx(path) or is_office(path):
            return ToolType.VIEW
        return ToolType.EDIT if file_kind(path) in ("audio", "image") else ToolType.VIEW

    @staticmethod
    def _is_graphical_diff(exe: str) -> bool:
        return "meld" in exe or "WinMerge" in exe
