"""Typer CLI: thin subcommands + no-args interactive driver. UI only; logic lives in the engine."""

from __future__ import annotations

from pathlib import Path

import typer
from rich.console import Console
from rich.markup import escape
from rich.table import Table

from . import render
from .actions import actions_for
from .analyze import GroupAnalysis
from .cache import clean_cache
from .config import Config, default_path, load, write_init_config
from .engine import Action, ActionKind, Engine
from .launchers import ToolType
from .media import KINDS
from .patterns import build_patterns
from .render import (
    fmt_mtime as _fmt_mtime,
)
from .render import (
    human as _human,
)
from .render import (
    metadata_diff_table,
    office_diff_lines,
    text_diff_lines,
)
from .render import (
    truncate_meta as _truncate_meta,  # noqa: F401  (re-export for tests)
)
from .render import (
    truncate_name as _truncate_name,  # noqa: F401  (re-export for tests)
)
from .scan import ConflictGroup

app = typer.Typer(
    help="Resolve 'conflicted copy'-style sync conflict files. No args = interactive resolve.",
    add_completion=False,
    no_args_is_help=False,
    invoke_without_command=True,
)
console = Console()

_PATTERN_DESCR = {
    "nextcloud": "name (conflicted copy YYYY-MM-DD HHMMSS)",
    "pacman": "*.pacnew/.pacsave/.pacorig",
    "syncthing": "*.sync-conflict-*",
    "dpkg": "*.dpkg-dist/.dpkg-old",
    "rpm": "*.rpmnew/.rpmorig",
    "windows-copy": "* - Copy, * (copy)",
}


def _cfg(config_path: Path | None, pattern: str | None, dirs: list[Path]) -> Config:
    cfg = load(config_path)
    if pattern:
        cfg.engine.enabled_patterns = [pattern]
    cfg.engine.dirs = dirs or cfg.engine.dirs
    return cfg


def _require_dirs(cfg: Config) -> Engine:
    if not cfg.engine.dirs:
        console.print(
            "[red]no directories.[/red] Pass dirs or set [scan].default_dirs in config "
            "(see `deconflict config`); run `deconflict init-config` to create one."
        )
        raise typer.Exit(2)
    return Engine(cfg.engine)


def _invalidate_cache(engine: Engine) -> None:
    """After a real resolve, drop the now-stale scan cache for the next run."""
    if not engine.cfg.dry_run and engine.cache is not None:
        engine.cache.clear()


def _size_cell(ga: GroupAnalysis, a, idx) -> str:
    """Size, with the copy's delta vs base shown in parentheses (base has none)."""
    size = f"{_human(a.info.size)}"
    if idx is None:
        return size
    delta = a.info.size - ga.base.info.size if ga.base else 0
    if ga.base and a.info.sha == ga.base.info.sha:
        return f"{size} [dim](=)[/dim]"
    if ga.base is None:
        arrow = "↑" if delta > 0 else "↓" if delta < 0 else "="
        return f"{size} [dim]({arrow})[/dim]"
    arrow = "↑" if delta > 0 else "↓" if delta < 0 else "≠"
    bit = f" {_human(abs(delta))}" if delta else ""
    return f"{size} [yellow]({arrow}{bit})[/yellow]"


_ATTR_FIELDS = render._ATTR_FIELDS  # fs-attribute field names (shown, not "diff" markers)


@app.callback()
def _root(
    ctx: typer.Context,
    config: Path | None = typer.Option(None, "--config", help="Alternate config file"),
) -> None:
    """Run interactive resolve when invoked with no subcommand."""
    ctx.obj = config
    if ctx.invoked_subcommand is None:
        engine, groups = _collect_groups(None, [], config)
        _run_interactive(engine, groups)
        _invalidate_cache(engine)


def _collect_groups(pattern: str | None, dirs: list[Path], config_path: Path | None = None):
    engine = _require_dirs(_cfg(config_path, pattern, dirs))
    _report_scan(engine)
    result = engine.scan()
    if not result:
        console.print("[green]no conflicts found[/green]")
        raise typer.Exit(0)
    return engine, engine.analyze(result)


