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
    # (m)etadata is always offered (generic file attrs), even for text files
    assert engine.supported_tools(readme) == [ToolType.VIEW, ToolType.DIFF, ToolType.VIEW_META]

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


def test_view_edit_collapse_when_same_tool(sample_dir, tmp_path):
    """When VIEW and EDIT resolve to the same executable (e.g. LibreOffice), the
    CLI collapses them to a single action (item: view/edit identical)."""
    from deconflict.launchers import Launcher
    from deconflict.tools import Tools

    same = Launcher(Tools({"office": "/usr/bin/soffice", "editor": "/usr/bin/editor"}))
    assert same.view_edit_collapse("office")  # office is both view and edit
    assert same.view_edit_collapse("text")  # editor is both view and edit

    diff = Launcher(Tools({"audio_player": "/usr/bin/mpv", "mp3_editor": "/usr/bin/kid3"}))
    assert not diff.view_edit_collapse("audio")  # player vs tag-editor differ


def test_edit_command_office_opens_both(sample_dir, tmp_path):
    """Edit for office (same app as view) opens BOTH files, not just the copy
    (G4 ODT bug: edit must let the user compare, not open only one file)."""
    from deconflict.launchers import Launcher, ToolType
    from deconflict.tools import Tools

    engine = _engine(sample_dir, tmp_path)
    result = engine.scan()
    g = next(g for g in result.groups if g.base and g.base.name.endswith(".ods"))
    launcher = Launcher(Tools({"office": "/usr/bin/soffice"}))
    cmd = launcher.command(g, ToolType.EDIT, g.conflicts[0])
    assert cmd and cmd[0] == "/usr/bin/soffice"
    assert str(g.conflicts[0]) in cmd and str(g.base) in cmd


def test_launch_prints_full_command_paths(sample_dir, tmp_path, capsys, monkeypatch):
    """Loading a tool prints the FULL command incl. file paths (not just the exe)."""
    import deconflict.launchers as launchers_mod

    recorded: list[list[str]] = []
    monkeypatch.setattr(launchers_mod.subprocess, "Popen", lambda cmd, **kw: recorded.append(cmd))
    launchers_mod._launch(["/usr/bin/editor", str(tmp_path / "a file.txt")])
    out = capsys.readouterr().out
    assert "launching:" in out
    assert "/usr/bin/editor" in out
    assert "a file.txt" in out
    assert recorded == [["/usr/bin/editor", str(tmp_path / "a file.txt")]]


def test_launch_blocking_uses_run(sample_dir, tmp_path, capsys, monkeypatch):
    """Blocking launches (e.g. keepass merge) use subprocess.run, not detach."""
    import deconflict.launchers as launchers_mod

    calls: list[tuple] = []
    monkeypatch.setattr(
        launchers_mod.subprocess, "Popen", lambda cmd, **kw: calls.append(("popen", cmd))
    )
    monkeypatch.setattr(
        launchers_mod.subprocess, "run", lambda cmd, **kw: calls.append(("run", cmd))
    )
    launchers_mod._launch(["some-cli"], block=True)
    assert calls == [("run", ["some-cli"])]


def test_launcher_graphical_and_terminal_helpers():
    """meld/WinMerge are graphical; text/other editors need the terminal."""
    from deconflict.launchers import Launcher, ToolType

    la = Launcher(Tools({"diff": "/usr/bin/meld", "editor": "/usr/bin/fresh"}))
    assert la.graphical_diff("text") is True
    assert la.graphical_diff("other") is True
    assert la.graphical_diff("office") is False
    assert la.runs_in_terminal(ToolType.EDIT, "text") is True
    assert la.runs_in_terminal(ToolType.VIEW, "other") is True
    assert la.runs_in_terminal(ToolType.VIEW, "office") is False
    assert la.runs_in_terminal(ToolType.DIFF, "text") is False
    plain = Launcher(Tools({"diff": "/usr/bin/diff"}))
    assert plain.graphical_diff("text") is False


def test_text_diff_prefers_graphical_diff(sample_dir, tmp_path):
    """actions_for uses the external GUI diff for text when one is available."""
    from deconflict.actions import actions_for
    from deconflict.launchers import ToolType

    engine = _engine_with(sample_dir, tmp_path, {"diff": "/usr/bin/meld"})
    groups = engine.analyze(engine.scan())
    _g, ga = next((g, a) for g, a in groups if a.kind == "text")
    by_key = {k: t for k, _l, t in actions_for(engine, ga)}
    assert by_key["d"] is ToolType.DIFF


def test_text_diff_falls_back_to_terminal(sample_dir, tmp_path):
    """Without a GUI diff tool, text diff stays the inline terminal diff."""
    from deconflict.actions import actions_for

    # A non-graphical diff tool (or none) keeps the inline terminal diff.
    engine = _engine_with(sample_dir, tmp_path, {"diff": "/usr/bin/diff"})
    groups = engine.analyze(engine.scan())
    _g, ga = next((g, a) for g, a in groups if a.kind == "text")
    by_key = {k: t for k, _l, t in actions_for(engine, ga)}
    assert by_key["d"] == "term_diff"


def test_launcher_run_forces_blocking(sample_dir, tmp_path, monkeypatch):
    """run(blocking=True) foregrounds a launch (terminal editor under suspend)."""
    import deconflict.launchers as launchers_mod
    from deconflict.launchers import ToolType

    calls: list[bool] = []
    monkeypatch.setattr(launchers_mod, "_launch", lambda cmd, block=False: calls.append(block))
    engine = _engine_with(sample_dir, tmp_path, {"editor": "/usr/bin/fresh"})
    groups = engine.analyze(engine.scan())
    g, ga = next((g, a) for g, a in groups if a.kind == "text")
    target = ga.copies[0].path if ga.copies else ga.base.path
    engine.launcher().run(g, ga, ToolType.EDIT, target, blocking=True)
    assert calls == [True]
