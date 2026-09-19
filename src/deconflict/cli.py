"""Typer CLI: thin subcommands + no-args interactive driver. UI only; logic lives in the engine."""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

import typer
from rich.console import Console
from rich.markup import escape
from rich.table import Table

from . import __version__, render
from .actions import actions_for
from .analyze import GroupAnalysis, consider, recommend
from .cache import clean_cache
from .config import Config, default_path, load, write_init_config
from .engine import Action, ActionKind, Engine
from .launchers import ToolType
from .media import KINDS
from .patterns import build_patterns
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
from .scan import ConflictGroup, ScanResult

app = typer.Typer(
    help="Resolve 'conflicted copy'-style sync conflict files. "
    "No args = interactive resolve (TUI if available, else CLI).",
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


def _version_cb(value: bool) -> None:
    if value:
        console.print(f"deconflict {__version__}")
        raise typer.Exit()


@app.command("version")
def version_command() -> None:
    """Show the deconflict version."""
    console.print(f"deconflict {__version__}")


def _path_cell(engine: Engine, path: Path) -> str:
    """Absolute path with the scan-start root highlighted (sync origin stays visible).

    `path` is resolved and rendered as `[bold cyan]<scan-root>/[/bold cyan]<rest>`
    so a quick glance shows which sync directory produced the conflict.
    """
    full = str(path.resolve())
    for root in engine.cfg.dirs:
        r = str(Path(root).expanduser().resolve())
        if full == r or full.startswith(r + os.sep):
            rest = full[len(r) + len(os.sep) :]
            return f"[bold cyan]{escape(r + os.sep)}[/bold cyan]{escape(rest)}"
    return escape(full)


def _scan_with_progress(
    engine: Engine, show_progress: bool, label: str | None = None
) -> ScanResult:
    """Engine.scan() wrapped in a Rich progress bar when requested (else plain scan).

    The filesystem walk has no known total up front, so the bar shows a live
    counter + the current file (instead of an indeterminate spinner) so the user
    sees forward movement.
    """
    if not show_progress:
        return engine.scan()
    from rich.progress import Progress

    count = {"n": 0}
    with Progress() as p:
        task = p.add_task(label or "scanning", total=1)
        result = engine.scan(
            on_file=lambda _f: (
                count.__setitem__("n", count["n"] + 1),
                p.update(task, description=f"scanning ({count['n']} files): {_f.name}"),
            )
        )
        p.update(task, completed=1, description=f"scanned {count['n']} files")
        return result


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


_ATTR_FIELDS = render._ATTR_FIELDS  # fs-attribute field names (shown, not "diff" markers)


@app.callback()
def _root(
    ctx: typer.Context,
    version: bool = typer.Option(
        False, "--version", callback=_version_cb, help="Show version and exit"
    ),
    config: Path | None = typer.Option(None, "--config", help="Alternate config file"),
) -> None:
    """Run interactive resolve when invoked with no subcommand.

    Uses TUI when the optional 'textual' dependency is installed; falls back
    to the CLI interactive loop otherwise.  Pass `resolve --tui` or
    `resolve --no-tui` to override.
    """
    ctx.obj = config
    if ctx.invoked_subcommand is None:
        _bare_resolve(config, [])


def _bare_resolve(config: Path | None, dirs: list[Path], show_progress: bool | None = None) -> None:
    """Bare `deconflict [dirs]`: scan + resolve, auto-selecting TUI when available."""
    engine, groups = _collect_groups(None, dirs, config, show_progress=show_progress)
    if _tui_available():
        _launch_tui(engine, groups)
    else:
        _run_interactive(engine, groups)
    _invalidate_cache(engine)


def _collect_groups(
    pattern: str | None,
    dirs: list[Path],
    config_path: Path | None = None,
    show_progress: bool | None = None,
):
    """Scan + analyze; `engine` is returned so callers can invalidate the cache."""
    engine = _require_dirs(_cfg(config_path, pattern, dirs))
    _report_scan(engine)
    t0 = time.perf_counter()
    result = _scan_with_progress(
        engine, sys.stdout.isatty() if show_progress is None else show_progress
    )
    t_scan = time.perf_counter()
    if not result:
        console.print("[green]no conflicts found[/green]")
        raise typer.Exit(0)
    groups = engine.analyze(result)
    t_analyze = time.perf_counter()
    _print_times(t0, t_scan, t_analyze)
    return engine, groups


def _print_times(t0: float, t_scan: float, t_analyze: float) -> None:
    """Item: stage timing so a slow startup's cost is attributable (scan vs analyze)."""
    console.print(
        f"[dim]  scan {t_scan - t0:.1f}s, analyze {t_analyze - t_scan:.1f}s "
        f"(total {t_analyze - t0:.1f}s)[/dim]"
    )


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


@app.command("setup")
def setup_cmd(
    ctx: typer.Context,
    config: Path | None = typer.Option(None, "--config", help="Alternate config file"),
) -> None:
    """Analyze the local setup and recommend installs (tools detection + hints)."""
    cfg = load(config or ctx.obj)
    engine = Engine(cfg.engine)
    console.print(f"[bold]deconflict {__version__}[/bold]")
    console.print(f"  config file: {escape(str(cfg.engine.config_path or default_path()))}")
    console.print(f"  backup dir:  {escape(str(cfg.engine.backup_dir))}")
    console.print(f"  cache dir:   {escape(str(cfg.engine.cache_dir))}")
    dirs = engine.cfg.dirs
    if dirs:
        console.print(f"  scan dirs:   {', '.join(escape(str(p)) for p in dirs)}")
    else:
        console.print(
            "  [yellow]scan dirs: none — run `deconflict init-config` and set "
            "[scan].default_dirs[/yellow]"
        )
    if _tui_available():
        console.print("  TUI: [green]available[/green] (textual installed)")
    else:
        console.print(
            "  TUI: [yellow]not installed[/yellow] — bare `deconflict` falls back to "
            "the CLI loop (install textual for the GUI)"
        )
    console.print()
    console.print("[bold]external tools:[/bold]")
    missing = 0
    for name, found, candidates, hint in engine.tool_status():
        if found:
            console.print(f"  [green]{name}[/green]: {escape(found)}")
        else:
            missing += 1
            suffix = f"  ({escape(hint)})" if hint else ""
            console.print(
                f"  [red]{name}[/red]: missing ({', '.join(escape(c) for c in candidates)}){suffix}"
            )
    console.print()
    if missing:
        console.print(
            f"[yellow]{missing} tool(s) missing.[/yellow] Install the recommended ones "
            "above (see each hint). The (?)tools action and `deconflict config tools` "
            "list what each file kind actually uses."
        )
    else:
        console.print("[green]all tools found.[/green]")
    console.print()
    console.print("[bold]next steps:[/bold]")
    console.print("  - `deconflict init-config`  -> write a documented config template")
    console.print("  - `deconflict config`       -> print the effective config")
    console.print("  - `deconflict config tools` -> which tool opens each file type")


@app.command()
def scan(
    ctx: typer.Context,
    dirs: list[Path] = typer.Argument(None, help="Directories (default: config)"),
    pattern: str | None = typer.Option(None, "--pattern", help="One pattern group"),
    config: Path | None = typer.Option(None, "--config", help="Alternate config file"),
    no_cache: bool = typer.Option(False, "--no-cache", help="Disable file cache"),
    refresh: bool = typer.Option(False, "--refresh", help="Force full rescan"),
    progress: bool | None = typer.Option(
        None,
        "--progress/--no-progress",
        help="Show scan progress (default: auto — on when interactive)",
    ),
) -> None:
    """Scan directories and list unresolved conflict groups. Exits after printing."""
    engine = _require_dirs(_cfg(ctx.obj or config, pattern, dirs))
    if (no_cache or refresh) and engine.cache is not None:
        engine.cache.clear()
        engine.cache_used = False
    _report_scan(engine)
    result = _scan_with_progress(engine, sys.stdout.isatty() if progress is None else progress)
    if not result:
        console.print("[green]no conflicts found[/green]")
        raise typer.Exit(0)
    table = Table("group", "pattern", "files")
    table.columns[0].overflow = "fold"
    for g in result.groups:
        name = _path_cell(engine, g.base) if g.base else g.key
        table.add_row(name, g.pattern, str(len(g.files)))
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
        None,
        "--auto",
        help="Non-interactive rule: base|copy|newest|bigger|recommended",
    ),
    dry_run: bool = typer.Option(
        False, "--dry-run", help="Preview without touching the filesystem"
    ),
    progress: bool | None = typer.Option(
        None,
        "--progress/--no-progress",
        help="Show scan progress (default: auto — on when interactive)",
    ),
    use_tui: bool | None = typer.Option(
        None, "--tui/--no-tui", help="Use TUI or CLI (default: CLI; bare mode: TUI if available)"
    ),
) -> None:
    """Resolve conflicts. Losers move to backup (never hard-deleted). --auto = non-interactive."""
    if auto and use_tui is True:
        console.print("[red]--auto and --tui are mutually exclusive.[/red]")
        raise typer.Exit(2)
    cfg = _cfg(ctx.obj or config, pattern, dirs)
    if dry_run:
        cfg.engine.dry_run = True
    engine = _require_dirs(cfg)
    _report_scan(engine)
    t0 = time.perf_counter()
    result = _scan_with_progress(engine, sys.stdout.isatty() if progress is None else progress)
    t_scan = time.perf_counter()
    if not result:
        console.print("[green]no conflicts to resolve[/green]")
        raise typer.Exit(0)
    groups = engine.analyze(result)
    t_analyze = time.perf_counter()
    _print_times(t0, t_scan, t_analyze)
    if auto:
        _run_auto(engine, groups, auto)
    elif use_tui is True:
        _launch_tui(engine, groups)
    else:
        _run_interactive(engine, groups)
    _invalidate_cache(engine)


