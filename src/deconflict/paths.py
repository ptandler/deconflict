"""Path helpers: OS-aware path construction, backup location, safe names."""

from __future__ import annotations

import os
import re
from pathlib import Path

_ILLEGAL = re.compile(r'[<>:"/\\|?*\x00-\x1f]')

CASE_INSENSITIVE = os.name == "nt"


def expand(path: str | Path) -> Path:
    """Expand ~ and env vars to an absolute Path."""
    return Path(os.path.expandvars(os.path.expanduser(str(path)))).resolve()


def sanitize(name: str) -> str:
    """Make a name safe for use inside a backup directory on any OS."""
    cleaned = _ILLEGAL.sub("_", name)
    cleaned = cleaned.rstrip(" .")
    return cleaned or "_"


def casefold(value: str) -> str:
    """Case-insensitive fold on NT, exact on POSIX."""
    return value.casefold() if CASE_INSENSITIVE else value


def same_file(a: Path, b: Path) -> bool:
    """True if two paths denote the same file (case-insensitive on NT)."""
    if casefold(str(a.resolve())) == casefold(str(b.resolve())):
        return True
    return os.path.samefile(a, b) if (a.exists() and b.exists()) else False


def is_within(path: Path, root: Path) -> bool:
    """True if `path` is inside `root` (both resolved)."""
    try:
        path.resolve().relative_to(root.resolve())
        return True
    except ValueError:
        return False
