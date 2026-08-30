"""Resolver tests: safety, copy→verify→remove, dry-run, keep semantics."""

from __future__ import annotations

import pytest

from deconflict.patterns import build_patterns
from deconflict.resolve import Resolver
from deconflict.scan import scan

PATTERNS = build_patterns()


def _readme_group(sample_dir):
    result = scan([sample_dir], PATTERNS)
    return next(g for g in result.groups if g.base.name == "Readme.md")


def _resolver(tmp_path) -> Resolver:
    return Resolver(tmp_path / "backup", dry_run=False)


def test_move_to_backup_copies_then_removes(sample_dir, tmp_path):
    resolver = _resolver(tmp_path)
    target = sample_dir / "Readme (conflicted copy 2024-12-01 213224).md"
    original = target.read_bytes()
    res = resolver.move_to_backup(target, "Readme.md")
    assert res.action == "move"
    assert not target.exists()
    assert res.moved_to is not None and res.moved_to.exists()
    assert res.moved_to.read_bytes() == original


def test_move_to_backup_backup_is_outside_scan_root(sample_dir, tmp_path):
    resolver = _resolver(tmp_path)
    target = sample_dir / "Readme (conflicted copy 2024-12-01 213224).md"
    res = resolver.move_to_backup(target, "Readme.md")
    assert not str(res.moved_to).startswith(str(sample_dir))


def test_never_hard_deletes(tmp_path):
    src = tmp_path / "keep.txt"
    src.write_text("hello")
    resolver_ = _resolver(tmp_path)
    res = resolver_.move_to_backup(src, "keep")
    # The content survives in the backup; the original is gone but not overwritten.
    assert res.moved_to.read_text() == "hello"
    assert list(tmp_path.rglob("*"))  # backup still exists


def test_dry_run_touches_nothing(tmp_path):
    src = tmp_path / "x.txt"
    src.write_text("data")
    backup_root = tmp_path / "backup"
    resolver_ = Resolver(backup_root, dry_run=True)
    res = resolver_.move_to_backup(src, "x")
    assert res.detail == "(dry-run) would move"
    assert src.exists()
    assert not backup_root.exists()


def test_keep_base_moves_copies_only(sample_dir, tmp_path):
    resolver = _resolver(tmp_path)
    g = _readme_group(sample_dir)
    res = resolver.keep_base(g.key, g.base, g.conflicts, tmp_path / "log.jsonl")
    assert g.base.exists()
    assert not g.conflicts[0].exists()
    assert len(res) == 1
    assert (tmp_path / "log.jsonl").exists()


def test_keep_copy_backs_up_base_and_renames_copy(sample_dir, tmp_path):
    resolver = _resolver(tmp_path)
    g = _readme_group(sample_dir)
    copy = g.conflicts[0]
    original_base = g.base.read_bytes()
    copy_content = copy.read_bytes()
    others = [c for c in g.conflicts if c is not copy]
    res = resolver.keep_copy(g.key, g.base, copy, others, tmp_path / "log.jsonl")
    # The clean base path now holds the COPY's content (the chosen winner).
    assert g.base.exists()
    assert g.base.read_bytes() == copy_content
    # The original base content survives in the backup (never hard-deleted).
    backups = list((tmp_path / "backup").rglob("Readme.md"))
    assert backups and backups[0].read_bytes() == original_base
    assert not list(sample_dir.glob("* conflicted copy *"))
    assert len(res) >= 1


def test_backup_path_collision_dedups(tmp_path):
    src = tmp_path / "a.txt"
    src.write_text("1")
    resolver = _resolver(tmp_path)
    resolver.move_to_backup(src, "g")
    src2 = tmp_path / "again.txt"
    src2.write_text("2")
    res = resolver.move_to_backup(src2, "g")
    assert res.moved_to != (tmp_path / "backup" / "g" / "a.txt")


def test_long_name_collision_stays_within_name_limit(tmp_path):
    """Long filenames + collision must dedup without hitting ENAMETOOLONG."""
    long = "x" * 250
    src = tmp_path / f"{long}.txt"
    src.write_text("1")
    resolver = _resolver(tmp_path)
    resolver.move_to_backup(src, "g")
    # second collide onto the exact same name -> needs a dedup marker, stays < 255B
    src2 = tmp_path / f"{long}.txt"
    src2.write_text("2")
    res = resolver.move_to_backup(src2, "g")
    assert res.moved_to is not None
    name = res.moved_to.name
    assert len(name.encode("utf-8")) <= 255
    assert name != f"{long}.txt"


def test_refuses_to_move_backup_root_itself(tmp_path):
    root = tmp_path / "backup"
    root.mkdir()
    resolver = Resolver(root)
    with pytest.raises(ValueError):
        resolver.move_to_backup(root, "g")