def _report_scan(engine: Engine) -> None:
    """Item: log to console whether we're rescanning or reusing the cache."""
    if engine.cache is not None and engine.cache_used:
        console.print(f"[dim]using cached scan ({engine.cache.path.name})…[/dim]")
    else:
        dirs = ", ".join(str(d) for d in engine.cfg.dirs)
        console.print(f"[dim]scanning {dirs}…[/dim]")


@app.command()
def patterns(
    enabled_only: bool = typer.Option(False, "--enabled", help="Only enabled patterns"),
) -> None:
    """List recognized conflict patterns."""
    all_patterns = build_patterns()
    table = Table("name", "enabled", "winner", "matches")
    for p in all_patterns:
        if enabled_only and not p.enabled:
            continue
        table.add_row(p.name, str(p.enabled), p.winner, _PATTERN_DESCR.get(p.name, ""))
    console.print(table)


@app.command()
def init_config(path: Path | None = typer.Option(None, "--path", help="Write config here")) -> None:
    """Write a documented default config (refuses to overwrite)."""
    try:
        written = write_init_config(path)
    except FileExistsError as exc:
        console.print(f"[yellow]exists:[/yellow] {exc}")
        raise typer.Exit(1) from exc
    console.print(f"wrote config: {written}")


config_app = typer.Typer(
    help="Configuration: print the effective config (default) or per-filetype tools.",
    invoke_without_command=True,
    no_args_is_help=False,
)
app.add_typer(config_app, name="config")


@config_app.callback()
def _config(
    ctx: typer.Context,
    path: Path | None = typer.Option(
        None, "--path", help="Config file to read (default: user config)"
    ),
) -> None:
    """Print the effective config (defaults merged with the loaded file)."""
    cfg = load(path or ctx.obj)
    ctx.obj = cfg
    if ctx.invoked_subcommand is None:
        _print_config(cfg)


def _print_config(cfg: Config) -> None:
    if cfg.source is None:
        console.print(
            "[bold]config file:[/bold] none — built-in defaults (run `deconflict init-config`)"
        )
    else:
        console.print(f"[bold]config file:[/bold] {cfg.source}")
    console.print(escape("[scan]"))
    console.print(escape(f"  default_dirs = [{', '.join(str(p) for p in cfg.engine.dirs)}]"))
    console.print(escape(f"  enabled_patterns = {cfg.engine.enabled_patterns}"))
    for name, base, winner in cfg.engine.custom_patterns:
        console.print(
            escape(f"  custom pattern: name={name!r} base_name={base!r} winner={winner!r}")
        )
    console.print(escape("[general]"))
    console.print(escape(f"  backup_dir = {str(cfg.engine.backup_dir)!r}"))
    console.print(escape(f"  cache_dir = {str(cfg.engine.cache_dir)!r}"))
    if cfg.tools:
        console.print(escape("[tools]"))
        for name, path_ in cfg.tools.items():
            console.print(escape(f"  {name} = {path_!r}"))
    if cfg.engine.file_type_tools:
        console.print(escape("[file_types]"))
        for kind, tool in cfg.engine.file_type_tools.items():
            console.print(escape(f"  {kind} = {tool!r}"))


@config_app.command("tools")
def config_tools(
    ctx: typer.Context,
    path: Path | None = typer.Option(
        None, "--path", help="Config file to read (default: user config)"
    ),
) -> None:
    """Show which tool backs each file type (view/edit/meta) — the (?)tools view
    for every kind, with found/missing status and install hints."""
    cfg = load(path) if path else ctx.obj
    engine = Engine(cfg.engine)
    for kind in KINDS:
        _print_kind_tools(engine, kind)
    cfg_path = cfg.engine.config_path or default_path()
    _print_tools_footer(cfg_path)


