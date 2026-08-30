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
from .config import Config, default_path, load, write_init_config
from .engine import Action, ActionKind, Engine
from .launchers import ToolType
from .media import ATTR_CREATED, ATTR_MODIFIED, ATTR_SIZE, META_ATTR_FIELDS
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

_ATTR_FIELDS = set(META_ATTR_FIELDS)  # fs-attribute field names (shown, not "diff" markers)


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


def _actions(engine: Engine, ga: GroupAnalysis) -> list[tuple[str, str, ToolType | str]]:
    """Ordered (hotkey, label, ToolType | inline-tag) tool actions for this kind.

    A ToolType means 'launch an external tool'; an inline-tag ('term_diff',
    'office_diff', 'meta_view') means the action is handled directly in the
    prompt loop.
    """
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
    parts += [f"({key}){label}" for key, label, _tool in _actions(engine, ga)]
    parts += ["(h)keep-both", "(s)kip", "(q)uit", "(?)tools"]
    return " ".join(parts) + ": "


def _hl_runs(old: str, new: str) -> tuple[str, str]:
    """Return (old, new) lines with the differing character runs wrapped in markers.

    Uses SequenceMatcher so only the changed characters are highlighted; identical
    bulk stays plain. Returns the plain lines when both are empty/unchanged.
    """
    from difflib import SequenceMatcher

    matcher = SequenceMatcher(None, old, new)
    old_parts: list[str] = []
    new_parts: list[str] = []
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag == "equal":
            old_parts.append(old[i1:i2])
            new_parts.append(new[j1:j2])
        else:
            old_parts.append(f"{{+}}{old[i1:i2]}{{/}}")
            new_parts.append(f"{{+}}{new[j1:j2]}{{/}}")
    return "".join(old_parts), "".join(new_parts)


def _print_rich_line(sign: str, text: str, base_color: str) -> None:
    """Print a diff line (sign + text) with the {+...+/} runs colored differently."""
    # walk segments split on {+ ... +/} markers; highlight deltas with a brighter style
    out: list[str] = []
    pos = 0
    while True:
        start = text.find("{+", pos)
        if start == -1:
            out.append(f"[{base_color}]{escape(text[pos:])}[/{base_color}]")
            break
        out.append(f"[{base_color}]{escape(text[pos:start])}[/{base_color}]")
        end = text.find("{/}", start)
        if end == -1:
            out.append(f"[{base_color}]{escape(text[start:])}[/{base_color}]")
            break
        seg = text[start + 2 : end]
        hi = "bold magenta" if base_color == "red" else "bold cyan"
        out.append(f"[{hi}]{escape(seg)}[/{hi}]")
        pos = end + 3
    console.print(sign + "".join(out))


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
    # Render hunks as old/new pairs with char-level highlight on changed lines.
    pending_removed: list[str] = []
    for line in diff:
        if line.startswith("---") or line.startswith("+++") or line.startswith("@@"):
            console.print(f"[bold yellow]{escape(line)}[/bold yellow]")
            continue
        if line.startswith("-"):
            pending_removed.append(line[1:])
            continue
        if line.startswith("+"):
            new = line[1:]
            if pending_removed:
                old = pending_removed.pop(0)
                # paired change -> highlight only the differing characters
                if old == new:
                    console.print(f"[dim] {escape(old)}[/dim]")
                    continue
                hl_old, hl_new = _hl_runs(old, new)
                _print_rich_line("[red]-[/red] ", hl_old, "red")
                _print_rich_line("[green]+[/green] ", hl_new, "green")
            else:
                _print_rich_line("[green]+[/green] ", new, "green")
            continue
        # context line: flush any orphaned removed lines, then print context dim
        for rem in pending_removed:
            _print_rich_line("[red]-[/red] ", rem, "red")
        pending_removed = []
        console.print(f"[dim] {escape(line)}[/dim]")
    for rem in pending_removed:
        _print_rich_line("[red]-[/red] ", rem, "red")


def _print_office_diff(group: ConflictGroup, target: Path | None) -> None:
    """Diff two office docs by their extracted text (avoids soffice --compare, which
    this LibreOffice build rejects; also makes office (d)iff work headlessly)."""
    from .media import _extract_office_text

    if target is None:
        console.print("[yellow]nothing to diff[/yellow]")
        return
    other = next((f for f in group.files if f.resolve() != target.resolve()), None)
    if other is None:
        console.print("[yellow]nothing to diff against[/yellow]")
        return
    a = _extract_office_text(other).splitlines()
    b = _extract_office_text(target).splitlines()
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


def _diff_fields(ga: GroupAnalysis, idx: int) -> FieldDicts:
    """Metadata field dicts for (base, copy) at `idx`, read from analysis (no re-extract)."""
    if ga.base is None or idx >= len(ga.copies):
        return {}, {}
    base = ga.meta_fields[0] if ga.meta_fields else {}
    copy = ga.meta_fields[idx + 1] if idx + 1 < len(ga.meta_fields) else {}
    return base, copy


def _differing_fields(ga: GroupAnalysis, idx: int) -> list[str]:
    """Short `field: value` lines for the copy's CONTENT fields that differ from base."""
    bf, cf = _diff_fields(ga, idx)
    return [
        f"{k}: {_truncate_meta(cf.get(k, '∅'))}"
        for k in dict.fromkeys([*bf, *cf])
        if bf.get(k) != cf.get(k) and k not in _ATTR_FIELDS
    ]


_META_HEADER_TAIL = 26  # trailing chars kept on header truncation (conflict pattern/date+ext)