def _tui_available() -> bool:
    """Check whether the optional Textual TUI extra is installed."""
    try:
        import textual  # noqa: F401

        return True
    except ImportError:
        return False


def _launch_tui(engine: Engine, groups) -> None:
    """Launch the Textual TUI; raises typer.Exit(1) if textual is missing."""
    if not _tui_available():
        console.print(
            "[red]TUI requires the optional dependency `textual`.[/red] "
            "Install it with: `uv sync --extra tui` or `pip install deconflict[tui]`."
        )
        raise typer.Exit(1)
    from .tui.app import DeconflictApp

    tui_app = DeconflictApp(engine, groups)
    tui_app.run()


def _run_auto(engine: Engine, groups, rule: str) -> None:
    if rule == "recommended":
        _run_auto_recommended(engine, groups)
        return
    for group, ga in groups:
        _apply(engine, group, engine.auto_choice(ga, rule))


def _run_auto_recommended(engine: Engine, groups) -> None:
    """Resolve only groups with a clear recommendation; skip the rest.

    Prints stats and, when anything was skipped, hints to rerun interactively.
    The engine invalidates the cache after real applies so no extra scan is needed.
    """
    resolved = 0
    skipped = 0
    for group, ga in groups:
        rec = _recommend(ga)
        if rec is None:
            skipped += 1
            continue
        _apply(engine, group, rec[1])
        resolved += 1
    msg = f"[done]resolved {resolved} group(s)"
    if skipped:
        msg += f" ({skipped} skipped)"
    console.print(msg)
    if skipped:
        console.print(
            "[yellow]hint: re-run `deconflict resolve` (without --auto) to handle "
            "the skipped group(s) interactively.[/yellow]"
        )