@app.command()
def scan(
    ctx: typer.Context,
    dirs: list[Path] = typer.Argument(None, help="Directories (default: config)"),
    pattern: str | None = typer.Option(None, "--pattern", help="One pattern group"),
    config: Path | None = typer.Option(None, "--config", help="Alternate config file"),
    no_cache: bool = typer.Option(False, "--no-cache", help="Disable file cache"),
    refresh: bool = typer.Option(False, "--refresh", help="Force full rescan"),
) -> None:
    """Scan directories and list unresolved conflict groups. Exits after printing."""
    engine = _require_dirs(_cfg(ctx.obj or config, pattern, dirs))
    if (no_cache or refresh) and engine.cache is not None:
        engine.cache.clear()
        engine.cache_used = False
    _report_scan(engine)
    result = engine.scan()
    if not result:
        console.print("[green]no conflicts found[/green]")
        raise typer.Exit(0)
    table = Table("group", "pattern", "files")
    for g in result.groups:
        table.add_row(g.base.name if g.base else g.key, g.pattern, str(len(g.files)))
    console.print(table)
    console.print(
        f"[cyan]{len(result.groups)}[/cyan] conflict group(s). Run `deconflict resolve` to act."
    )


cache_app = typer.Typer(
    help="Manage the file cache (stale-entry cleanup).",
    invoke_without_command=True,
    no_args_is_help=True,
)
app.add_typer(cache_app, name="cache")


@cache_app.command("clean")
def cache_clean(
    ctx: typer.Context,
    config: Path | None = typer.Option(None, "--config", help="Alternate config file"),
    dry_run: bool = typer.Option(False, "--dry-run", help="Report removals without deleting"),
    all_: bool = typer.Option(False, "--all", help="Remove every cache file"),
    older_than_days: float | None = typer.Option(
        None, "--older-than-days", help="Remove files older than N days"
    ),
) -> None:
    """Remove stale cache files (dirs no longer on disk, or age/size-based)."""
    cfg = load(ctx.obj or config)
    cache_dir = cfg.engine.cache_dir
    removed, freed = clean_cache(
        cache_dir,
        dry_run=dry_run,
        all_=all_,
        older_than=older_than_days * 86400 if older_than_days is not None else None,
    )
    prefix = "[yellow]would remove[/yellow]" if dry_run else "[red]removed[/red]"
    if not removed:
        console.print("[green]cache is clean[/green]")
        return
    for f, reason in removed:
        console.print(f"  {prefix} {f.name} ({reason})")
    console.print(f"[cyan]{len(removed)}[/cyan] file(s), {_human(freed)} freed")


@app.command()
def resolve(
    ctx: typer.Context,
    dirs: list[Path] = typer.Argument(None, help="Directories (default: config)"),
    pattern: str | None = typer.Option(None, "--pattern", help="One pattern group"),
    config: Path | None = typer.Option(None, "--config", help="Alternate config file"),
    auto: str | None = typer.Option(
        None, "--auto", help="Non-interactive rule: base|copy|newest|bigger"
    ),
    dry_run: bool = typer.Option(
        False, "--dry-run", help="Preview without touching the filesystem"
    ),
) -> None:
    """Resolve conflicts. Losers move to backup (never hard-deleted). --auto = non-interactive."""
    cfg = _cfg(ctx.obj or config, pattern, dirs)
    if dry_run:
        cfg.engine.dry_run = True
    engine = _require_dirs(cfg)
    _report_scan(engine)
    result = engine.scan()
    if not result:
        console.print("[green]no conflicts to resolve[/green]")
        raise typer.Exit(0)
    groups = engine.analyze(result)
    if auto:
        _run_auto(engine, groups, auto)
    else:
        _run_interactive(engine, groups)
    _invalidate_cache(engine)


