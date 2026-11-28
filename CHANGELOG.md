# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Experimental Rust library modules for cache, media metadata and external tool discovery.
- Rust CLI scan progress reuses the previous successful file-count estimate, labels discovery and conflict-analysis counts, and separates timings/counters from the current path on interactive terminals.
- Rust interactive resolve reports content differences, file metadata, analysis notes and recommendations before prompting for an action.
- Rust interactive resolve streams completed group analyses during scanning; `--wait-for-scan` and `--wait-for-analysis` offer buffered-start modes, with choices applied only after a complete scan.
- Rust analysis tables use terminal-aware color and emphasis, and include all available ExifTool audio/image metadata.
- Rust interactive reports show group positions and pending-work status, `(m)` reprints the report, and `(?)` maps each offered action to its actual command for every file kind.
- Rust groups with 3+ files render one wide metadata table when the terminal allows it, with wrapped file headers instead of one table per copy; 3- and 4-file sample groups were added under `sample_files/`.
- Experimental Rust TUI with a mouse-resizable group/detail split, metadata/files/diff/log tabs, action buttons and CLI-style status rows; available via `resolve --tui` and bare TTY invocation, with `--no-tui` forcing CLI fallback.
- Rust TUI PTY contracts cover dry-run rendering, buffered skip, real keep-base resolution, forced cancel, Ctrl-C exit, quit with no groups and non-TTY rejection.
- Rust TUI shares live counters, elapsed time and the final scan summary with the CLI, shows the conflict count in the title row, and offers mouse-selectable tabs.
- Rust TUI has a `(h)` keyboard-shortcuts tab, hides tabs with no content, and can show scaled image pairs side by side on kitty terminals.
- Rust TUI `files` tab details show content/size/modified relations for the selected file; PTY contracts cover `?` command listing and file relations.
- Rust TUI `Left`/`Right` switch tabs and are documented in `(h)`.
- Rust TUI `files` detail shows per-file analysis findings (newest, largest, additions-only/deletions-only, recommendation) and prompts to select a copy when `#0` is active.
- Rust TUI conflict list shows a `(Nx)` suffix for groups with more than two files.
- Rust TUI `(?)` shows the selected group's file type and only its file-type commands, instead of repeating keep/skip/config/quit actions already shown in the action box.

### Changed
- Rust TUI status now uses two rows: the first keeps timing, phase and decided/error counters; the second shows the current path only while work is pending and phase timings when done.
- Rust TUI starts on the `log` tab and switches to `meta` once the first conflict group arrives, without overriding later tab choices.
- Rust TUI `files` detail compares copies against `#0`, shows size deltas as `↑`/`↓` with a human-readable difference, and uses a darker selected-row background.
- Rust progress wording now uses `discovery` for enumeration and `conflicts` for analysis counts and final summaries in both CLI and TUI; live directory counts are reconciled with the final unique canonical count.
- Rust `scan` keeps a legacy `conflict group(s)` alias line for frozen executable contracts while using `conflicts` in the main summary.
- Rust quit actions are now `(q) quit` for immediate cancel and `(Q) quit when scan is done` while work is pending; `Q` is hidden once scanning finishes but remains available as a shortcut.
- Rust is now the main project at the repository root (version 0.3.0); the frozen Python reference and its fixtures moved under `python/`, with Rust-first CI and publish workflows and a rewritten README.
- README documents how final `discovery`/`analysis`/`waiting`/`total` timings are counted.

