"""Shared per-kind action matrix used by every frontend (CLI menu + TUI buttons).

The matrix maps a file kind to the ordered actions a user can take on the
current group, each as `(hotkey, label, ToolType | inline-tag)`.

- A `ToolType` means "launch an external tool via `Engine.launch_tool()`".
- An inline-tag (`term_diff`, `office_diff`, `meta_view`) means the action is
  handled directly by the frontend (char diff / office text diff / metadata
  table) using the shared `render` renderables.

Keeping this UI-free lets the CLI prompt menu and the TUI action-bar buttons
offer exactly the same actions per file kind.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from .launchers import ToolType

if TYPE_CHECKING:
    from .analyze import GroupAnalysis
    from .engine import Engine


def actions_for(engine: Engine, ga: GroupAnalysis) -> list[tuple[str, str, ToolType | str]]:
    """Ordered (hotkey, label, ToolType | inline-tag) actions for this group's kind."""
    k = ga.kind
    a: list[tuple[str, str, ToolType | str]] = []
    la = engine.launcher()
    view = (not la.view_edit_collapse(k)) and la.view_available(k)
    edit = engine.available_edit(ga)
    if k in ("text", "other"):
        if view:
            a.append(("v", "view", ToolType.VIEW))
        if k == "text":
            a.append(("d", "diff", "term_diff"))
        if edit:
            a.append(("e", "edit", ToolType.EDIT))
    elif k == "audio":
        if view:
            a.append(("v", "listen", ToolType.VIEW))
        if edit:
            a.append(("e", "edit tags", ToolType.EDIT))
        if engine.available_diff(ga):
            a.append(("d", "meta-diff", ToolType.DIFF))
    elif k == "image":
        if view:
            a.append(("v", "view", ToolType.VIEW))
        if edit:
            a.append(("e", "edit", ToolType.EDIT))
        if engine.available_diff(ga):
            a.append(("d", "meta-diff", ToolType.DIFF))
        if engine.available_edit_meta(ga):
            a.append(("x", "edit exif", ToolType.EDIT_META))
    elif k == "video":
        if view:
            a.append(("v", "play", ToolType.VIEW))
        if engine.available_diff(ga):
            a.append(("d", "meta-diff", ToolType.DIFF))
    elif k == "office":
        if view:
            a.append(("o", "pen", ToolType.VIEW))
        if edit:
            a.append(("e", "edit", ToolType.EDIT))
        a.append(("d", "diff", "office_diff"))
    elif k == "kdbx":
        a.append(("v", "merge", ToolType.VIEW))
    # (m)etadata is always available (generic file attrs at minimum).
    a.append(("m", "etadata", "meta_view"))
    return a