@app.command()
def tui(
    dirs: list[Path] = typer.Argument(None, help="Directories (default: config)"),
    pattern: str | None = typer.Option(None, "--pattern", help="One pattern group"),
    config: Path | None = typer.Option(None, "--config", help="Alternate config file"),
    dry_run: bool = typer.Option(
        False, "--dry-run", help="Preview without touching the filesystem"
    ),
) -> None:
    """Launch the interactive Textual TUI (needs the optional 'tui' extra)."""
    try:
        from .tui.app import DeconflictApp
    except ImportError as exc:  # textual not installed
        console.print(
            "[red]TUI requires the optional dependency `textual`.[/red] "
            "Install it with: `uv sync --extra tui` or `pip install deconflict[tui]`."
        )
        raise typer.Exit(1) from exc
    cfg = _cfg(ctx.obj or config, pattern, dirs)
    if dry_run:
        cfg.engine.dry_run = True
    engine = _require_dirs(cfg)
    _report_scan(engine)
    result = engine.scan()
    if not result:
        console.print("[green]no conflicts to resolve[/green]")
        raise typer.Exit(0)
    groups = engine.analyze(result)
    app = DeconflictApp(engine, groups)
    app.run()
    _invalidate_cache(engine)


def _run_auto(engine: Engine, groups, rule: str) -> None:
    for group, ga in groups:
        _apply(engine, group, engine.auto_choice(ga, rule))


def _run_interactive(engine: Engine, groups) -> None:
    total = len(groups)
    resolved = 0
    skipped = 0
    for i, (group, ga) in enumerate(groups, 1):
        while True:
            _show_group(engine, group, ga, i, total)
            action = _prompt(engine, group, ga)
            if action is None:  # (q)uit
                console.print(f"\n[done]aborted after {i - 1} of {total} group(s)")
                return
            if action.kind is ActionKind.TOOL:
                if action.tool is ToolType.VIEW and not engine.launcher().view_commands(
                    group, action.target
                ):
                    console.print(
                        "[yellow]no viewer/editor found for this type — see (?)tools[/yellow]"
                    )
                    continue
                engine.launch_tool(group, ga, action.tool or ToolType.VIEW, action.target)
                if action.tool is ToolType.VIEW:
                    console.print("[dim]press [bold]Enter[/bold] when done reviewing[/dim]")
                    input()
                continue
            if action.kind is ActionKind.SKIP:
                skipped += 1
            else:
                resolved += 1
            _apply(engine, group, action)
            break
    msg = f"\n[done]resolved {resolved} group(s)"
    if skipped:
        msg += f" ({skipped} skipped)"
    console.print(msg)


def _apply(engine: Engine, group: ConflictGroup, action: Action) -> None:
    from .analyze import analyze_group

    ga = (
        analyze_group(group, group.base, group.conflicts)
        if action.kind in (ActionKind.KEEP_COPY, ActionKind.KEEP_BASE)
        else None
    )
    resolved = engine.apply(group, ga, action)
    for r in resolved:
        label = f"[bold]{r.action}[/bold]"
        dst = f" -> {r.moved_to}" if r.moved_to else ""
        detail = f" [{r.detail}]" if r.detail else ""
        console.print(f"  {label}{dst}{detail}")


def _show_group(
    engine: Engine, group: ConflictGroup, ga: GroupAnalysis, i: int, total: int
) -> None:
    console.print()
    title = group.base.name if group.base else group.key
    table = Table(title=f"[bold]Group {i}/{total}[/bold] — {title}")
    table.add_column("#")
    table.add_column("file", style="cyan")
    table.add_column("size")
    table.add_column("mtime")
    table.add_column("metadata")
    rows: list[tuple] = ([(ga.base, None)] if ga.base else []) + [
        (c, i) for i, c in enumerate(ga.copies)
    ]
    for a, idx in rows:
        if a is None:
            continue
        if idx is None:  # base row
            num = "[bold]#0[/bold]"
            meta = "[dim]—[/dim]"
        else:
            num = f"[bold]#{idx + 1}[/bold]"
            md = ga.meta[idx] if idx < len(ga.meta) else None
            if md is None:
                meta = "[dim]—[/dim]"
            elif md is True:
                meta = "[green]same[/green]"
            else:
                differ = _differing_fields(ga, idx)
                meta = "[yellow]diff[/yellow]"
                if differ:
                    meta += "\n" + "\n".join(f"[dim]{d}[/dim]" for d in differ)
        table.add_row(
            num,
            a.path.name,
            _size_cell(ga, a, idx),
            _fmt_mtime(a.info.mtime),
            meta,
        )
    console.print(table)
    if ga.all_equal:
        console.print("[dim]copies identical in content[/dim]")
    else:
        console.print("[yellow]copies differ in content[/yellow]")
        for idx, c in enumerate(ga.copies):
            if idx >= len(ga.meta) or ga.meta[idx] is None:
                continue
            if ga.meta[idx] is True:
                console.print(f"[dim]  {c.path.name}: content differs, metadata same[/dim]")
            else:
                detail = ga.meta_diff[idx] or "metadata differs"
                console.print(f"[yellow]  {c.path.name}: {detail}[/yellow]")


