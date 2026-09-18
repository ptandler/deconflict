# Plan: First Textual TUI for deconflict (v1)

**Date:** 2026-08-30 · **Status:** v1 COMPLETE — 103 tests green (96 existing + 7 TUI Pilot)

## Goal
A first interactive TUI for deconflict alongside the existing generic CLI. Reuses the
UI-free engine (`Engine`/`ConflictGroup`/`GroupAnalysis`/`Launcher`/`Resolver`); the TUI is a
second frontend that owns the interaction loop. Existing CLI (`deconflict`, `resolve`,
`scan`, `--auto`) stays untouched and fully testable.

## Decisions (confirmed + refined by user)
- **Framework**: Textual (Rich-native). Added as an **optional** dependency; bare install keeps working
  (textual import guarded behind `_tui_available()`).
- **Entrypoint**: `--tui/--no-tui` flags (v2 change, replaces the v1 `tui` subcommand). Bare `deconflict`
  auto-selects TUI when installed, CLI otherwise; `resolve` defaults to CLI; `--auto` + `--tui` = error.
- **Image preview (kitty/uneixel)**: **deferred to v2**. Note: TGP protocol probe must run
  *before* the Textual app starts; use `textual-image` in a follow-up with a `p` toggle.
- **Main pane switchable** between Files table / inline Diff / Resolve-Log. `Ctrl+T` cycles
  (Tab is consumed by Textual focus management).
- **External diff/merge tool still launchable** from an action button for text/office/media.
- **Left pane marks groups**: resolved = green (`✓`), skipped = gray dim (`▸`), unresolved = blank.
  Sub-title shows resolved / skipped / total counters.
- Status + resolve-log tracking: TUI-local for v1 (only promote into the engine if a GUI
  later wants it).

## Layout (implemented)
```
Left (DataTable, row cursor)   │  Main pane (tab-switchable)
 marker | base | pattern | #   │    [Files] [Diff] [Log]     Ctrl+T cycles
 ✓ base                        │    files table (metadata format,
 ▸ base (skipped)              │        # | file | size | mtime | metadata)
                               │    / metadata diff table
                               │    / char-level diff / office diff
                               │ ───────────────────────────────────
                               │ action buttons (mouse + hotkey)
                               │ (0)keep-base (1)keep-copy (h)both
                               │ (s)kip (v)iew (d)iff (e)dit (m)etadata ...
                               │ (q)uit
Header: deconflict | group 2/6 resolved 1 · skipped 1
```

## Engine reuse (no logic duplication)
- `Engine.scan()`/`analyze()` → groups + `GroupAnalysis` (uses already-computed `meta_fields`).
- Shared `actions.actions_for(engine, ga)` → ordered `(hotkey, label, ToolType | inline-tag)` per kind.
  Both CLI menu and TUI buttons use it — identical per-kind action lists.
- `engine.apply()` + `Resolver` for effects; `engine.launch_tool()` for external view/edit/diff/merge.
- `render.py` extractors (`metadata_diff_table`, `text_diff_lines`, `office_diff_lines`) shared by CLI + TUI.

## Modules
```
src/deconflict/actions.py        # SHARED: actions_for(engine, ga) → [(hotkey, label, ToolType|tag)]
src/deconflict/render.py         # SHARED: Rich renderable builders (table, diff, metadata)
src/deconflict/tui/__init__.py   # exports DeconflictApp
src/deconflict/tui/app.py        # DeconflictApp: compose, bindings, dispatch, status tracking
src/deconflict/tui/views.py      # GroupTable, FilesTable, DiffView, LogView widgets
src/deconflict/tui/actions.py    # ActionEntry model + build_actions()
tests/test_tui.py                # 7 Pilot headless tests
```
cli.py: `--tui/--no-tui` on `resolve` + bare-mode auto-TUI (`_tui_available`/`_launch_tui`).

## Implemented behaviors
1. `deconflict resolve --tui [--dirs] [--config] [--dry-run]` / bare `deconflict` → scan → populate left list.
2. Select group (arrow keys / mouse click) → files table in main pane.
3. `Ctrl+T` / mouse buttons cycle Files | Diff | Log tabs.
4. Action buttons (mouse + hotkey); kind-driven per `actions_for()`.
5. After action: mark group ✓ (resolved) or ▸ (skipped), log a line to Log view,
   auto-advance to next unresolved group. `q` prints summary and exits.
6. External diff/view/edit launchable (threaded, non-blocking).
7. Metadata diff table rendered on group selection (Diff tab default).
8. `--dry-run` passthrough: engine/Resolver won't touch the filesystem.

