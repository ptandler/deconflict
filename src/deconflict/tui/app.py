"""Textual TUI for deconflict. Second frontend beside the Typer CLI.

The App owns the interaction loop over the UI-free engine: it presents the
current group's files/diff/log, exposes an action bar (mouse + keyboard), and
applies decisions via `Engine.apply()` / `Engine.launch_tool()` — mirroring the
CLI's `_run_interactive` but as a live, mouse-capable interface.
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING

from textual import on, work
from textual.app import App, ComposeResult
from textual.containers import Horizontal, HorizontalScroll, Vertical
from textual.widgets import Button, Footer, Header, TabbedContent, TabPane

from ..engine import Action
from ..launchers import ToolType
from ..render import metadata_diff_table, office_diff_lines, text_diff_lines
from .actions import (
    KEEP_BASE,
    KEEP_BOTH,
    KEEP_COPY,
    QUIT,
    SKIP,
    ActionEntry,
    build_actions,
)
from .views import DiffView, FilesTable, GroupTable, LogView

if TYPE_CHECKING:
    from ..analyze import GroupAnalysis
    from ..engine import Engine
    from ..scan import ConflictGroup


class DeconflictApp(App):
    """Two-pane live resolver: left = conflict groups, right = files/diff/log + actions."""

    TITLE = "deconflict"
    SUB_TITLE = ""
    CSS = """
    #left {
        width: 40%;
        border-right: solid $primary;
    }
    #right {
        width: 60%;
    }
    #tabs {
        height: 1fr;
    }
    #actions {
        height: 3;
        dock: bottom;
        background: $surface;
        padding: 0 1;
    }
    Button {
        margin: 0 1;
    }
    #files, #groups {
        height: 1fr;
    }
    """

    BINDINGS = [
        ("q", "quit_app", "Quit"),
        ("tab", "cycle_tab", "Files/Diff/Log"),
    ]

    def __init__(self, engine: "Engine", groups: list[tuple["ConflictGroup", "GroupAnalysis"]]):
        super().__init__()
        self.engine = engine
        self.groups = groups
        self.current_index = 0
        self.status: dict[str, str] = {}  # group.key -> 'resolved' | 'skipped'
        self.resolved = 0
        self.skipped = 0
        self.action_map: dict[str, ActionEntry] = {}

    # -- lifecycle ----------------------------------------------------------
    def on_mount(self) -> None:
        self.query_one("#files", FilesTable).prime()
        grouptable = self.query_one("#groups", GroupTable)
        for group, _ga in self.groups:
            grouptable.add_group(
                group.key,
                group.base.name if group.base else group.key,
                group.pattern,
                len(group.files),
            )
        if self.groups:
            self._select_index(0)

    def compose(self) -> ComposeResult:
        yield Header(show_clock=False)
        with Horizontal():
            with Vertical(id="left"):
                yield GroupTable(id="groups")
            with Vertical(id="right"):
                with TabbedContent(id="tabs"):
                    with TabPane("Files", id="tab-files"):
                        yield FilesTable(id="files")
                    with TabPane("Diff", id="tab-diff"):
                        yield DiffView(id="diff")
                    with TabPane("Log", id="tab-log"):
                        yield LogView(id="log")
                yield HorizontalScroll(id="actions")
        yield Footer()

    # -- selection ----------------------------------------------------------
    def _select_index(self, index: int) -> None:
        if not self.groups:
            return
        index = max(0, min(index, len(self.groups) - 1))
        self.current_index = index
        group, ga = self.groups[index]
        grouptable = self.query_one("#groups", GroupTable)
        row = grouptable.rows.get(group.key)
        if row is not None:
            self._set_cursor_to(group.key)
        self.query_one("#files", FilesTable).show(ga)
        self._show_default_diff(group, ga)
        self.log_view().write(f"[bold]— group {index + 1}/{len(self.groups)}: "
                              f"{group.base.name if group.base else group.key}[/bold]")
        self._render_actions(group, ga)
        self._update_subtitle()

    def _set_cursor_to(self, key: str) -> None:
        grouptable = self.query_one("#groups", GroupTable)
        rows = grouptable.rows
        ordered = list(rows.order)
        if key in ordered:
            idx = ordered.index(key)
            try:
                grouptable.move_cursor(row=idx)
            except Exception:
                pass

    def _show_default_diff(self, group: "ConflictGroup", ga: "GroupAnalysis") -> None:
        """Prime the Diff tab with the metadata table for the selected group."""
        diff = self.query_one("#diff", DiffView)
        if ga.base is not None and ga.copies:
            table = metadata_diff_table(ga, 0)
            diff.show_renderable(
                f"metadata diff — {group.base.name}", table or "[dim]no metadata[/dim]"
            )
        else:
            diff.show_renderable("metadata diff", "[dim]no base/copy to compare[/dim]")

    def on_data_table_row_selected(self, event) -> None:
        if event.data_table.id == "groups":
            key = event.row_key.value
            for i, (group, _ga) in enumerate(self.groups):
                if group.key == key:
                    self._select_index(i)
                    break

    # -- action bar ---------------------------------------------------------
    def _render_actions(self, group: "ConflictGroup", ga: "GroupAnalysis") -> None:
        box = self.query_one("#actions", HorizontalScroll)
        box.remove_children()
        self.action_map = {}
        entries = build_actions(self.engine, ga)
        for i, entry in enumerate(entries):
            btn = Button(f"({entry.hotkey}) {entry.label}", id=f"act-{i}")
            btn.variant = self._button_variant(entry.kind)
            self.action_map[f"act-{i}"] = entry
            box.mount(btn)

    @staticmethod
    def _button_variant(kind: str) -> str:
        if kind in (KEEP_BASE, KEEP_COPY):
            return "success"
        if kind == KEEP_BOTH:
            return "warning"
        if kind == SKIP:
            return "default"
        if kind == QUIT:
            return "error"
        if kind == "tool":
            return "primary"
        return "default"

    @on(Button.Pressed)
    def _on_button(self, event: Button.Pressed) -> None:
        entry = self.action_map.get(event.button.id)
        if entry is not None:
            self.dispatch(entry)
            if entry.kind not in ("tool", QUIT, "term_diff", "office_diff", "meta_view"):
                # a final decision -> re-render remaining actions for the next group
                self._render_actions_after_decision()

    # -- keyboard parity with the CLI menu ----------------------------------
    def on_key(self, event) -> None:
        char = getattr(event, "character", None)
        if not char:
            return
        key = char.lower()
        for entry in self.action_map.values():
            if entry.hotkey.lower() == key:
                event.stop()
                self.dispatch(entry)
                if entry.kind not in ("tool", QUIT, "term_diff", "office_diff", "meta_view"):
                    self._render_actions_after_decision()
                return

    def action_quit_app(self) -> None:
        msg = f"resolved {self.resolved} of {len(self.groups)} group(s)"
        if self.skipped:
            msg += f" ({self.skipped} skipped)"
        self.log_view().write(f"[bold][green]{msg}[/green][/bold]")
        self.exit(return_code=0, message=msg)

    def action_cycle_tab(self) -> None:
        tabs = self.query_one("#tabs", TabbedContent)
        order = ["tab-files", "tab-diff", "tab-log"]
        cur = tabs.active
        nxt = order[(order.index(cur) + 1) % len(order)] if cur in order else "tab-files"
        tabs.active = nxt

    # -- dispatch -----------------------------------------------------------
    def dispatch(self, entry: ActionEntry) -> None:
        group, ga = self.groups[self.current_index]
        if entry.kind == KEEP_BASE:
            self.apply_resolve(Action.keep_base(), group, ga)
        elif entry.kind == KEEP_COPY:
            idx = entry.target_index if entry.target_index is not None else 0
            target = ga.copies[idx].path if idx < len(ga.copies) else None
            if target is not None:
                self.apply_resolve(Action.keep_copy(target), group, ga)
        elif entry.kind == KEEP_BOTH:
            self.apply_resolve(Action.keep_both(), group, ga)
        elif entry.kind == SKIP:
            self._mark(group.key, "skipped")
        elif entry.kind == QUIT:
            self.action_quit_app()
        elif entry.kind in ("term_diff", "office_diff"):
            target = ga.copies[0].path if ga.copies else (ga.base.path if ga.base else None)
            self._render_diff(entry.kind, group, target)
        elif entry.kind == "meta_view":
            self._render_meta(ga)
        elif entry.kind == "tool" and entry.tool is not None:
            self._launch_tool(group, ga, entry.tool)

    def apply_resolve(self, action: Action, group: "ConflictGroup", ga: "GroupAnalysis") -> None:
        log = self.log_view()
        log.write(f"[bold yellow]applying {action.kind.value}…[/bold yellow]")
        resolved = self.engine.apply(group, ga, action)
        for r in resolved:
            dst = f" -> {r.moved_to}" if r.moved_to else ""
            detail = f" [{r.detail}]" if r.detail else ""
            log.write(f"  [green]{r.action}[/green]{dst}{detail}")
        self._mark(group.key, "resolved")

    def _mark(self, key: str, status: str) -> None:
        self.status[key] = status
        if status == "resolved":
            self.resolved += 1
        else:
            self.skipped += 1
        self.query_one("#groups", GroupTable).set_status(key, status)
        self._update_subtitle()
        self._advance()

    def _advance(self) -> None:
        for i, (group, _ga) in enumerate(self.groups):
            if group.key not in self.status:
                self._select_index(i)
                return
        # all groups decided
        self.log_view().write(
            f"[bold green]all {len(self.groups)} group(s) decided[/bold green]"
        )
        self._update_subtitle(force_done=True)

    def _render_actions_after_decision(self) -> None:
        # after a decision we've advanced; re-render actions for the new current group
        group, ga = self.groups[self.current_index]
        self._render_actions(group, ga)

    def _render_diff(self, kind: str, group: "ConflictGroup", target: Path | None) -> None:
        diff = self.query_one("#diff", DiffView)
        if target is None:
            diff.show_renderable(kind, "[yellow]nothing to diff[/yellow]")
            self._activate("tab-diff")
            return
        other = next((f for f in group.files if f.resolve() != target.resolve()), None)
        if other is None:
            diff.show_renderable(kind, "[yellow]nothing to diff against[/yellow]")
            self._activate("tab-diff")
            return
        if kind == "term_diff":
            try:
                lines = text_diff_lines(other, target)
            except OSError as exc:
                diff.show_renderable("text diff", f"[red]diff failed: {exc}[/red]")
                self._activate("tab-diff")
                return
            title = f"text diff — {other.name} vs {target.name}"
            diff.show_lines(title, lines or [])
        elif kind == "office_diff":
            lines = office_diff_lines(other, target)
            if lines:
                diff.show_lines("office diff (extracted text)", lines)
            else:
                diff.show_renderable("office diff (extracted text)", "[dim]no diff[/dim]")
        self._activate("tab-diff")

    def _render_meta(self, ga: "GroupAnalysis") -> None:
        diff = self.query_one("#diff", DiffView)
        if ga.base is not None and ga.copies:
            table = metadata_diff_table(ga, 0)
            diff.show_renderable("metadata diff", table or "[dim]no extractable metadata[/dim]")
        else:
            diff.show_renderable("metadata diff", "[yellow]nothing to compare[/yellow]")
        self._activate("tab-diff")

    @work(thread=True)
    def _launch_tool(self, group: "ConflictGroup", ga: "GroupAnalysis", tool: ToolType) -> None:
        target = ga.copies[0].path if ga.copies else (ga.base.path if ga.base else None)
        if target is None:
            self.call_from_thread(self._log, "[yellow]nothing to open[/yellow]")
            return
        self.call_from_thread(self._log, f"[bold]launching {tool.value} for {target.name}[/bold]")
        try:
            self.engine.launch_tool(group, ga, tool, target)
        except Exception as exc:  # noqa: BLE001
            self.call_from_thread(self._log, f"[red]tool error: {exc}[/red]")

    # -- helpers ------------------------------------------------------------
    def log_view(self) -> LogView:
        return self.query_one("#log", LogView)

    def _log(self, text: str) -> None:
        self.log_view().write(text)

    def _activate(self, tab_id: str) -> None:
        tabs = self.query_one("#tabs", TabbedContent)
        tabs.active = tab_id

    def _update_subtitle(self, force_done: bool = False) -> None:
        total = len(self.groups)
        if force_done:
            self.sub_title = f"done — {self.resolved} resolved · {self.skipped} skipped / {total}"
        else:
            self.sub_title = (
                f"{self.current_index + 1}/{total} | {self.resolved} resolved · "
                f"{self.skipped} skipped"
            )