def _prompt(engine: Engine, group: ConflictGroup, ga: GroupAnalysis) -> Action | None:
    target = ga.copies[0].path if ga.copies else (ga.base.path if ga.base else None)
    bykey: dict[str, tuple[str, ToolType | str]] = {
        key: (label, tool) for key, label, tool in actions_for(engine, ga)
    }
    while True:
        ans = input(_menu_text(engine, ga)).strip().lower()
        if ans.isdigit():
            action = _keep_by_number(ga, int(ans))
            if action is not None:
                return action
            print("invalid choice")
            continue
        if ans in bykey:
            label, tool = bykey[ans]
            if isinstance(tool, ToolType):
                return Action.tool_action(tool, target) if target else None
            if tool == "term_diff":
                _print_text_diff(group, target)
                continue
            if tool == "office_diff":
                _print_office_diff(group, target)
                continue
            if tool == "meta_view":
                _print_metadata_diff(ga, 0)
                continue
        if ans in ("h", "keep-both"):
            return Action.keep_both()
        if ans in ("s", "skip"):
            return Action.skip()
        if ans in ("q", "quit", "exit"):
            return None
        if ans in ("?", "help", "tools"):
            _print_tools(engine, ga)
            continue
        print("invalid choice")


def _keep_by_number(ga: GroupAnalysis, idx: int) -> Action | None:
    """Map a menu number to a keep action: #0 = base, #1..N = copies. None when invalid."""
    if idx == 0 and ga.base is not None:
        return Action.keep_base()
    if 1 <= idx <= len(ga.copies):
        return Action.keep_copy(ga.copies[idx - 1].path)
    return None


def _menu_text(engine: Engine, ga: GroupAnalysis) -> str:
    parts = ["(0) keep base"] if ga.base else []
    for n, _c in enumerate(ga.copies, 1):
        parts.append(f"({n}) keep copy #{n}")
    parts += [f"({key}){label}" for key, label, _tool in actions_for(engine, ga)]
    parts += ["(h)keep-both", "(s)kip", "(q)uit", "(?)tools"]
    return " ".join(parts) + ": "


def _hl_runs(old: str, new: str):
    """Char-level diff highlight (shared with the TUI via render._hl_runs).

    Returns (old, new) Text objects with changed runs highlighted.
    """
    return render._hl_runs(old, new)


def _print_text_diff(group: ConflictGroup, target: Path | None) -> None:
    """Print a char-level unified diff between two files of a group (text kinds).

    Delegates rendering to the shared `render.text_diff_lines` so the TUI shows
    the identical diff.
    """
    if target is None:
        console.print("[yellow]nothing to diff[/yellow]")
        return
    other = next((f for f in group.files if f.resolve() != target.resolve()), None)
    if other is None:
        console.print("[yellow]nothing to diff against[/yellow]")
        return
    try:
        lines = text_diff_lines(other, target)
    except OSError as exc:
        console.print(f"[red]diff failed: {exc}[/red]")
        return
    if lines is None:
        console.print("[dim]no textual difference[/dim]")
        return
    for line in lines:
        console.print(line)


def _print_office_diff(group: ConflictGroup, target: Path | None) -> None:
    """Print a diff of two office docs by their extracted text (headless)."""
    if target is None:
        console.print("[yellow]nothing to diff[/yellow]")
        return
    other = next((f for f in group.files if f.resolve() != target.resolve()), None)
    if other is None:
        console.print("[yellow]nothing to diff against[/yellow]")
        return
    lines = office_diff_lines(other, target)
    if lines is None:
        console.print("[dim]no textual difference[/dim]")
        return
    for line in lines:
        console.print(line)


