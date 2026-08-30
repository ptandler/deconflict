# Plan: First Textual TUI for deconflict (v1)

**Date:** 2026-08-30 · **Status:** in progress — building v1

## Goal
A first interactive TUI for deconflict alongside the existing generic CLI. Reuses the
UI-free engine (`Engine`/`ConflictGroup`/`GroupAnalysis`/`Launcher`/`Resolver`); the TUI is a
second frontend that owns the interaction loop. Existing CLI (`deconflict`, `resolve`,
`scan`, `--auto`) stays untouched and fully testable.

## Decisions (confirmed + refined by user)
- **Framework**: Textual (Rich-native). Added as an **optional** dependency; bare CLI keeps
  working with a bare install (guard the textual import behind the `tui` subcommand).
- **Entrypoint**: new `deconflict tui` subcommand. Bare `deconflict` / `resolve` / `scan` /
  `--auto` unchanged (CI + scripting parity).
- **Image preview (kitty/uneixel)**: **deferred to v2**. Note: TGP protocol probe must run
  *before* the Textual app starts; use `textual-image` in a follow-up with a `p` toggle.
- **Main pane switchable** between Files table / inline Diff / Resolve-Log.
- **External diff/merge tool still launchable** from an action button for text/office/media.
- **Left pane marks groups**: resolved = green, skipped = gray (dim), unresolved = normal.
  Footer shows resolved / skipped / total counters.
- Status + resolve-log tracking: TUI-local for v1 (only promote into the engine if a GUI
  later wants it).

## Layout
```
Left (DataTable, row cursor)   │  Main pane (tab-switchable)
 group | pattern | #count      │    [Files] [Diff] [Log]
 ~ base (conflicted…           │    files table (metadata format,
 ✓ base                        │        # | file | size | mtime | metadata)
 ▒ base (skipped)              │    / char-level diff (shared renderable)
                               │    / resolver output history
                               │ ───────────────────────────────────
                               │ action buttons (mouse + hotkey)
                               │ (0)(1)(v)iew (d)iff (e)dit (m)etadata
                               │ (h)both (s)kip (q)uit (?)tools
Footer: resolved X | skipped Y | total Z | bindings
```

## Engine reuse (no logic duplication)
- `Engine.scan()`/`analyze()` → groups + `GroupAnalysis` (uses already-computed `meta_fields`).
- `engine.supported_tools()` / the CLI `_actions` matrix → which action buttons per kind.
- `engine.apply()` + `Resolver` for effects; `engine.launch_tool()` for external view/edit/diff/merge.
- **Main refactor**: extract cli.py's diff/metadata printers (`_print_text_diff`,
  `_print_office_diff`, `_print_metadata_diff`) into **UI-agnostic Rich renderables** (return
  `Table`/`Group` instead of `console.print`), shared by CLI + TUI. Behavior-identical; keep
  CliRunner diff tests green (this is the risk area).

## New modules
```
src/deconflict/tui/__init__.py
                   app.py      # compose, bindings, footer/header status
                   views.py    # GroupTable(left), FilesTable/DiffView/LogView (main tab)
                   actions.py  # buttons -> Action via engine kind matrix
```
cli.py: add `tui` subcommand (guard the textual import).

## Behavior
1. `deconflict tui` → scan (reuse cache reporting) → populate left list.
2. Select group (up/down / mouse) → files table in main pane.
3. `<tab>` / buttons switch Files | Diff | Log.
4. Action buttons (mouse + hotkey), kind-driven; resolve safely via Resolver.
5. After action: mark group green (resolved) or gray (skipped), log a line to Log view,
   auto-advance to next unresolved group. `q` aborts with "resolved X of Y (M skipped)".
6. External `(d)iff` still launches meld/soffice/exiftool for text/office/media.

## Testing (Textual Pilot, headless)
`tests/test_tui.py`: mount App with sample_dir; assert left populated; select → files table
rows; switch to Diff; action hotkey → status flips + Log line; quit → summary. Add `textual`
to dev extras. Existing suite stays green.

## Deferred (v2)
- `textual-image` in-terminal preview (`p` toggle), terminal protocol pre-probe.
- Command palette / fuzzy group search.
- `Enter` → detail / metadata overlay; in-TUI `(e)dit` / `(x)edit-meta`.
- Promote session-status + log tracking into the shared engine if a GUI wants it.

## TodoList
- [ ] Persist this plan
- [ ] Add `textual` optional dep + `deconflict tui` subcommand hook
- [ ] Refactor diff/metadata printers to shared Rich renderables
- [ ] Scaffold `tui/` package (app/views/actions)
- [ ] Left group pane + right files/diff/log panes + action buttons
- [ ] Pilot tests; existing suite green
- [ ] `mise run check` + manual verify