def _run_interactive(engine: Engine, groups) -> None:
    total = len(groups)
    resolved = 0
    skipped = 0
    for i, (group, ga) in enumerate(groups, 1):
        while True:
            rec = _show_group(engine, group, ga, i, total)
            action = _prompt(engine, group, ga, rec)
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


def _recommend(ga: GroupAnalysis) -> tuple[str, Action] | None:
    """A clear winner (label, keep-Action) or None.

    Thin wrapper over the UI-free `analyze.recommend` that maps the winner to an
    Action (keep base vs keep copy) for the CLI prompt.
    """
    rec = recommend(ga)
    if rec is None:
        return None
    action = Action.keep_base() if rec.is_base else Action.keep_copy(rec.winner.path)
    return rec.label, action


def _show_group(
    engine: Engine, group: ConflictGroup, ga: GroupAnalysis, i: int, total: int
) -> tuple[str, Action] | None:
    """Primary per-group view: full paths + metadata-diff table(s) + recommendation.

    Returns the recommended `(label, Action)` when a clear winner exists (used as
    the Enter default in `_prompt`), else None.
    """
    console.print()
    title = group.base.name if group.base else group.key
    console.print(f"[bold]Group {i}/{total}[/bold] — {title}")
    if ga.all_equal:
        console.print("[dim]all files identical in content[/dim]")
    else:
        console.print("[yellow]files differ in content[/yellow]")
    console.print("[dim]full paths:[/dim]")
    for a in ([ga.base] if ga.base else []) + ga.copies:
        console.print(f"  {_path_cell(engine, a.path)}")
    if ga.base is not None:
        for idx, _c in enumerate(ga.copies):
            tb = metadata_diff_table(ga, idx)
            if tb is not None:
                console.print(tb)
    # Reasoning: show what was evaluated (always), so the user understands WHY a
    # recommendation is or isn't made (esp. the "no clear winner" cases).
    cons = consider(ga)
    if cons.notes:
        console.print("[dim]considered:[/dim]")
        for n_ in cons.notes:
            console.print(f"  [dim]· {escape(n_)}[/dim]")
    # Recommendation is surfaced ONLY via the (Enter) menu line (see _menu_text);
    # printing it here too would duplicate it.
    return _recommend(ga)


