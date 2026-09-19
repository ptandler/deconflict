"""TUI smoke tests using Textual Pilot (headless, no pytest-asyncio needed)."""

from __future__ import annotations

import asyncio
import shutil
from pathlib import Path

from deconflict.engine import Engine, EngineConfig

SAMPLE = Path(__file__).resolve().parent.parent / "sample-files"


def _make_groups(tmp_path: Path):
    """Set up scan dir + engine + groups for TUI tests."""
    root = tmp_path / "scan"
    root.mkdir()
    for src in sorted(SAMPLE.rglob("*")):
        if src.is_file():
            rel = src.relative_to(SAMPLE)
            target = root / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, target)
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
            assert tabs.active == "tab-meta"
            await pilot.press("ctrl+t")
            await pilot.pause()
            assert tabs.active == "tab-files"
            await pilot.press("ctrl+t")
            await pilot.pause()
            assert tabs.active == "tab-diff"
            await pilot.press("ctrl+t")
            await pilot.pause()
            assert tabs.active == "tab-log"
            await pilot.press("ctrl+t")
            await pilot.pause()
            assert tabs.active == "tab-meta"

    asyncio.run(_run())


def test_ctrl_up_down_skips_decided_groups(tmp_path):
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            assert app.current_index == 0
            # ctrl+down -> next undecided group (1)
            await pilot.press("ctrl+down")
            await pilot.pause()
            assert app.current_index == 1
            # skip group 1; _advance returns to first undecided (0)
            await pilot.press("s")
            await pilot.pause()
            assert app.current_index == 0
            assert app.skipped == 1
            # ctrl+down must hop over skipped group 1 straight to 2
            await pilot.press("ctrl+down")
            await pilot.pause()
            assert app.current_index == 2
            # ctrl+up must hop back over skipped group 1 to 0
            await pilot.press("ctrl+up")
            await pilot.pause()
            assert app.current_index == 0
            # now skip 0 too; _advance lands on first undecided = 2
            await pilot.press("s")
            await pilot.pause()
            assert app.current_index == 2
            # ctrl+down from 2 -> 3, ctrl+up from 3 -> 2
            await pilot.press("ctrl+down")
            await pilot.pause()
            assert app.current_index == 3
            await pilot.press("ctrl+up")
            await pilot.pause()
            assert app.current_index == 2

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
            # Meta pane should have been re-primed for the new group
            meta = app.query_one("#meta")
            static = meta.query_one("#meta-title")
            text = str(static.render())
            assert text.strip(), "meta title should be populated after arrow navigation"
            # Press up arrow -> back to group 0
            await pilot.press("up")
            await pilot.pause()
            assert app.current_index == 0, f"expected index 0, got {app.current_index}"

    asyncio.run(_run())


def test_meta_tab_persists_after_diff(tmp_path):
    """The Meta tab keeps the metadata table when the Diff tab renders a text diff."""
    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            meta_title = str(app.query_one("#meta-title").render())
            # Render a text diff directly (avoids launching an external diff tool).
            group, ga = app.groups[app.current_index]
            target = ga.copies[0].path if ga.copies else ga.base.path
            app._render_diff("term_diff", group, target)
            await pilot.pause()
            # Diff tab activated, Meta tab content unchanged
            assert app.query_one("#tabs").active == "tab-diff"
            assert str(app.query_one("#meta-title").render()) == meta_title

    asyncio.run(_run())


def test_config_help_action_shows_tools(tmp_path):
    """The (?) config action renders the current kind's tool mapping."""
    import io

    from rich.console import Console

    engine, groups = _make_groups(tmp_path)
    from deconflict.tui.app import DeconflictApp

    async def _run():
        app = DeconflictApp(engine, groups)
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("?")
            await pilot.pause()
            assert app.query_one("#tabs").active == "tab-diff"
            title = str(app.query_one("#diff-title").render())
            assert "config" in title
            buf = io.StringIO()
            visual = app.query_one("#diff-body").render()
            renderable = getattr(visual, "_renderable", visual)
            Console(file=buf, width=120).print(renderable)
            assert "file kind" in buf.getvalue()

    asyncio.run(_run())
