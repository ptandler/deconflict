"""UI-agnostic Rich renderables for deconflict output.

These builders RETURN Rich renderables (Table / Group / RenderGroup) instead of
printing, so any frontend (CLI console, Textual TUI) can render the same
content. The CLI wraps them in `console.print(...)`; the TUI mounts them in
widgets. No Rich `Console` is constructed here — renderables are pure.

Origin: extracted from cli.py's *_print_* helpers (which printed directly) so
the terminal diff/metadata output is shared, not re-implemented.
"""

from __future__ import annotations

import datetime
import difflib
from pathlib import Path

from rich.markup import escape
from rich.table import Table
from rich.text import Text

from .analyze import GroupAnalysis
from .media import ATTR_CREATED, ATTR_MODIFIED, ATTR_SIZE, META_ATTR_FIELDS

# Packaged here so the TUI and CLI share identical formatting.
_ATTR_FIELDS = set(META_ATTR_FIELDS)

_META_CELL_MAX = 70  # cap per-metadata-value length in on-screen tables
_META_HEADER_TAIL = 26  # trailing chars kept on header truncation (conflict pattern/date+ext)


def human(n: int) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024
    return str(n)


def fmt_mtime(t: float) -> str:
    return datetime.datetime.fromtimestamp(t).strftime("%Y-%m-%d %H:%M")


def truncate_meta(value, limit: int = _META_CELL_MAX) -> str:
    """Shorten a long metadata value (e.g. office XML) for a table cell."""
    if value is None:
        return "∅"
    text = str(value)
    if len(text) <= limit:
        return text
    return text[:limit] + f"… (+{len(text) - limit} more)"


def truncate_name(name: str, limit: int, tail: int = _META_HEADER_TAIL) -> str:
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


def size_meta_cell(size: int, other: int | None) -> str:
    """Size cell: human-readable + bytes, with diff vs the other file."""
    cell = f"{human(size)} ({size:,} bytes)"
    if other is None:
        return cell
    delta = size - other
    if delta == 0:
        return f"{cell} [dim](=)[/dim]"
    arrow = "↑" if delta > 0 else "↓"
    bit = f" {human(abs(delta))}" if delta else ""
    return f"{cell} [yellow]({arrow}{bit})[/yellow]"


def _diff_fields(ga: GroupAnalysis, idx: int):
    """Metadata field dicts for (base, copy) at `idx`, read from analysis (no re-extract)."""
    if ga.base is None or idx >= len(ga.copies):
        return {}, {}
    base = ga.meta_fields[0] if ga.meta_fields else {}
    copy = ga.meta_fields[idx + 1] if idx + 1 < len(ga.meta_fields) else {}
    return base, copy


def _meta_diff_columns(ga: GroupAnalysis, idx: int, limit: int = 40) -> tuple[str, str]:
    """Short header labels for the metadata table: `#N <truncated-filename>`.

    Truncation keeps the trailing conflict-pattern chars (see `truncate_name`).
    """
    name = lambda p: truncate_name(p, limit)  # noqa: E731
    base = f"#0 {name(ga.base.path.name)}" if ga.base else "#0"
    copy = f"#{idx + 1} {name(ga.copies[idx].path.name)}" if idx < len(ga.copies) else f"#{idx + 1}"
    return base, copy


def _hl(cell: str, flag: bool, style: str = "bold") -> str:
    """Wrap a rendered cell in Rich markup when the condition holds (else unchanged)."""
    return f"[{style}]{cell}[/{style}]" if flag else cell


def _row_cells(bf, cf, keys: list[str]) -> list[tuple[str, str, str]]:
    """(field, base-cell, copy-cell) rows, truncated and escaped."""
    out = []
    for k in keys:
        out.append((k, escape(truncate_meta(bf.get(k, ""))), escape(truncate_meta(cf.get(k, "")))))
    return out