def _prompt(
    engine: Engine, group: ConflictGroup, ga: GroupAnalysis, rec: tuple[str, Action] | None
) -> Action | None:
    target = ga.copies[0].path if ga.copies else (ga.base.path if ga.base else None)
    bykey: dict[str, tuple[str, ToolType | str]] = {
        key: (label, tool) for key, label, tool in actions_for(engine, ga)
    }
    while True:
        # Render the menu through Rich first so `[bold cyan]…[/bold cyan]` markup
        # becomes real ANSI color codes; input() would otherwise print them raw.
        console.print(_menu_text(engine, ga, rec), end="")
        ans = input().strip().lower()
        if ans == "":
            if rec is not None:
                return rec[1]
            print("invalid choice: there is no recommended version to keep")
            continue
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


def _menu_text(engine: Engine, ga: GroupAnalysis, rec: tuple[str, Action] | None = None) -> str:
    parts: list[str] = []
    if rec is not None:
        parts.append(f"[bold cyan](Enter) keep {escape(rec[0])}[/bold cyan]")
    parts.append("(0) keep base") if ga.base else None
    for n, _c in enumerate(ga.copies, 1):
        parts.append(f"({n}) keep copy #{n}")
    parts += [f"({key}){label}" for key, label, _tool in actions_for(engine, ga)]
    parts += ["(h)keep-both", "(s)kip", "(q)uit", "(?)tools"]
    return " ".join(parts) + ": "


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


FieldDicts = tuple[dict[str, str], dict[str, str]]


def _diff_fields(ga: GroupAnalysis, idx: int) -> FieldDicts:
    """Metadata field dicts for (base, copy) at `idx`, read from analysis (no re-extract)."""
    return render._diff_fields(ga, idx)


_META_HEADER_TAIL = render._META_HEADER_TAIL  # trailer chars kept on header truncation


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


_TOP_LEVEL_COMMANDS = frozenset(
    {"patterns", "init-config", "config", "scan", "cache", "resolve", "version", "setup"}
)


def main() -> None:
    argv = sys.argv[1:]
    # Bare `deconflict /path/...`: a positional directory appears where a
    # subcommand name would go. Typer would misread the path as a command
    # name, so detect it and dispatch straight to the bare interactive driver
    # (auto-TUI + resolve). Known subcommands / leading options are untouched.
    first_dirs_idx = None
    for i, a in enumerate(argv):
        if a.startswith("-"):
            continue
        if a in _TOP_LEVEL_COMMANDS:
            first_dirs_idx = None
            break
        first_dirs_idx = i
        break
    if first_dirs_idx is not None:
        config: Path | None = None
        dirs: list[Path] = []
        i = 0
        while i < len(argv):
            a = argv[i]
            if a == "--config":
                if i + 1 >= len(argv):
                    raise typer.BadParameter("--config requires a value")
                config = Path(argv[i + 1])
                i += 2
                continue
            if a.startswith("-"):
                raise typer.BadParameter(f"unknown option for bare invocation: {a}")
            dirs.append(Path(a))
            i += 1
        _bare_resolve(config, dirs)
        return
    app()


if __name__ == "__main__":
    main()