## Testing
`tests/test_tui.py`: 7 Pilot headless tests using `asyncio.run()` (no pytest-asyncio needed):
1. Mount: group table populated, action bar rendered
2. Skip: status flips to "skipped", advances to next group
3. Quit: exits cleanly with summary
4. Keep-base: resolves, advances
5. Tab cycle: Ctrl+T cycles Files → Diff → Log → Files
6. Skip-all-then-quit: all skipped, clean exit
7. Keep-copy: resolves, advances

Existing 96 tests unchanged; `mise run check` (ruff check + format-check + pytest) green.

## Deferred (v2)
- `textual-image` in-terminal preview (`p` toggle), terminal protocol pre-probe.
- Command palette / fuzzy group search.
- `Enter` → detail / metadata overlay; in-TUI `(e)dit` / `(x)edit-meta`.
- Promote session-status + log tracking into the shared engine if a GUI wants it.
- Rich metadata table rendering in Diff tab (currently basic; reuse `_meta_diff_columns` logic).

## TodoList
- [x] Persist this plan
- [x] Add `textual` optional dep + `--tui/--no-tui` entrypoint hook (v2 replaced the `tui` subcommand)
- [x] Refactor diff/metadata printers to shared Rich renderables (`render.py`)
- [x] Extract `actions_for()` into shared `actions.py` module
- [x] Scaffold `tui/` package (app/views/actions)
- [x] Left group pane + right files/diff/log panes + action buttons
- [x] Fix widget `__init__` id-accepting signatures (GroupTable, FilesTable, DiffView)
- [x] Fix DataTable `update_cell` to use string column keys, not int indices
- [x] Fix action bar duplicate-ID race via unique sequential button IDs
- [x] Pilot tests; existing suite green (103 total)
- [x] `mise run check` + manual verify

## Session 2026-09-17 — navigation + diff tab rework
- [x] **Ctrl+Up / Ctrl+Down jump to next/previous undecided group** — new bindings
  `("ctrl+up","prev_group")`, `("ctrl+down","next_group")`; `action_next_group()`/`action_prev_group()`
  skip groups whose key is in `self.status` (resolved or skipped). Textual action bindings are
  app-wide, so they work regardless of which tab/widget has focus. `_select_index` moves the
  row cursor. Test `test_ctrl_up_down_skips_decided_groups` added.
- [x] **Fix button ID generation — `?` hotkey invalid in Textual IDs** — `_render_actions`
  now uses `act-{seq}` only (dropping hotkey suffix); widget IDs are valid identifiers.
  Test `test_action_bar_hotkeys` passes.
- [ ] **Meta-diff must stay visible** — the "(d)iff" action currently overwrites the Diff tab's
  initial metadata table with the plain text diff. Plan: give the meta-diff table its own
  persistent tab (e.g. rename tabs to Meta/Files/Diff/Log), so diff output never clobbers it.
  Update `_show_default_diff`/`_render_diff` + cycle order + tests.
- [ ] **Text diff via meld when available** — `(d)iff` for text should prefer external meld
  (shared `launchers._diff_cmd`), inline terminal diff as fallback. TUI launches meld for
  current group.
- [ ] **"(?) config" help button** — show tools configured for the CURRENT group kind:
  [tools]/[file_types] mapping, found/missing, config path, install hints. Reuse shared
  `_print_kind_tools`/INSTALL_HINTS logic in a renderable form in the Log/tab pane.
- [ ] **Terminal editor vs TUI conflict (e.g. `fresh`)** — suspending the Textual app around a
  blocking editor run. Investigate Textual's app suspension (stop the app → run editor in a
  fresh terminal/screen area → resume), or detect TUI editors and suggest a GUI alternative.

## Session 2026-09-18 — test fixes + unit coverage
- [x] **Fix config help action test** — `RichVisual._renderable` rendered via `Console`
  for content assertions instead of relying on `.title` attr (test `test_config_help_action_shows_tools`).
- [x] **Fix pre-existing version test failures** — tests asserted hardcoded `0.1.0` vs actual
  `0.2.0`; updated to assert against dynamic `__version__` import.
- [x] **Added 4 launcher/action unit tests** in `test_engine.py`:
  `test_launcher_graphical_and_terminal_helpers`,
  `test_text_diff_prefers_graphical_diff`,
  `test_text_diff_falls_back_to_terminal`,
  `test_launcher_run_forces_blocking`.
- All 140 tests pass; `mise run check` green (lint + format-check + pytest).
