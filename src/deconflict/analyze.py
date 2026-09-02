"""Per-group analysis: size, hash, mtime, content equality, text/binary sniff."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from pathlib import Path

from .media import file_kind, metadata_diff, metadata_equal, metadata_fields

CHUNK = 1 << 16
_TEXT_MAGIC_LEN = 8192


@dataclass(frozen=True)
class FileInfo:
    path: Path
    size: int
    mtime: float
    sha: str
    is_text: bool


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(CHUNK):
            h.update(chunk)
    return h.hexdigest()


def sniff_text(path: Path) -> bool:
    """Heuristic: no NUL bytes in the first few KB => likely text."""
    try:
        with open(path, "rb") as f:
            head = f.read(_TEXT_MAGIC_LEN)
    except OSError:
        return False
    return b"\x00" not in head


def _stamp_ts(match, default=None):
    stamp = getattr(match, "stamp", None)
    if stamp is None:
        return default
    return stamp


@dataclass
class FileAnalysis:
    path: Path
    info: FileInfo
    name_stamp: str | None = None


@dataclass
class GroupAnalysis:
    base: FileAnalysis | None
    copies: list[FileAnalysis]
    all_equal: bool
    all_text: bool
    sizes: dict[str, int]
    hashes: dict[str, str]
    kind: str = "text"
    meta: list[bool | None] = field(default_factory=list)  # per copy vs base (content metadata)
    meta_diff: list[str | None] = field(default_factory=list)  # per copy vs base
    # Full metadata field dicts (fs attrs + content) aligned with rows ([base]? + copies).
    meta_fields: list[dict[str, str]] = field(default_factory=list)

    def winner_hint(self) -> str | None:
        """'base' or a copy path if one file is clearly newest by mtime."""
        items = [a for a in ([self.base] if self.base else []) + self.copies if a]
        if not items:
            return None
        newest = max(items, key=lambda a: a.info.mtime)
        return "base" if (self.base and newest is self.base) else str(newest.path)


@dataclass(frozen=True)
class Recommendation:
    """A clear winner for a group: which file to keep and a human-readable label."""

    winner: FileAnalysis
    label: str  # e.g. "#0 base 'x' (all identical)" / "#1 'y' (newest)"
    is_base: bool  # True when winner is the base file
    notes: tuple[str, ...] = ()

    def explain(self) -> str:
        return f"recommend keep {self.label}"


@dataclass(frozen=True)
class Consideration:
    """Diagnostic observations about a group, even when there is no clear winner.

    Gives the user visibility into what was looked at (newest, largest, superset,
    size deltas) so they can judge why no / a given recommendation was made.
    """

    notes: tuple[str, ...] = ()
    recommendation: Recommendation | None = None


def _text_superset(base: FileAnalysis, newest: FileAnalysis) -> bool:
    """True when `newest`'s text contains all of `base`'s non-blank lines (plus something).

    A base that is only whitespace/blank lines is treated as empty, so a copy that
    adds real content supersedes it (a genuinely improved, not regressed, file).
    """
    try:
        a_lines = base.path.read_text(encoding="utf-8", errors="replace").splitlines()
        b_lines = newest.path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return False
    a = {ln.strip() for ln in a_lines if ln.strip()}
    b = {ln.strip() for ln in b_lines if ln.strip()}
    return a <= b and a != b


def _human(p: Path) -> str:
    return f"'{p.name}'"


def _file_tag(a: FileAnalysis, base: FileAnalysis | None, copies: list[FileAnalysis]) -> str:
    """A short '#N' tag for a file (base or copy) to name it in diagnostics."""
    if base is not None and a is base:
        return "#0"
    if copies:
        idx = next((i for i, c in enumerate(copies) if c is a), None)
        if idx is not None:
            return f"#{idx + 1}"
    return "?"


def consider(ga: GroupAnalysis) -> Consideration:
    """Gather what was evaluated for this group, with the resulting recommendation.

    Works whether or not a clear winner exists: the `notes` list always describes
    the newest / largest files and which checks passed or failed, so the frontend
    can show the reasoning to the user.
    """
    notes: list[str] = []
    if ga.base is None:
        return Consideration(("no base file — nothing to recommend",), None)
    items = [ga.base, *ga.copies]
    newest = max(items, key=lambda a: a.info.mtime)
    largest = max(items, key=lambda a: a.info.size)
    notes.append(
        f"newest is {_file_tag(newest, ga.base, ga.copies)} {_human(newest.path)} "
        f"({round(newest.info.mtime)})"
    )
    notes.append(
        f"largest is {_file_tag(largest, ga.base, ga.copies)} {_human(largest.path)} "
        f"({largest.info.size} B) vs base {ga.base.info.size} B"
    )
    if ga.all_equal:
        notes.append("all files identical in content")
        return Consideration(
            tuple(notes),
            Recommendation(ga.base, "#0 base (all files identical)", is_base=True),
        )
    if newest is not ga.base:
        if ga.kind == "text":
            superset = _text_superset(ga.base, newest)
            tag = _file_tag(newest, ga.base, ga.copies)
            if superset:
                notes.append(f"{tag} text is a superset of base (content added, nothing removed)")
            else:
                notes.append(
                    f"{tag} text is NOT a superset of base (content changed/removed) — "
                    "no recommendation"
                )
        elif newest.info.size <= ga.base.info.size:
            tag = _file_tag(newest, ga.base, ga.copies)
            notes.append(
                f"{tag} is newer but smaller than base — possible content/metadata loss, "
                "no recommendation"
            )
        else:
            tag = _file_tag(newest, ga.base, ga.copies)
            notes.append(f"{tag} is newer and larger than base")
    rec = recommend(ga)
    if rec is not None:
        notes.append(rec.explain())
    else:
        notes.append("no clear recommendation (no single file wins on all checks)")
    return Consideration(tuple(notes), rec)


def recommend(ga: GroupAnalysis) -> Recommendation | None:
    """A clear winner (the file to keep) or None.

    - All files identical in content -> keep base.
    - One file is strictly-newest:
        * text: newest's content is a superset of the base's (added content, none
          removed) -> keep it; ALSO keep the newer file when the base is effectively
          empty (only blank/whitespace lines) and the copy has real content.
        * non-text: keep newest only if it did NOT lose size vs the base (content
          was clearly added, not deleted). A strictly-smaller newest is a red flag
          (possible metadata/content loss), so withhold the recommendation.
    """
    if ga.base is None:
        return None
    if ga.all_equal:
        return Recommendation(ga.base, "#0 base (all files identical)", is_base=True)
    items = [ga.base, *ga.copies]
    newest = max(items, key=lambda a: a.info.mtime)
    non_newest = [a for a in items if a is not newest]
    if not all(n.info.mtime < newest.info.mtime for n in non_newest):
        return None
    if newest is ga.base:
        return Recommendation(
            ga.base,
            f"#0 base '{ga.base.path.name}' (newest)",
            is_base=True,
        )
    # newest is a copy: require it to supersede the base (no content/metadata loss).
    if ga.kind == "text":
        if not _text_superset(ga.base, newest):
            return None
    elif newest.info.size <= ga.base.info.size:
        # newer copy that is not bigger -> possible content/metadata loss.
        return None
    idx = next((i for i, c in enumerate(ga.copies) if c is newest), None)
    num = "#1" if idx is None else f"#{idx + 1}"
    return Recommendation(
        newest,
        f"{num} '{newest.path.name}' (newest)",
        is_base=False,
    )


def _info(path: Path) -> FileInfo:
    st = path.stat()
    return FileInfo(
        path=path,
        size=st.st_size,
        mtime=st.st_mtime,
        sha=sha256(path),
        is_text=sniff_text(path),
    )


def analyze_group(
    group, base: Path | None, conflicts: list[Path], tools: dict[str, str] | None = None
) -> GroupAnalysis:
    analyses: list[FileAnalysis] = []
    if base is not None:
        analyses.append(FileAnalysis(base, _info(base)))
    for c in conflicts:
        match = group.matches.get(c)
        stamp = getattr(match, "stamp", None) if match is not None else None
        analyses.append(FileAnalysis(c, _info(c), name_stamp=stamp))

    base_a = analyses[0] if base is not None else None
    copies = analyses[1:] if base is not None else analyses

    hashes = {a.path.name: a.info.sha for a in analyses}
    sizes = {a.path.name: a.info.size for a in analyses}
    all_equal = len(set(hashes.values())) == 1
    all_text = all(a.info.is_text for a in analyses)
    first = base_a or (copies[0] if copies else None)
    kind = file_kind(first.path, first.info.is_text) if first else "text"
    meta: list[bool | None] = [None] * len(copies)
    meta_diff: list[str | None] = [None] * len(copies)
    meta_fields: list[dict[str, str]] = [metadata_fields(a.path, kind, tools) for a in analyses]
    if base_a is not None:
        for i, c in enumerate(copies):
            meta[i] = metadata_equal(base, c.path, kind, tools)
            meta_diff[i] = metadata_diff(base, c.path, kind, tools)
    return GroupAnalysis(
        base_a, copies, all_equal, all_text, sizes, hashes, kind, meta, meta_diff, meta_fields
    )
