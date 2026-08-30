"""Per-group analysis: size, hash, mtime, content equality, text/binary sniff."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from pathlib import Path

from .media import file_kind, metadata_diff, metadata_equal

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
    meta: list[bool | None] = field(default_factory=list)  # per copy vs base
    meta_diff: list[str | None] = field(default_factory=list)  # per copy vs base

    def winner_hint(self) -> str | None:
        """'base' or a copy path if one file is clearly newest by mtime."""
        items = [a for a in ([self.base] if self.base else []) + self.copies if a]
        if not items:
            return None
        newest = max(items, key=lambda a: a.info.mtime)
        return "base" if (self.base and newest is self.base) else str(newest.path)


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
    if base_a is not None and kind in ("audio", "image", "video", "office"):
        for i, c in enumerate(copies):
            meta[i] = metadata_equal(base, c.path, kind, tools)
            meta_diff[i] = metadata_diff(base, c.path, kind, tools)
    return GroupAnalysis(base_a, copies, all_equal, all_text, sizes, hashes, kind, meta, meta_diff)
