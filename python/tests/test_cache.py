"""Cache cleanup tests: stale-dir pruning, --all, age-based, dry-run."""

from __future__ import annotations

import json
import time
from pathlib import Path

from deconflict.cache import cache_key, clean_cache


def _write(engine_dir: Path, cache_dir: Path, key: str, files: list[str]) -> Path:
    cache_dir.mkdir(parents=True, exist_ok=True)
    data = {str(engine_dir / f): {"size": 1, "mtime": 1.0} for f in files}
    p = cache_dir / f"{key}.json"
    p.write_text(json.dumps(data), encoding="utf-8")
    return p


def test_keeps_live_dir(tmp_path):
    engine = tmp_path / "scan"
    engine.mkdir()
    (engine / "a.txt").write_text("x")
    _write(engine, tmp_path / "cache", "live", ["a.txt"])
    removed, _ = clean_cache(tmp_path / "cache")
    assert removed == []


def test_prunes_vanished_dir(tmp_path):
    engine = tmp_path / "scan"
    engine.mkdir()
    cache = tmp_path / "cache"
    _write(engine, cache, "gone", ["a.txt"])
    engine.rmdir()  # scan dir disappears
    removed, freed = clean_cache(cache)
    assert len(removed) == 1
    assert removed[0][0].name == "gone.json"
    assert freed > 0
    assert not (cache / "gone.json").exists()


def test_ignores_live_gone_and_tmp(tmp_path):
    live = tmp_path / "live"
    live.mkdir()
    (live / "a.txt").write_text("x")
    gone = tmp_path / "gone"
    gone.mkdir()
    cache = tmp_path / "cache"
    _write(live, cache, "live", ["a.txt"])
    _write(gone, cache, "gone", ["a.txt"])
    (cache / "pending.json.tmp").write_text("{}")
    gone.rmdir()
    removed, _ = clean_cache(cache)
    names = {f.name for f, _ in removed}
    assert names == {"gone.json"}  # live kept, .tmp never touched


def test_all_removes_everything(tmp_path):
    engine = tmp_path / "scan"
    engine.mkdir()
    (engine / "a.txt").write_text("x")
    cache = tmp_path / "cache"
    _write(engine, cache, "one", ["a.txt"])
    _write(engine, cache, "two", ["a.txt"])
    removed, _ = clean_cache(cache, all_=True)
    assert len(removed) == 2


def test_older_than(tmp_path):
    engine = tmp_path / "scan"
    engine.mkdir()
    (engine / "a.txt").write_text("x")
    cache = tmp_path / "cache"
    p = _write(engine, cache, "old", ["a.txt"])
    old = time.time() - 10_000
    import os

    os.utime(p, (old, old))
    assert clean_cache(cache, older_than=3600)[0] != []  # pruned
    assert clean_cache(cache, older_than=60) == ([], 0)  # not old enough -> nothing


def test_dry_run_reports_without_deleting(tmp_path):
    engine = tmp_path / "scan"
    engine.mkdir()
    cache = tmp_path / "cache"
    _write(engine, cache, "gone", ["a.txt"])
    engine.rmdir()
    removed, freed = clean_cache(cache, dry_run=True)
    assert len(removed) == 1
    assert freed > 0
    assert (cache / "gone.json").exists()  # untouched


def test_missing_dir_noop(tmp_path):
    assert clean_cache(tmp_path / "nope") == ([], 0)


def test_cache_key_stable_ordered(tmp_path):
    a, b = tmp_path / "a", tmp_path / "b"
    assert cache_key([a, b]) == cache_key([a, b])
    assert cache_key([a, b]) != cache_key([b, a])
