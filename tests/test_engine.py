"""Engine tests: end-to-end scan→analyze→auto-apply on tmp copies (dry-run + real)."""

from __future__ import annotations

from deconflict.engine import Action, ActionKind, Engine, EngineConfig


def _engine(sample_dir, tmp_path, **kw) -> Engine:
    cfg = EngineConfig(
        dirs=[sample_dir],
        backup_dir=str(tmp_path / "backup"),
        cache_dir=str(tmp_path / "cache"),
        **kw,
    )
    return Engine(cfg)


def test_scan_returns_groups(sample_dir, tmp_path):
    engine = _engine(sample_dir, tmp_path)
    result = engine.scan()
    assert len(result.groups) == 6


def test_auto_newest_dry_run_touches_nothing(sample_dir, tmp_path):
    engine = _engine(sample_dir, tmp_path, dry_run=True)
    result = engine.scan()
    groups = engine.analyze(result)
    for g, ga in groups:
        action = engine.auto_choice(ga, "newest")
        engine.apply(g, ga, action)
    # No backup dir should have been created during dry-run.
    assert not (tmp_path / "backup").exists()


def test_auto_base_resolves_readme(sample_dir, tmp_path):
    engine = _engine(sample_dir, tmp_path)
    result = engine.scan()
    for g, ga in engine.analyze(result):
        if g.base and g.base.name == "Readme.md":
            action = engine.auto_choice(ga, "base")
            assert action.kind is ActionKind.KEEP_BASE
            return
    raise AssertionError("Readme.md group not found")


def test_auto_copy_keeps_conflict(sample_dir, tmp_path):
    engine = _engine(sample_dir, tmp_path)
    result = engine.scan()
    for _group, ga in engine.analyze(result):
        action = engine.auto_choice(ga, "copy")
        assert action.kind is ActionKind.KEEP_COPY or action.kind is ActionKind.SKIP


def test_factory_actions():
    assert Action.keep_base().kind is ActionKind.KEEP_BASE
    assert Action.keep_both().kind is ActionKind.KEEP_BOTH
    from pathlib import Path

    a = Action.keep_copy(Path("/x"))
    assert a.target == Path("/x")
    assert Action.skip().kind is ActionKind.SKIP