def _meta_differs(ga: GroupAnalysis) -> bool:
    """True when any copy's metadata differs from the base (media kinds)."""
    return ga.base is not None and any(m is False for m in ga.meta)


FieldDicts = tuple[dict[str, str], dict[str, str]]


def _diff_fields(ga: GroupAnalysis, idx: int) -> FieldDicts:
    """Metadata field dicts for (base, copy) at `idx`, read from analysis (no re-extract)."""
    return render._diff_fields(ga, idx)


def _differing_fields(ga: GroupAnalysis, idx: int) -> list[str]:
    """Short `field: value` lines for the copy's CONTENT fields that differ from base."""
    bf, cf = _diff_fields(ga, idx)
    return [
        f"{k}: {_truncate_meta(cf.get(k, '∅'))}"
        for k in dict.fromkeys([*bf, *cf])
        if bf.get(k) != cf.get(k) and k not in _ATTR_FIELDS
    ]


_META_HEADER_TAIL = render._META_HEADER_TAIL  # trailer chars kept on header truncation


def _meta_diff_columns(ga: GroupAnalysis, idx: int, limit: int = 40) -> tuple[str, str]:
    """Short header labels for the metadata table: `#N <truncated-filename>`."""
    return render._meta_diff_columns(ga, idx, limit)


def _print_metadata_diff(ga: GroupAnalysis, idx: int) -> None:
    """Print the ONE combined metadata-diff table (base vs copy columns).

    Delegates to the shared `render.metadata_diff_table` so the TUI and CLI
    render the identical table.
    """
    table = metadata_diff_table(ga, idx)
    if table is None:
        console.print("[yellow]nothing to compare[/yellow]")
        return
    console.print(table)
    bf, cf = _diff_fields(ga, idx)
    content = [k for k in dict.fromkeys([*bf, *cf]) if k not in _ATTR_FIELDS]
    if any(bf.get(k) != cf.get(k) for k in content):
        console.print("[dim]differing fields for copy vs base (shared rows dimmed)[/dim]")


def _row_cells(bf, cf, keys: list[str]) -> list[tuple[str, str, str]]:
    """(field, base-cell, copy-cell) rows, truncated and escaped."""
    return render._row_cells(bf, cf, keys)


def _size_meta_cell(size: int, other: int | None) -> str:
    """Size cell: human-readable + bytes, with diff vs the other file."""
    return render.size_meta_cell(size, other)


def _print_kind_tools(engine: Engine, kind: str) -> None:
    """One file kind's tool mapping + found/missing status + hints.

    Shared by the interactive (?)tools action and `deconflict config tools`.
    """
    la = engine.launcher()
    console.print(f"[bold]file kind:[/bold] {kind}")
    console.print(f"  view: {la.kind_tool(kind)} · edit: {la.edit_tool(kind)}")
    for key in la.tools_for(kind):
        found = engine.tools.get(key)
        if found:
            console.print(f"  [green]{key}[/green]: {found}")
        else:
            hint = engine.tools.hint(key)
            suffix = f"  ({hint})" if hint else ""
            console.print(
                f"  [red]{key}[/red]: missing ({', '.join(engine.tools.candidates(key))}){suffix}"
            )


def _print_tools_footer(cfg_path: Path) -> None:
    """Dim hint: where to configure the tools (escaped — section names are markup)."""
    footer = (
        f"configure tools in ({cfg_path}) — sections [tools], [file_types], "
        "[file_types_edit]; run `deconflict init-config` to write a template."
    )
    console.print(f"[dim]{escape(footer)}[/dim]")


def _print_tools(engine: Engine, ga: GroupAnalysis) -> None:
    cfg_path = engine.cfg.config_path or default_path()
    _print_kind_tools(engine, ga.kind)
    _print_tools_footer(cfg_path)


def main() -> None:
    app()


if __name__ == "__main__":
    main()