def _truncate_name(name: str, limit: int, tail: int = _META_HEADER_TAIL) -> str:
    """Keep both a start AND a meaningful tail when the name overflows the limit.

    Truncation shows `head … tail` instead of only `head …`: the tail preserves
    the conflict-pattern suffix (date + time + extension) that's otherwise cut
    off — while `limit` stays the total printed width.
    """
    if len(name) <= limit:
        return name
    tail = min(tail, limit - 2)
    head = limit - 1 - tail
    return name[:head] + "…" + name[-tail:]


def _meta_diff_columns(ga: GroupAnalysis, idx: int, limit: int = 40) -> tuple[str, str]:
    """Short header labels for the metadata table: `#N <truncated-filename>`.

    Truncation keeps the trailing conflict-pattern chars (see `_truncate_name`).
    """
    name = lambda p: _truncate_name(p, limit)  # noqa: E731
    base = f"#0 {name(ga.base.path.name)}" if ga.base else "#0"
    copy = f"#{idx + 1} {name(ga.copies[idx].path.name)}" if idx < len(ga.copies) else f"#{idx + 1}"
    return base, copy


def _print_metadata_diff(ga: GroupAnalysis, idx: int) -> None:
    """ONE combined table with base and copy as columns.

    Merges the generic file attributes (size/created/modified) and the per-kind
    metadata fields into a single table. Reads stored `ga.meta_fields` (set once
    during analysis), so nothing is re-extracted. The `size` row shows both the
    human-readable size and the byte count, plus the diff vs the other file.
    """
    if idx >= len(ga.copies) or ga.base is None:
        console.print("[yellow]nothing to compare[/yellow]")
        return
    bf, cf = _diff_fields(ga, idx)
    if not bf and not cf:
        console.print("[yellow]no extractable metadata[/yellow]")
        return
    base_label, copy_label = _meta_diff_columns(ga, idx)
    keys = list(dict.fromkeys([*bf, *cf]))
    content = [k for k in keys if k not in _ATTR_FIELDS]
    common = [k for k in content if bf.get(k) == cf.get(k)]
    diff = [k for k in content if bf.get(k) != cf.get(k)]

    table = Table(title="metadata diff", header_style="bold")
    table.add_column("field")
    table.add_column(base_label, overflow="ellipsis")
    table.add_column(copy_label, overflow="ellipsis")
    # Generic attrs: dim the whole row when the two files' values are identical,
    # matching how identical content rows are dimmed below.
    copy = ga.copies[idx]
    sizes_equal = ga.base.info.size == copy.info.size
    table.add_row(
        ATTR_SIZE,
        _size_meta_cell(ga.base.info.size, None),
        _size_meta_cell(copy.info.size, ga.base.info.size),
        style="dim" if sizes_equal else None,
    )
    created_equal = bf.get(ATTR_CREATED) == cf.get(ATTR_CREATED)
    table.add_row(
        ATTR_CREATED,
        escape(_truncate_meta(bf.get(ATTR_CREATED, ""))),
        escape(_truncate_meta(cf.get(ATTR_CREATED, ""))),
        style="dim" if created_equal else None,
    )
    modified_equal = bf.get(ATTR_MODIFIED) == cf.get(ATTR_MODIFIED)
    table.add_row(
        ATTR_MODIFIED,
        escape(_truncate_meta(bf.get(ATTR_MODIFIED, ""))),
        escape(_truncate_meta(cf.get(ATTR_MODIFIED, ""))),
        style="dim" if modified_equal else None,
    )

    if content:
        table.add_section()
        for k, bv, cv in _row_cells(bf, cf, common):
            table.add_row(k, bv, cv, style="dim")
        if common and diff:
            table.add_section()
        for k, bv, cv in _row_cells(bf, cf, diff):
            table.add_row(k, bv, cv)
    console.print(table)
    if diff:
        console.print("[dim]differing fields for copy vs base (shared rows dimmed)[/dim]")


def _row_cells(bf, cf, keys: list[str]) -> list[tuple[str, str, str]]:
    """(field, base-cell, copy-cell) rows, truncated and escaped."""
    out = []
    for k in keys:
        out.append(
            (k, escape(_truncate_meta(bf.get(k, ""))), escape(_truncate_meta(cf.get(k, ""))))
        )
    return out


def _size_meta_cell(size: int, other: int | None) -> str:
    """Size cell: human-readable + bytes, with diff vs the other file."""
    cell = f"{_human(size)} ({size:,} bytes)"
    if other is None:
        return cell
    delta = size - other
    if delta == 0:
        return f"{cell} [dim](=)[/dim]"
    arrow = "↑" if delta > 0 else "↓"
    bit = f" {_human(abs(delta))}" if delta else ""
    return f"{cell} [yellow]({arrow}{bit})[/yellow]"


def _print_tools(engine: Engine, ga: GroupAnalysis) -> None:
    la = engine.launcher()
    cfg_path = engine.cfg.config_path or default_path()
    console.print(f"[bold]file kind:[/bold] {ga.kind}")
    console.print(f"  view: {la.kind_tool(ga.kind)} · edit: {la.edit_tool(ga.kind)}")
    console.print("[bold]tools used for this file:[/bold]")
    for key in la.tools_for(ga.kind):
        found = engine.tools.get(key)
        if found:
            console.print(f"  [green]{key}[/green]: {found}")
        else:
            hint = engine.tools.hint(key)
            suffix = f"  ({hint})" if hint else ""
            console.print(
                f"  [red]{key}[/red]: missing ({', '.join(engine.tools.candidates(key))}){suffix}"
            )
    console.print(
        f"[dim]configure tools in ({cfg_path}) — sections [tools], [file_types], "
        "[file_types_edit]; run `deconflict init-config` to write a template.[/dim]"
    )


def main() -> None:
    app()


if __name__ == "__main__":
    main()
