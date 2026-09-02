"""Action entries for the TUI action bar.

Each group exposes an ordered list of `ActionEntry`s: the resolution actions
(keep base / keep copy #N / keep-both / skip / quit) plus the per-kind tool
actions from the shared `actions.actions_for` matrix. Frontends (here the TUI
buttons) render them and dispatch on `kind`.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from ..actions import actions_for
from ..analyze import recommend
from ..launchers import ToolType

if TYPE_CHECKING:
    from ..analyze import GroupAnalysis
    from ..engine import Engine


# Resolution / lifecycle kinds handled directly by the app (not a ToolType).
KEEP_BASE = "keep_base"
KEEP_COPY = "keep_copy"
KEEP_BOTH = "keep_both"
SKIP = "skip"
QUIT = "quit"
RECOMMENDED = "recommended"
# Inline (handled by the frontend using shared render) kinds.
_DIFF = "term_diff"
_OFFICE_DIFF = "office_diff"
_META = "meta_view"


@dataclass(frozen=True)
class ActionEntry:
    hotkey: str
    label: str
    kind: str
    tool: ToolType | None = None  # set when kind is an external ToolType
    target_index: int | None = None  # 0-based copy index for KEEP_COPY
    extra: object = field(default=None, repr=False)


def build_actions(engine: Engine, ga: GroupAnalysis) -> list[ActionEntry]:
    """Ordered action entries for the current group's kind + resolution choices."""
    entries: list[ActionEntry] = []
    rec = recommend(ga)
    if rec is not None:
        entries.append(
            ActionEntry("r", f"keep recommended ({rec.label})", RECOMMENDED, extra=rec.is_base)
        )
    if ga.base is not None:
        entries.append(ActionEntry("0", "keep base", KEEP_BASE))
    for i, _c in enumerate(ga.copies):
        entries.append(ActionEntry(str(i + 1), f"keep copy {i + 1}", KEEP_COPY, target_index=i))
    entries.append(ActionEntry("h", "keep-both", KEEP_BOTH))
    entries.append(ActionEntry("s", "skip", SKIP))
    for hotkey, label, tool in actions_for(engine, ga):
        if isinstance(tool, ToolType):  # ToolType is a str-Enum; check it FIRST
            entries.append(ActionEntry(hotkey, label, "tool", tool=tool))
        else:  # inline tag (term_diff / office_diff / meta_view)
            entries.append(ActionEntry(hotkey, label, str(tool)))
    entries.append(ActionEntry("q", "quit", QUIT))
    return entries
