"""TUI smoke tests using Textual Pilot (headless, no pytest-asyncio needed)."""

from __future__ import annotations

import asyncio
import shutil
from pathlib import Path

from deconflict.engine import Engine, EngineConfig

SAMPLE = Path(__file__).resolve().parent.parent / "sample files"


def _make_groups(tmp_path: Path):
    """Set up scan dir + engine + groups for TUI tests."""
    root = tmp_path / "scan"
    root.mkdir()
    for src in SAMPLE.iterdir():
        if src.is_file() and not src.name.startswith("."):
            shutil.copy2(src, root / src.name)
    cfg = EngineConfig(
        dirs=[root],
        backup_dir=str(tmp_path / "backup"),
        cache_dir=str(tmp_path / "cache"),
    )
    engine = Engine(cfg)
    result = engine.scan()
    groups = engine.analyze(result)
    return engine, groups


def test_mount_populates_group_table(tmp_path):
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            gt = app.query_one("#groups")
            assert len(gt.rows) == len(groups)
            assert app.current_index == 0
            assert len(app.action_map) > 0

    asyncio.run(_run())


def test_skip_advances_and_marks(tmp_path):
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("s")
            await pilot.pause()
            assert app.skipped == 1
            assert app.resolved == 0
            assert app.current_index == 1
            assert len(app.status) == 1

    asyncio.run(_run())


def test_quit_with_summary(tmp_path):
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("q")
            await pilot.pause()
            assert app.return_code == 0

    asyncio.run(_run())


def test_keep_base_resolves_and_advances(tmp_path):
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            # group 0 has a base, so hotkey "0" = keep base
            assert app.action_map  # sanity: action bar populated
            await pilot.press("0")
            await pilot.pause()
            assert app.resolved == 1
            assert app.skipped == 0
            assert app.current_index == 1

    asyncio.run(_run())


def test_tab_cycle(tmp_path):
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            tabs = app.query_one("#tabs")
            assert tabs.active == "tab-diff"
            await pilot.press("ctrl+t")
            await pilot.pause()
            assert tabs.active == "tab-files"
            await pilot.press("ctrl+t")
            await pilot.pause()
            assert tabs.active == "tab-log"
            await pilot.press("ctrl+t")
            await pilot.pause()
            assert tabs.active == "tab-diff"

    asyncio.run(_run())


def test_skip_all_then_quit(tmp_path):
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            for _ in range(len(groups)):
                await pilot.press("s")
                await pilot.pause()
            assert app.skipped == len(groups)
            # last group: no more to advance to
            assert app.current_index == len(groups) - 1
            await pilot.press("q")
            await pilot.pause()
            assert app.return_code == 0

    asyncio.run(_run())


def test_keep_copy_advances(tmp_path):
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            # group 0 has 1 copy, hotkey "1" = keep copy #1
            await pilot.press("1")
            await pilot.pause()
            assert app.resolved == 1
            assert app.current_index == 1

    asyncio.run(_run())


def test_arrow_keys_sync_selection_and_diff(tmp_path):
    """Up/down arrow navigation updates the current group + right pane immediately."""
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            assert app.current_index == 0
            # Press down arrow -> moves to group 1 without pressing Enter
            await pilot.press("down")
            await pilot.pause()
            assert app.current_index == 1, f"expected index 1, got {app.current_index}"
            # Right pane diff view should have been re-primed for the new group
            diff = app.query_one("#diff")
            static = diff.query_one("#diff-title")
            text = str(static.render())
            assert text.strip(), "diff title should be populated after arrow navigation"
            # Press up arrow -> back to group 0
            await pilot.press("up")
            await pilot.pause()
            assert app.current_index == 0, f"expected index 0, got {app.current_index}"

    asyncio.run(_run())