### Fixed
- Rust TUI `q` now keeps the CLI-style progress bar visible until scan and group analysis complete, unless progress is disabled.
- Rust TUI analysis counter now tracks rich group analysis results instead of provisional scanner copies; elapsed time stops at scan/analysis completion in both CLI and TUI.
- Rust CLI always reports scan errors to stderr, including with `--no-progress`; Rust TUI exposes scan and analysis errors in an `errors` tab.
- Rust interactive scan progress pauses while a prompt is open; wait messages are shown only while work remains.
- Rust scanning reports cancellation reliably and analysis detects disappearing base files.
- Rust metadata reports distinguish creation and modification times and display local timestamps.
- Rust scan summaries print immediately on completion and exclude time spent waiting for interactive input.
- Rust interactive analysis reuses scanner file snapshots, hashing each conflict copy once; stale snapshots are re-hashed.
- Rust TUI tab hit targets now match rendered padded tab widths, so clicking `diff` no longer selects `log`.
- Rust final scan timing reports unaccounted time as `waiting` instead of leaving a large gap between phase and total durations.
- Rust metadata diff table keeps its color cues in the TUI, and the `files` table uses the same colors for size and modification-time differences.
- Rust TUI `?` now always opens the command list, even before analysis or without a selected conflict.
- Rust TUI `files` table widens the size column so size-diff annotations are no longer truncated.
- Rust TUI selected file cells use a dark background with readable foreground instead of colored text on a bright selection background.
- Rust TUI built-in diff now uses the selected file in groups with more than two files; file-targeting actions use the same base-first selection order.
- Rust file tools now work on one selected file from the TUI `files` tab; CLI asks which file to view and offers `b` for both where supported (not for audio).
- Rust audio view plays one file at a time, prefers GUI audio players and suppresses GUI tool output leaking into the terminal.
- Rust images moved from an automatic overlay to an explicit `images` tab and scale within that pane instead of overlapping the UI.
- Rust TUI no longer prints per-file `moved:` lines into the normal terminal.
- Rust CLI and TUI map TUI Ctrl-C and Esc to interrupt.
- Rust progress no longer estimates an unreliable ETA; it shows elapsed time only.
- Rust TUI uses a stronger selected-tab highlight, starts on the log tab, drains queued input while scanning, and reserves a bottom hint row for `(h)`.
- Rust ExifTool parsing now reads its JSON output, preserving metadata keys and values instead of misreading colon-delimited dates as field names.
- Experimental Rust analysis detects some Unix file changes even when modification time is restored.
- Bare invocation (`deconflict [dirs]`) no longer crashes with `typer.exceptions.Exit: 0` when there is nothing to resolve — Typer's `Exit`/`Abort`/`BadParameter` are now translated into process exit codes
- `deconflict init-config` works from an installed wheel — `config.example.toml` ships as package data (`src/deconflict/config.example.toml`) instead of living in the repo root only
- "no directories" hint no longer loses `[scan]` to Rich markup parsing
- Docs: `deconflict init-config` (not `--init-config`) — wrong flag in README and both config templates
- Removed a duplicated `_from_dict()` in `config.py` (the dead second copy shadowed the first)
- Nextcloud conflict filename regex now allows multiple spaces before `(conflicted copy ...)` (fixes FileNotFoundError when base filename has double space)
- `analyze_group()` gracefully handles missing base files (only conflict file exists) — no longer crashes with `FileNotFoundError`

### Changed
- `config.example.toml` moved into the package (`src/deconflict/`) so it is installed with it

### Added
- Python executable compatibility contracts and reproducible migration benchmarks.
- Experimental read-only Rust scanning core with incremental scheduling, in-memory work queues, bounded worker and directory handles, recommendation diagnostics and benchmark comparison; migration remains incomplete.
- Test case for conflict group without base file

## [0.2.0]

### Added
- **TUI (Textual)**: Full interactive TUI with left group pane, right tabbed pane (Files/Diff/Log), action buttons with hotkeys, `Ctrl+T` tab cycling, mouse support
- **TUI Navigation**: `Ctrl+Up` / `Ctrl+Down` jump to next/previous undecided group (app-wide bindings)
- **Recommendation Engine**: `--auto recommended` resolves clear-winner groups (all identical → base; strictly newer + superset for text; strictly newer + larger for non-text), skips the rest with hint to rerun interactively
- **Setup Command**: `deconflict setup` analyzes local setup, recommends installs, prints config/backup/cache dirs, tool status with install hints
- **Metadata Diff Table**: Primary per-group view showing size/created/modified + per-kind fields; identical attr rows dimmed; newer dates bold cyan; larger sizes bold
- **ExifTool Enrichment**: Audio/image metadata via `exiftool -s -j` (bitrate, sample rate, channel mode, resolution, color components, IPTC/XMP), fallback to mutagen/PIL
- **Office Text Extraction**: `docProps/core.xml` (title/creator/lastModifiedBy/created/modified) + stripped text for docx/pptx/xlsx/ods (no raw XML)
- **External Office Text Converter**: Optional `odt2txt`/`pandoc` backend via `[tools].office_text` with zip fallback, cached
- **Char-Level Diff**: `SequenceMatcher` highlights changed character runs within changed lines for text/office
- **Config Tools Subcommand**: `deconflict config tools` shows per-kind tool mappings, found/missing status, install hints
- **Scan Progress Bar**: Live counter + current filename (`scanning (N files): <file>`), indeterminate bar with visible forward movement
- **Stage Timing Output**: `resolve` prints `scan X.Xs, analyze Y.Ys (total Z.Zs)` after analysis
- **Full Paths in Launch Output**: `_launch` prints entire argv with paths (space-containing args quoted)
- **Numbered Keep Actions**: Multiple copies → `(0) keep base`, `(1) keep copy #1`, etc.
- **Keep-Both Action**: Renames conflict copies in place to `<base> (copy N).ext` (non-destructive, dry-run safe)
- **View/Edit Collapse**: When VIEW and EDIT resolve to same exe (office/editor), menu shows only `(e)dit`; office `(v)open` renamed `(o)pen`
- **Metadata Always Available**: `(m)etadata` always offered (generic attrs guarantee content); embedded cover art (APIC/PIC) excluded from summary
- **Config Path in `(?)tools`**: Footer prints effective config path + `[tools]/[file_types]/[file_types_edit]` sections to edit
- **XDG Support**: `XDG_CONFIG_HOME`/`XDG_CACHE_HOME` honored for default config file and cache dir
- **Cache Invalidation**: Active cache invalidated after real resolve (not on `--dry-run`)
- **Cache Clean Command**: `deconflict cache clean [--dry-run|--all|--older-than-days]` prunes stale caches
- **Atomic Backup**: `os.replace` fast path (same filesystem), EXDEV fallback copy→verify→remove
- **Bare Mode Interactive**: `deconflict` (no args) → scan + resolve; auto-TUI when `textual` installed
- **Version Flag/Command**: `--version` flag + `version` command printing `deconflict {version}`
- **Patterns**: Nextcloud, pacman (`.pacnew/.pacsave/.pacorig`), Syncthing (`.sync-conflict-*`); custom regex via config
- **KeePass Merge Recipe**: `.kdbx` → `keepassxc-cli merge` (blocking, dry-run preview)
- **Decision Log**: JSONL in backup dir; cache JSON in cache dir

