"""Typer CLI: thin subcommands + no-args interactive driver. UI only; logic lives in the engine."""

from __future__ import annotations

import datetime
from pathlib import Path

import typer
from rich.console import Console
from rich.markup import escape
from rich.table import Table

from .analyze import GroupAnalysis
from .cache import clean_cache
from .config import Config, load, write_init_config
from .engine import Action, ActionKind, Engine
from .launchers import ToolType
from .patterns import build_patterns
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
            "(see `deconflict config`)."
        )
        raise typer.Exit(2)
    return Engine(cfg.engine)


def _invalidate_cache(engine: Engine) -> None:
    """After a real resolve, drop the now-stale scan cache for the next run."""
    if not engine.cfg.dry_run and engine.cache is not None:
        engine.cache.clear()


def _human(n: int) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024
    return str(n)


def _fmt_mtime(t: float) -> str:
    return datetime.datetime.fromtimestamp(t).strftime("%Y-%m-%d %H:%M")


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
    result = engine.scan()
    if not result:
        console.print("[green]no conflicts found[/green]")
        raise typer.Exit(0)
    return engine, engine.analyze(result)


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


@app.command()
def config(
    path: Path | None = typer.Option(
        None, "--path", help="Config file to read (default: user config)"
    ),
) -> None:
    """Print the effective config (defaults merged with the loaded file)."""
    cfg = load(path)
    if cfg.source is None:
        console.print(
            "[bold]config file:[/bold] none — built-in defaults (run `deconflict init-config`)"
        )
    else:
        console.print(f"[bold]config file:[/bold] {cfg.source}")
    console.print(escape("[scan]"))
    console.print(f"  default_dirs = [{', '.join(str(p) for p in cfg.engine.dirs)}]")
    console.print(f"  enabled_patterns = {cfg.engine.enabled_patterns}")
    for name, base, winner in cfg.engine.custom_patterns:
        console.print(f"  custom pattern: name={name!r} base_name={base!r} winner={winner!r}")
    console.print(escape("[general]"))
    console.print(f"  backup_dir = {str(cfg.engine.backup_dir)!r}")
    console.print(f"  cache_dir = {str(cfg.engine.cache_dir)!r}")
    if cfg.tools:
        console.print(escape("[tools]"))
        for name, path_ in cfg.tools.items():
            console.print(f"  {name} = {path_!r}")


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


def _run_auto(engine: Engine, groups, rule: str) -> None:
    for group, ga in groups:
        _apply(engine, group, engine.auto_choice(ga, rule))


def _run_interactive(engine: Engine, groups) -> None:
    total = len(groups)
    for i, (group, ga) in enumerate(groups, 1):
        while True:
            _show_group(engine, group, ga, i, total)
            action = _prompt(group, ga)
            if action.kind is ActionKind.TOOL:
                engine.launch_tool(group, ga, action.tool or ToolType.EDITOR, action.target)
                continue
            _apply(engine, group, action)
            break
    console.print(f"\n[done][/done] resolved {total} group(s)")


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
    table.add_column("file", style="cyan")
    table.add_column("size")
    table.add_column("mtime")
    table.add_column("suggest")
    for a in ([ga.base] if ga.base else []) + ga.copies:
        if a is None:
            continue
        table.add_row(
            a.path.name, _human(a.info.size), _fmt_mtime(a.info.mtime), engine.suggest(a.path).value
        )
    console.print(table)
    if ga.all_equal:
        console.print("[dim]copies identical in content[/dim]")
    else:
        console.print("[yellow]copies differ[/yellow]")


def _prompt(group: ConflictGroup, ga: GroupAnalysis) -> Action:
    while True:
        menu = (
            "(b)ase (c)opy (e)ditor (d)iff (m)eld (v)iewer (o)ffice (k)eepass (h)keep-both (s)kip: "
        )
        ans = input(menu).strip().lower()
        if ans in ("b", "base"):
            return Action.keep_base()
        if ans in ("c", "copy"):
            target = ga.copies[0].path if ga.copies else (ga.base.path if ga.base else None)
            return Action.keep_copy(target) if target else Action.skip()
        if ans == "h":
            return Action.keep_both()
        if ans in ("s", "skip"):
            return Action.skip()
        tool = _tool_from_hint(ans, ga)
        if tool is not None:
            return tool
        print("invalid choice")


def _tool_from_hint(ans: str, ga: GroupAnalysis) -> Action | None:
    tool = {
        "e": ToolType.EDITOR,
        "m": ToolType.DIFF,
        "v": ToolType.VIEWER,
        "o": ToolType.OFFICE,
        "k": ToolType.KEEPASS,
    }.get(ans)
    if tool is None:
        return None
    target = ga.copies[0].path if ga.copies else (ga.base.path if ga.base else None)
    return Action.tool_action(tool, target) if target else None


def main() -> None:
    app()


if __name__ == "__main__":
    main()
