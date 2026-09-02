"""Reusable pane widgets for the deconflict TUI (textual)."""

from __future__ import annotations

import datetime
from typing import TYPE_CHECKING

from rich.console import Group
from rich.text import Text
from textual.app import ComposeResult
from textual.containers import VerticalScroll
from textual.widgets import DataTable, RichLog, Static

from ..render import human

if TYPE_CHECKING:
    from ..analyze import GroupAnalysis


class GroupTable(DataTable):
    """Left pane: one row per conflict group.

    Columns: marker, base filename, pattern, file count. The app drives group
    selection; mouse clicks arrive as native RowSelected.
    """

    def __init__(self, id: str | None = None) -> None:
        super().__init__(id=id, cursor_type="row", zebra_stripes=True)
        self._row_keys: dict[str, object] = {}  # group.key -> RowKey

    def on_mount(self) -> None:
        self.add_column("", key="marker", width=2)
        self.add_column("group (base)", key="base")
        self.add_column("pattern", key="pattern", width=12)
        self.add_column("#", key="nfiles", width=3)

    def add_group(self, key: str, base_name: str, pattern: str, nfiles: int) -> None:
        row_key = self.add_row("", base_name, pattern or "—", str(nfiles), key=key)
        self._row_keys[key] = row_key

    def set_status(self, key: str, status: str) -> None:
        """Update a group row's marker + colour: '' | 'resolved' (green) | 'skipped' (dim)."""
        row_key = self._row_keys.get(key)
        if row_key is None:
            return
        if status == "resolved":
            marker, style = "✓", "bold green"
        elif status == "skipped":
            marker, style = "▸", "dim"
        else:
            marker, style = " ", ""
        self.update_cell(row_key, "marker", Text(marker, style=style), update_width=False)
        # Re-colour the base-name column so the whole row reads as done.
        base = self.get_row(row_key)[1]
        if isinstance(base, str):
            self.update_cell(row_key, "base", Text(base, style=style), update_width=False)


class FilesTable(DataTable):
    """Right pane 'Files' tab: the metadata-format table for the selected group."""

    def __init__(self, id: str | None = None) -> None:
        super().__init__(id=id, cursor_type="row", zebra_stripes=True)

    def on_mount(self) -> None:
        self.add_column("#", key="num", width=4)
        self.add_column("file", key="file")
        self.add_column("size", key="size", width=16)
        self.add_column("mtime", key="mtime", width=14)
        self.add_column("metadata", key="metadata")

    def prime(self) -> None:
        self.clear(columns=False)
        self.add_row("", "[dim]no group selected[/dim]", "", "", "", key="empty")

    def show(self, ga: GroupAnalysis) -> None:
        self.clear(columns=False)
        rows: list[tuple] = ([(ga.base, None)] if ga.base else []) + [
            (c, i) for i, c in enumerate(ga.copies)
        ]
        for a, idx in rows:
            num = "#0" if idx is None else f"#{idx + 1}"
            if idx is None:
                meta = "—"
            elif idx < len(ga.meta):
                md = ga.meta[idx]
                meta = (
                    "—" if md is None else "[green]same[/green]" if md else "[yellow]diff[/yellow]"
                )
            else:
                meta = "—"
            size = human(a.info.size)
            if idx is not None and ga.base is not None:
                if a.info.sha == ga.base.info.sha:
                    size = f"{size} [dim](=)[/dim]"
                else:
                    delta = a.info.size - ga.base.info.size
                    arrow = "↑" if delta > 0 else "↓"
                    bit = f" {human(abs(delta))}" if delta else ""
                    size = f"{size} [yellow]({arrow}{bit})[/yellow]"
            self.add_row(
                num,
                a.path.name,
                size,
                _fmt_dt(a.info.mtime),
                meta,
                key=str(a.path),
            )


def _fmt_dt(t: float) -> str:
    return datetime.datetime.fromtimestamp(t).strftime("%Y-%m-%d %H:%M")


class DiffView(VerticalScroll):
    """Right pane 'Diff' tab: shows a Rich renderable (text diff / metadata table)."""

    def __init__(self, id: str | None = None) -> None:
        super().__init__(id=id)
        self._title: Static | None = None
        self._static: Static | None = None

    def compose(self) -> ComposeResult:
        self._title = Static("", id="diff-title")
        self._static = Static("[dim]no diff yet — press (d)iff or select a group[/dim]")
        yield self._title
        yield self._static

    def show_renderable(self, title: str, renderable) -> None:
        if self._title is not None:
            self._title.update(title)
        if self._static is not None:
            self._static.update(renderable)

    def show_lines(self, title: str, lines: list[Text]) -> None:
        renderable: object = Group(*lines) if lines else Text("[dim]no difference[/dim]")
        self.show_renderable(title, renderable)


class LogView(RichLog):
    """Right pane 'Log' tab: chronologically appended resolver output."""

    def add_action_line(self, text: str) -> None:
        self.write(text)

    def clear_log(self) -> None:
        self.clear()