### Changed
- **Text Diff Default**: `(d)iff` is inline terminal diff (text/office); meld dropped from default menu (available via config)
- **Metadata Table Unified**: Single canonical `_content_fields` extractor feeds both overview verdict and `(m)etadata` table (consistency guaranteed)
- **Non-Blocking External Launches**: GUI/player launches detached (`Popen`, `start_new_session=True`); CLI/TUI returns to menu immediately; only keepass merge stays blocking
- **Edit Opens Both Files**: Text/office edit appends sibling path so both files open for comparison (fixes ODT/same-app)
- **Diff Tab Default**: Diff pane is now the first tab (default) in TUI
- **Recommendation Display**: Single `(Enter) keep #N 'file' (newest)` line (bold cyan); TUI has `(r)` action button
- **Recommendation Logic**: Rejects newer non-text copies that are strictly smaller (possible loss); strips blank lines for text superset check
- **Action Buttons Wrapped**: TUI action bar uses wrapped `Horizontal` rows (`.action-row`) inside bottom-docked `Vertical` with exact height; buttons keep 3-line natural height
- **Arrow Navigation Sync**: TUI `on_data_table_row_highlighted` re-primes right pane immediately (no Enter needed)
- **Config MarkupError Fixed**: Entire rendered line escaped (not just values) to handle `[`/`]` in paths
- **Office Diff**: Extracted-text terminal diff (soffice `--compare` removed after build error)

### Fixed
- **TUI Button IDs**: `?` hotkey invalid in Textual IDs → `_render_actions` uses sequential `act-{seq}` only
- **Config Help Test**: `RichVisual._renderable` rendered via `Console` for content assertions
- **Version Tests**: Dynamic `__version__` assertion (was hardcoded 0.1.0 vs actual 0.2.0)
- **Move-to-Backup ENAMETOOLONG**: Bounded `_unique_target` collision re-prefix
- **Metadata Verdict vs Diff Disagreement**: Unified `_content_fields` so overview and `(m)` rows can't disagree
- **Empty Base Superset**: `_text_superset` strips blank/whitespace lines before comparing

## [0.1.0] - 2026-08-30

### Added
- Initial release: interactive CLI for sync conflict resolution (Nextcloud, pacman, Syncthing)
- Layered architecture: UI-free `engine.py` + `cli.py` (Typer) + `launchers.py`
- Pattern-driven scanning with cache (size/mtime/sha/is_text)
- Media analysis: mutagen (audio), Pillow (images), ffprobe (video), zip-office text
- Interactive per-group menu with kind-aware actions (view/edit/diff/keep/skip/quit)
- `--auto {base|copy|newest|bigger}` non-interactive resolution
- Backup to `~/deconflict-backups/` (configurable), JSONL decision log
- `--dry-run` safety, `--config FILE` override, `init-config` command
- Windows support via pathlib + os.name + shutil.which + case-insensitive matching
