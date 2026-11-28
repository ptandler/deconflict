"""Recursive scan over dirs, grouping base files with their conflict copies."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING

from .paths import casefold
from .patterns import Pattern, match_name

if TYPE_CHECKING:
    from .patterns import ConflictMatch


@dataclass
class ConflictGroup:
    key: str
    pattern: str
    base: Path | None
    conflicts: list[Path] = field(default_factory=list)
    matches: dict[Path, ConflictMatch] = field(default_factory=dict)

    @property
    def files(self) -> list[Path]:
        out = list(self.conflicts)
        if self.base is not None:
            out.insert(0, self.base)
        return out


@dataclass
class ScanResult:
    dirs: list[Path]
    groups: list[ConflictGroup]

    def __bool__(self) -> bool:
        return bool(self.groups)


def iter_files(root: Path, follow_symlinks: bool = False) -> Iterable[Path]:
    """Yield regular file paths under root, skipping symlink loops."""
    if not root.is_dir():
        return
    for p in sorted(root.rglob("*")):
        if p.is_dir():
            continue
        if p.is_symlink() and not follow_symlinks:
            continue
        if p.is_file():
            yield p


def group_conflicts(files: Iterable[Path], patterns: list[Pattern]) -> list[ConflictGroup]:
    """Classify files into conflict groups keyed by the base path."""
    bases: dict[str, Path] = {}
    matches: dict[str, list[tuple[Path, object]]] = {}
    pattern_names: dict[str, str] = {}

    for path in files:
        name = path.name
        m = match_name(name, patterns)
        if m is not None:
            base = path.with_name(m.base_name)
            key = casefold(str(base))
            bases.setdefault(key, base)
            matches.setdefault(key, []).append((path, m))
            pattern_names[key] = m.pattern
        else:
            key = casefold(str(path))
            bases.setdefault(key, path)
            if key not in pattern_names:
                pattern_names[key] = ""

    groups: list[ConflictGroup] = []
    for key in sorted(matches):
        matched = matches[key]
        matches_map: dict[Path, object] = {p: m for p, m in matched}
        paths = [p for p, _ in matched]
        pattern = pattern_names.get(key) or ""
        groups.append(ConflictGroup(key, pattern, bases.get(key), paths, matches_map))
    return groups


def scan(
    dirs: Iterable[Path],
    patterns: list[Pattern],
    follow_symlinks: bool = False,
    on_file=None,
) -> ScanResult:
    """Scan multiple directories and group found conflict files.

    `on_file` (optional) is called once per emitted file path so a UI can show
    scan progress.
    """
    dir_list = list(dict.fromkeys(Path(d).expanduser() for d in dirs))
    files: list[Path] = []
    for d in dir_list:
        for p in iter_files(d, follow_symlinks):
            if on_file is not None:
                on_file(p)
            files.append(p)
    return ScanResult(dir_list, group_conflicts(files, patterns))
