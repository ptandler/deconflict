"""JSON file cache of scan results, keyed by dir set."""

from __future__ import annotations

import json
import os
import time
from collections.abc import Iterable
from pathlib import Path

from .paths import expand


class FileCache:
    """Persists per-file metadata (size/mtime/sha/is_text) keyed by the scanned dir set."""

    def __init__(self, cache_dir: str | Path, key: str) -> None:
        self.cache_dir = expand(str(cache_dir))
        self.key = key
        self.path = self.cache_dir / f"{self.key}.json"
        self._data: dict[str, dict] = {}
        self._dirty = False

    def _load(self) -> None:
        if self._data or not self.path.exists():
            return
        try:
            self._data = json.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            self._data = {}

    def get(self, path: Path, size: int, mtime: float) -> dict | None:
        """Return cached metadata for `path` if it is unchanged by size+mtime."""
        self._load()
        entry = self._data.get(str(path))
        if entry and entry.get("size") == size and abs(entry.get("mtime", -1) - mtime) < 1e-6:
            return entry
        return None

    def put(self, path: Path, size: int, mtime: float, sha: str, is_text: bool) -> None:
        self._load()
        self._data[str(path)] = {
            "size": size,
            "mtime": mtime,
            "sha": sha,
            "is_text": is_text,
        }
        self._dirty = True

    def save(self) -> None:
        if not self._dirty:
            return
        self.cache_dir.mkdir(parents=True, exist_ok=True)
        tmp = self.path.with_suffix(".json.tmp")
        tmp.write_text(json.dumps(self._data, sort_keys=True), encoding="utf-8")
        os.replace(tmp, self.path)
        self._dirty = False

    def clear(self) -> None:
        if self.path.exists():
            self.path.unlink()
        self._data = {}


def cache_key(dirs: Iterable[Path]) -> str:
    """Derive a stable cache filename from the ordered, resolved dir set."""
    parts = "_".join(str(p.resolve()) for p in dirs)
    import hashlib

    return hashlib.sha1(parts.encode()).hexdigest()[:16]


def _file_is_stale(data: dict) -> bool:
    """A cache file whose every recorded path sits in a vanished directory.

    An entry is still valid when its leaf's parent dir exists (the scan tree is
    reachable). A cache is dead when none of the dirs it caches exist any more
    (e.g. pytest tmp dirs that have been culled). Generic ancestors like /tmp
    never count, so this reliably prunes test artifacts.
    """
    if not data:
        return True
    for p in data:
        parent = Path(str(p)).parent
        if parent.exists():
            return False  # at least one scan dir is still reachable -> keep
    return True


def clean_cache(
    cache_dir: str | Path,
    *,
    dry_run: bool = False,
    all_: bool = False,
    older_than: float | None = None,
) -> tuple[list[tuple[Path, str]], int]:
    """Remove stale cache files.

    Returns ([(path, reason)], freed_bytes). `freed_bytes` is the total size of
    the removed files (reported even for a dry run).

    A file is stale if (any):
      - `all_` is set
      - `older_than` seconds: its mtime is older than the cutoff
      - default: every dir recorded in it no longer exists on disk
    Never removes in-place `.tmp` writers. Does not touch the filesystem for
    `dry_run` (still returns what would be removed).
    """
    cache_dir = expand(str(cache_dir))
    if not cache_dir.is_dir():
        return [], 0
    now = time.time()
    removed: list[tuple[Path, str]] = []
    freed = 0
    for f in sorted(cache_dir.glob("*.json")):
        if f.name.endswith(".json.tmp"):
            continue
        if all_:
            removed.append((f, "explicit"))
        elif older_than is not None:
            age = now - f.stat().st_mtime
            if age > older_than:
                removed.append((f, f"older than {older_than:.0f}s"))
        else:
            try:
                data = json.loads(f.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                # unreadable file is junk -> prune
                removed.append((f, "unreadable"))
            else:
                if _file_is_stale(data):
                    removed.append((f, "scan dir no longer exists"))
    if not dry_run:
        for f, _ in removed:
            try:
                freed += f.stat().st_size
                f.unlink()
            except OSError:
                pass
    else:
        freed = sum(f.stat().st_size for f, _ in removed)
    return removed, freed
