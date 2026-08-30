"""Typer CLI: thin subcommands + no-args interactive driver. UI only; logic lives in the engine."""

from __future__ import annotations

import datetime
import difflib
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
from .media import metadata_fields
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
            "(see `deconflict config`); run `deconflict init-config` to create one."
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


_META_CELL_MAX = 70  # cap per-metadata-value length in on-screen tables


def _truncate_meta(value: str, limit: int = _META_CELL_MAX) -> str:
    """Shorten a long metadata value (e.g. office XML) for a table cell."""
    if value is None:
        return "∅"
    text = str(value)
    if len(text) <= limit:
        return text
    return text[:limit] + f"… (+{len(text) - limit} more)"


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
    if cfg.engine.file_type_tools:
        console.print(escape("[file_types]"))
        for kind, tool in cfg.engine.file_type_tools.items():
            console.print(f"  {kind} = {tool!r}")


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
            _apply(engine, group, action)
            break
    console.print(f"\n[done]resolved {total} group(s)")


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
                differ = _differing_fields(engine, ga, idx)
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


def _actions(engine: Engine, ga: GroupAnalysis) -> list[tuple[str, str, ToolType | str]]:
    """Ordered (hotkey, label, ToolType | inline-tag) tool actions for this kind.

    A ToolType means 'launch an external tool'; an inline-tag ('term_diff',
    'meta_view') means the action is handled directly in the prompt loop.
    """
    k = ga.kind
    a: list[tuple[str, str, ToolType | str]] = []
    la = engine.launcher()
    if k in ("text", "other"):
        if la.view_available(k):
            a.append(("v", "view", ToolType.VIEW))
        if k == "text":
            a.append(("d", "diff", "term_diff"))
        if engine.available_edit(ga) and k == "text":
            a.append(("e", "edit", ToolType.EDIT))
        if k == "text" and engine.available_diff(ga):
            a.append(("m", "meld", ToolType.DIFF))
    elif k == "audio":
        if la.view_available(k):
            a.append(("v", "listen", ToolType.VIEW))
        if engine.available_edit(ga):
            a.append(("e", "edit tags", ToolType.EDIT))
        if engine.available_diff(ga):
            a.append(("d", "meta-diff", ToolType.DIFF))
    elif k == "image":
        if la.view_available(k):
            a.append(("v", "view", ToolType.VIEW))
        if engine.available_edit(ga):
            a.append(("e", "edit", ToolType.EDIT))
        if engine.available_diff(ga):
            a.append(("d", "meta-diff", ToolType.DIFF))
        if engine.available_edit_meta(ga):
            a.append(("x", "edit exif", ToolType.EDIT_META))
    elif k == "video":
        if la.view_available(k):
            a.append(("v", "play", ToolType.VIEW))
        if engine.available_diff(ga):
            a.append(("d", "meta-diff", ToolType.DIFF))
    elif k == "office":
        if la.view_available(k):
            a.append(("v", "open", ToolType.VIEW))
        if engine.available_edit(ga):
            a.append(("e", "edit", ToolType.EDIT))
        if engine.available_diff(ga):
            a.append(("d", "diff", ToolType.DIFF))
    elif k == "kdbx":
        a.append(("v", "merge", ToolType.VIEW))
    if k == "text":
        pass
    elif _meta_differs(ga):
        a.append(("m", "metadata", "meta_view"))
    return a


