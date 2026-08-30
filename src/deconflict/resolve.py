"""Resolution engine: safe moves to backup + decision log. NEVER hard-deletes."""

from __future__ import annotations

import errno
import json
import os
import shutil
import time
from dataclasses import dataclass
from pathlib import Path

from .paths import is_within, sanitize

ACTION_LABEL = "decision-log.jsonl"

# Most filesystems cap a name component at 255 bytes (UTF-8). Reserve headroom
# for a `_N` dedup counter so collision handling can't blow past the limit.
_MAX_NAME_BYTES = 240


def _unique_target(directory: Path, target: Path) -> Path:
    """A non-colliding target name under `directory`, bounded and name-safe.

    Instead of recursively re-prefixing (which grows the name without bound and
    hits ENAMETOOLONG), it appends `_1`, `_2`, ... and truncates the original
    stem to keep any single component within filesystem byte limits.
    """
    if not target.exists():
        return target
    suffix = target.suffix
    for n in range(1, 10_000):
        marker = f"_{n}"
        stem_budget = _MAX_NAME_BYTES - len((marker + suffix).encode("utf-8"))
        stem = target.stem.encode("utf-8")[: max(stem_budget, 1)].decode("utf-8", "ignore")
        candidate = directory / f"{stem}{marker}{suffix}"
        if not candidate.exists():
            return candidate
    raise RuntimeError(f"could not find a free backup name for {target.name}")


@dataclass
class Resolved:
    action: str
    moved_to: Path | None = None
    detail: str = ""


@dataclass
class Resolver:
    backup_root: Path
    dry_run: bool = False

    def __post_init__(self) -> None:
        if not self.dry_run:
            self.backup_root.mkdir(parents=True, exist_ok=True)

    def _ensure_safe(self, path: Path) -> None:
        if path.resolve() == self.backup_root.resolve():
            raise ValueError("refusing to move the backup root itself")
        if is_within(path, self.backup_root):
            raise ValueError(f"{path} is inside the backup dir; refusing to move")

    def move_to_backup(self, path: Path, group_key: str) -> Resolved:
        """Move `path` into the backup root, preserving its relative name.

        Same-filesystem moves use an atomic rename; cross-device moves (EXDEV)
        fall back to copy→verify→remove.
        """
        if not path.exists():
            return Resolved("missing", detail="already gone")
        self._ensure_safe(path)
        target_dir = self.backup_root / sanitize(group_key.split("|")[0])
        target = target_dir / sanitize(path.name)
        if self.dry_run:
            return Resolved("move", moved_to=target, detail="(dry-run) would move")
        target_dir.mkdir(parents=True, exist_ok=True)

        target = _unique_target(target_dir, target)
        try:
            os.replace(path, target)
        except OSError as e:
            if e.errno != errno.EXDEV:
                raise  # EACCES/EPERM/EBUSY: propagate, don't downgrade to copy
            shutil.copy2(path, target)
            with open(target, "rb") as a, open(path, "rb") as b:
                if a.read() != b.read():
                    target.unlink()
                    raise RuntimeError(f"backup verify failed for {path}") from None
            path.unlink()
        return Resolved("move", moved_to=target)

    def keep_base(
        self, group_key: str, base: Path, conflicts: list[Path], log: Path
    ) -> list[Resolved]:
        out = [self.move_to_backup(c, group_key) for c in conflicts]
        self._log(log, group_key, "keep_base", base=base, decisions=out)
        return out

    def keep_copy(
        self,
        group_key: str,
        base: Path,
        copy: Path,
        other_conflicts: list[Path],
        log: Path,
    ) -> list[Resolved]:
        """Losers = base + non-chosen copies (backed up), then copy renamed to base name."""
        losers: list[Path] = []
        if base is not None:
            losers.append(base)
        losers += [c for c in other_conflicts if c.resolve() != copy.resolve()]
        out: list[Resolved] = []
        seen: set[str] = set()
        for loser in losers:
            if str(loser.resolve()) in seen:
                continue
            seen.add(str(loser.resolve()))
            if loser.resolve() == copy.resolve():
                continue
            out.append(self.move_to_backup(loser, group_key))
        renamed: Resolved | None = None
        if base is not None:
            renamed = self._rename_to_base(copy, base, group_key)
            out.append(renamed)
        self._log(log, group_key, "keep_copy", base=base, copy=copy, decisions=out, renamed=renamed)
        return out

    def _rename_to_base(self, copy: Path, base: Path, group_key: str) -> Resolved:
        if self.dry_run:
            return Resolved("rename", moved_to=base, detail="(dry-run) would rename")
        if base.exists():
            raise RuntimeError(f"cannot rename {copy} -> existing {base}; move base first")
        base.parent.mkdir(parents=True, exist_ok=True)
        copy.rename(base)
        return Resolved("rename", moved_to=base)

    def keep_both(
        self,
        group_key: str,
        base: Path | None,
        conflicts: list[Path],
        log: Path,
    ) -> list[Resolved]:
        """Keep the base AND every conflict copy by renaming each copy to a plain,
        non-conflict name beside the base (so it stops matching the pattern).

        This is non-destructive: nothing is moved to backup and the base is never
        touched. Copied contents remain next to the original under a disambiguated
        name, e.g. `report (copy 1).docx`.
        """
        if base is None:
            return []
        out: list[Resolved] = []
        for i, c in enumerate(conflicts, 1):
            target = base.with_name(f"{base.stem} (copy {i}){base.suffix}")
            if c.resolve() == target.resolve():
                continue
            if self.dry_run:
                out.append(Resolved("rename", moved_to=target, detail="(dry-run) would rename"))
                continue
            target = _unique_target(base.parent, target)
            c.rename(target)
            out.append(Resolved("rename", moved_to=target))
        self._log(log, group_key, "keep_both", base=base, decisions=out)
        return out

    def _log(self, log: Path, key: str, action: str, **kw) -> None:
        if self.dry_run:
            return
        log.parent.mkdir(parents=True, exist_ok=True)
        entry: dict = {"ts": time.time(), "group": key, "action": action}
        entry.update({k: _jsonable(v) for k, v in kw.items()})
        with open(log, "a", encoding="utf-8") as f:
            f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def _jsonable(value):
    """Coerce Resolved / Path / lists into JSON-serializable primitives."""
    if isinstance(value, Resolved):
        return {
            "action": value.action,
            "moved_to": str(value.moved_to) if value.moved_to else None,
            "detail": value.detail,
        }
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, (list, tuple)):
        return [_jsonable(v) for v in value]
    return value
