"""Engine tests: end-to-end scan→analyze→auto-apply on tmp copies (dry-run + real)."""

from __future__ import annotations

from deconflict.engine import Action, ActionKind, Engine, EngineConfig
from deconflict.tools import Tools


def _engine(sample_dir, tmp_path, **kw) -> Engine:
    cfg = EngineConfig(
        dirs=[sample_dir],
        backup_dir=str(tmp_path / "backup"),
        cache_dir=str(tmp_path / "cache"),
        **kw,
    )
    return Engine(cfg)


def _engine_with(sample_dir, tmp_path, tools: dict[str, str], **kw) -> Engine:
    cfg = EngineConfig(
        dirs=[sample_dir],
        backup_dir=str(tmp_path / "backup"),
        cache_dir=str(tmp_path / "cache"),
        **kw,
    )
    return Engine(cfg, tools=Tools(tools))


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


def test_supported_tools_per_kind(sample_dir, tmp_path):
    from deconflict.launchers import ToolType

    tools = {
        "editor": "/usr/bin/vi",
        "diff": "/usr/bin/meld",
        "audio_player": "/usr/bin/mpv",
        "mp3_editor": "/usr/bin/kid3",
        "exiftool": "/usr/bin/exiftool",
    }
    engine = _engine_with(sample_dir, tmp_path, tools)
    result = engine.scan()
    analyzed = engine.analyze(result)

    readme = next(ga for g, ga in analyzed if g.base and g.base.name == "Readme.md")
    assert engine.supported_tools(readme) == [ToolType.VIEW, ToolType.DIFF]

    mp3_name = "Fröhlicher Kreis - Track12 Scottish Circassian, Irish Washerwoman, My Old Man.mp3"
    mp3 = next(ga for g, ga in analyzed if g.base and g.base.name == mp3_name)
    assert set(engine.supported_tools(mp3)) == {
        ToolType.VIEW,
        ToolType.EDIT,
        ToolType.DIFF,
        ToolType.VIEW_META,
        ToolType.EDIT_META,
    }


def test_view_commands_text_opens_both(sample_dir, tmp_path):
    from deconflict.launchers import Launcher
    from deconflict.tools import Tools

    engine = _engine(sample_dir, tmp_path)
    result = engine.scan()
    g = next(g for g in result.groups if g.base and g.base.name == "Readme.md")
    launcher = Launcher(Tools({"editor": "/usr/bin/editor"}))
    cmds = launcher.view_commands(g, g.conflicts[0])
    assert cmds and len(cmds) == 1
    assert cmds[0][0] == "/usr/bin/editor"
    assert cmds[0][1:] == [str(g.conflicts[0]), str(g.base)]


def test_kind_tool_override_wins(sample_dir, tmp_path):
    from deconflict.launchers import Launcher
    from deconflict.tools import Tools

    launcher = Launcher(Tools({}), file_type_tools={"image": "eog"})
    assert launcher.kind_tool("image") == "eog"
    assert launcher.kind_tool("audio") == "audio_player"  # default unaffected