def _prompt(engine: Engine, group: ConflictGroup, ga: GroupAnalysis) -> Action | None:
    target = ga.copies[0].path if ga.copies else (ga.base.path if ga.base else None)
    bykey: dict[str, tuple[str, ToolType | str]] = {
        key: (label, tool) for key, label, tool in _actions(engine, ga)
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
            if tool == "meta_view":
                _print_metadata_diff(engine, ga, 0)
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
    parts += [f"({key}){label}" for key, label, _tool in _actions(engine, ga)]
    parts += ["(h)keep-both", "(s)kip", "(q)uit", "(?)tools"]
    return " ".join(parts) + ": "


def _print_text_diff(group: ConflictGroup, target: Path | None) -> None:
    if target is None:
        console.print("[yellow]nothing to diff[/yellow]")
        return
    other = next((f for f in group.files if f.resolve() != target.resolve()), None)
    if other is None:
        console.print("[yellow]nothing to diff against[/yellow]")
        return
    try:
        a = other.read_text(encoding="utf-8", errors="replace").splitlines()
        b = target.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as exc:
        console.print(f"[red]diff failed: {exc}[/red]")
        return
    diff = list(difflib.unified_diff(a, b, fromfile=str(other), tofile=str(target), lineterm=""))
    if not diff:
        console.print("[dim]no textual difference[/dim]")
        return
    for line in diff:
        if line.startswith("+"):
            color = "green"
        elif line.startswith("-"):
            color = "red"
        else:
            color = "dim"
        console.print(f"[{color}]{escape(line)}[/{color}]")


def _meta_differs(ga: GroupAnalysis) -> bool:
    """True when any copy's metadata differs from the base (media kinds)."""
    return ga.base is not None and any(m is False for m in ga.meta)


FieldDicts = tuple[dict[str, str], dict[str, str]]


def _diff_fields(engine: Engine, ga: GroupAnalysis, idx: int) -> FieldDicts:
    """Metadata field dicts for (base, copy) at `idx` ({} when not extractable)."""
    if ga.base is None or idx >= len(ga.copies):
        return {}, {}
    bf = metadata_fields(ga.base.path, ga.kind, engine.tools.found)
    cf = metadata_fields(ga.copies[idx].path, ga.kind, engine.tools.found)
    return bf, cf


def _differing_fields(engine: Engine, ga: GroupAnalysis, idx: int) -> list[str]:
    """Short `field: value` lines for the copy's fields that differ from the base."""
    bf, cf = _diff_fields(engine, ga, idx)
    return [
        f"{k}: {_truncate_meta(cf.get(k, '∅'))}"
        for k in dict.fromkeys([*bf, *cf])
        if bf.get(k) != cf.get(k)
    ]


def _meta_diff_columns(ga: GroupAnalysis, idx: int, limit: int = 40) -> tuple[str, str]:
    """Short header labels for the metadata table: `#N <truncated-filename>`."""
    name = lambda p: p if len(p) <= limit else p[: limit - 1] + "…"  # noqa: E731
    base = f"#0 {name(ga.base.path.name)}" if ga.base else "#0"
    copy = f"#{idx + 1} {name(ga.copies[idx].path.name)}" if idx < len(ga.copies) else f"#{idx + 1}"
    return base, copy


def _print_metadata_diff(engine: Engine, ga: GroupAnalysis, idx: int) -> None:
    """A table with base and copy as columns; common and differing fields as rows."""
    if not _meta_differs(ga):
        console.print("[yellow]no metadata difference to show[/yellow]")
        return
    if idx >= len(ga.copies) or ga.base is None:
        console.print("[yellow]nothing to compare[/yellow]")
        return
    bf, cf = _diff_fields(engine, ga, idx)
    if not bf and not cf:
        console.print("[yellow]no extractable metadata[/yellow]")
        return
    base_label, copy_label = _meta_diff_columns(ga, idx)
    keys = list(dict.fromkeys([*bf, *cf]))
    common = [k for k in keys if bf.get(k) == cf.get(k)]
    diff = [k for k in keys if bf.get(k) != cf.get(k)]
    table = Table(title="metadata diff", header_style="bold")
    table.add_column("field")
    table.add_column(base_label, overflow="ellipsis")
    table.add_column(copy_label, overflow="ellipsis")
    for k in common:
        table.add_row(
            k, escape(_truncate_meta(bf.get(k))), escape(_truncate_meta(cf.get(k))), style="dim"
        )
    if common and diff:
        table.add_section()
    for k in diff:
        table.add_row(
            k,
            escape(_truncate_meta(bf.get(k, "∅"))),
            escape(_truncate_meta(cf.get(k, "∅"))),
        )
    console.print(table)
    if diff:
        console.print("[dim]differing rows shown in plain; shared fields dimmed[/dim]")


def _print_tools(engine: Engine, ga: GroupAnalysis) -> None:
    tool_key = engine.launcher().kind_tool(ga.kind)
    console.print(f"[bold]file kind:[/bold] {ga.kind} → view tool: {tool_key}")
    console.print("[bold]tools:[/bold]")
    for name, found, candidates, hint in engine.tool_status():
        if found:
            console.print(f"  [green]{name}[/green]: {found}")
        else:
            suffix = f"  ({hint})" if hint else ""
            console.print(f"  [red]{name}[/red]: missing ({', '.join(candidates)}){suffix}")


def main() -> None:
    app()


if __name__ == "__main__":
    main()
