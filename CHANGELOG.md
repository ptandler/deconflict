# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Nextcloud conflict filename regex now allows multiple spaces before `(conflicted copy ...)` (fixes FileNotFoundError when base filename has double space)
- `analyze_group()` gracefully handles missing base files (only conflict file exists) — no longer crashes with `FileNotFoundError`

### Added
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