def _content_cell(ga: GroupAnalysis, copy) -> tuple[str, str]:
    """(base-cell, copy-cell) for the content row: identical or a brief diff.

    Uses the stored hash from analysis; for text/office kinds a short snippet of
    the differences is shown. Cells are escaped markup strings.
    """
    if ga.base is None or ga.base.info.sha == copy.info.sha:
        return ("[green]identical[/green]", "=")
    if ga.kind in ("text", "office"):
        return ("", _brief_text_diff(ga.base.path, copy.path))
    return ("[yellow]differs[/yellow]", "≠")


def _brief_text_diff(base: Path, copy: Path) -> str:
    """A one-line snippet of the first differing region for the content row."""
    from difflib import unified_diff

    def read(p):
        try:
            return p.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            return []

    for line in unified_diff(read(base), read(copy), lineterm=""):
        if line.startswith("+") and not line.startswith("+++"):
            return escape(truncate_meta(line[1:], 60))
    return "[yellow]≠[/yellow]"


def metadata_diff_table(ga: GroupAnalysis, idx: int) -> Table | None:
    """ONE combined metadata-diff table with base and copy as columns.

    Merges the generic file attributes (size/created/modified) and the per-kind
    metadata fields into a single table. Reads stored `ga.meta_fields` (set once
    during analysis), so nothing is re-extracted. Returns None when there is
    nothing to compare.
    """
    if idx >= len(ga.copies) or ga.base is None:
        return None
    bf, cf = _diff_fields(ga, idx)
    if not bf and not cf:
        return None
    base_label, copy_label = _meta_diff_columns(ga, idx)
    keys = list(dict.fromkeys([*bf, *cf]))
    content = [k for k in keys if k not in _ATTR_FIELDS]
    common = [k for k in content if bf.get(k) == cf.get(k)]
    diff = [k for k in content if bf.get(k) != cf.get(k)]

    table = Table(title="metadata diff", header_style="bold")
    table.add_column("field")
    table.add_column(base_label, overflow="ellipsis")
    table.add_column(copy_label, overflow="ellipsis")
    copy = ga.copies[idx]
    content_cell = _content_cell(ga, copy)
    table.add_row("content", *content_cell)
    if content_cell[0] or content_cell[1]:
        table.add_section()
    sizes_equal = ga.base.info.size == copy.info.size
    bigger = (
        "base"
        if ga.base.info.size > copy.info.size
        else "copy"
        if copy.info.size > ga.base.info.size
        else None
    )
    table.add_row(
        ATTR_SIZE,
        _hl(size_meta_cell(ga.base.info.size, None), bigger == "base"),
        _hl(size_meta_cell(copy.info.size, ga.base.info.size), bigger == "copy"),
        style="dim" if sizes_equal else None,
    )
    created_equal = bf.get(ATTR_CREATED) == cf.get(ATTR_CREATED)
    table.add_row(
        ATTR_CREATED,
        escape(truncate_meta(bf.get(ATTR_CREATED, ""))),
        escape(truncate_meta(cf.get(ATTR_CREATED, ""))),
        style="dim" if created_equal else None,
    )
    modified_equal = bf.get(ATTR_MODIFIED) == cf.get(ATTR_MODIFIED)
    newer = (
        "base"
        if ga.base.info.mtime > copy.info.mtime
        else "copy"
        if copy.info.mtime > ga.base.info.mtime
        else None
    )
    table.add_row(
        ATTR_MODIFIED,
        _hl(
            escape(truncate_meta(bf.get(ATTR_MODIFIED, ""))),
            newer == "base",
            style="bold cyan",
        ),
        _hl(
            escape(truncate_meta(cf.get(ATTR_MODIFIED, ""))),
            newer == "copy",
            style="bold cyan",
        ),
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
    return table


def _hl_runs(old: str, new: str) -> tuple[Text, Text]:
    """Return (old, new) Text lines with the differing character runs highlighted.

    Uses SequenceMatcher so only the changed characters are highlighted; identical
    bulk stays plain. `old` is styled red, `new` green, with changed runs in a
    brighter style.
    """
    matcher = difflib.SequenceMatcher(None, old, new)
    old_t = Text()
    new_t = Text()
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag == "equal":
            old_t.append(old[i1:i2])
            new_t.append(new[j1:j2])
        else:
            old_t.append(old[i1:i2], style="bold magenta")
            new_t.append(new[j1:j2], style="bold cyan")
    return old_t, new_t


def _signed_diff_line(sign: str, color: str, body: Text) -> Text:
    """A diff line: colored sign prefix + the (per-run styled) `body`."""
    line = Text()
    line.append(sign + " ", style="bold " + color)
    line.append_text(body)
    return line


def text_diff_lines(other: Path, target: Path) -> list[Text] | None:
    """Char-highlighted unified diff lines between two text files.

    Returns a list of styled `Text` lines, or None when there is no textual
    difference / unreadable input. Multi-line; callers page/scroll it.
    """
    try:
        a = other.read_text(encoding="utf-8", errors="replace").splitlines()
        b = target.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return None
    diff = list(difflib.unified_diff(a, b, fromfile=str(other), tofile=str(target), lineterm=""))
    if not diff:
        return None
    out: list[Text] = []
    pending_removed: list[str] = []
    for line in diff:
        if line.startswith("---") or line.startswith("+++") or line.startswith("@@"):
            out.append(Text(line, style="bold yellow"))
            continue
        if line.startswith("-"):
            pending_removed.append(line[1:])
            continue
        if line.startswith("+"):
            new = line[1:]
            if pending_removed:
                old = pending_removed.pop(0)
                if old == new:
                    out.append(Text(" " + old, style="dim"))
                    continue
                hl_old, hl_new = _hl_runs(old, new)
                out.append(_signed_diff_line("-", "red", hl_old))
                out.append(_signed_diff_line("+", "green", hl_new))
            else:
                out.append(_signed_diff_line("+", "green", Text(new)))
            continue
        for rem in pending_removed:
            out.append(_signed_diff_line("-", "red", Text(rem)))
        pending_removed = []
        out.append(Text(" " + line, style="dim"))
    for rem in pending_removed:
        out.append(_signed_diff_line("-", "red", Text(rem)))
    return out


def office_diff_lines(other: Path, target: Path) -> list[Text] | None:
    """Diff two office docs by their extracted text (headless, no soffice --compare)."""
    from .media import _extract_office_text

    a = _extract_office_text(other).splitlines()
    b = _extract_office_text(target).splitlines()
    diff = list(difflib.unified_diff(a, b, fromfile=str(other), tofile=str(target), lineterm=""))
    if not diff:
        return None
    out: list[Text] = []
    for line in diff:
        if line.startswith("+"):
            color = "green"
        elif line.startswith("-"):
            color = "red"
        else:
            color = "dim"
        out.append(Text(line, style=color))
    return out


def tools_help(launcher, tools, kind: str, config_path: Path) -> Table:
    """Renderable listing the tools backing `kind` (found/missing + hints).

    Shared by the CLI (?)tools action and the TUI config action. The
    launcher/tools objects are passed in (not imported here) so render.py stays
    dependency-light and UI-free.
    """
    table = Table(
        title=f"tools for file kind: {kind}",
        title_justify="left",
        show_header=False,
        box=None,
        caption=escape(
            f"configure in {config_path} — sections [tools], [file_types], [file_types_edit]"
        ),
        caption_justify="left",
    )
    table.add_column(style="bold", no_wrap=True)
    table.add_column()
    table.add_row("view", launcher.kind_tool(kind))
    table.add_row("edit", launcher.edit_tool(kind))
    for key in launcher.tools_for(kind):
        found = tools.get(key)
        if found:
            table.add_row(key, Text(found, style="green"))
        else:
            hint = tools.hint(key)
            suffix = f"  ({hint})" if hint else ""
            table.add_row(
                key,
                Text(f"missing ({', '.join(tools.candidates(key))}){suffix}", style="red"),
            )
    return table
