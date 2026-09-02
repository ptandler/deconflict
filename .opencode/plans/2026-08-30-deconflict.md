# Plan: deconflict — interactive sync-conflict resolver

**Date:** 2026-08-30 · **Status:** all 16 todo items done — `mise run check` green (87 tests)

## Goal
Interactive CLI to find and resolve "conflicted copy"-style sync conflict files:
Nextcloud `(conflicted copy YYYY-MM-DD HHMMSS)`, pacman `.pacnew/.pacsave/.pacorig`,
Syncthing `.sync-conflict-*`. Generic, pattern-driven, future TUI/GUI-ready.

## Decisions (agreed)
- Name: **deconflict** (PyPI + command + module). Package runs from any dir; repo dir will be renamed (rename happens on restart).
- Stack: Python >=3.11 (dev 3.13) · Typer (CLI) · Rich (colors/progress/live) · ruff (lint+format) · pytest · hatchling · uv + mise · GitHub Actions (MIT).
- Logic UI-free in `engine.py` → Textual TUI via bare `deconflict` (auto, if `textual` installed) or `resolve --tui`; PySide GUI would reuse it too.
- Patterns shipped: `nextcloud` (on, disable-able), `pacman`, `syncthing` (on). `dpkg`, `rpm`, `windows-copy` shipped disabled. Custom regex patterns in config.
- Disposal: NEVER hard-delete. Losers **moved** to backup dir OUTSIDE scan roots (`~/deconflict-backups/...`), configurable. `--dry-run` must not touch FS. JSONL decision log.
- KeePass: `.kdbx` → recipe launching `keepassxc-cli merge` (dry-run preview → real merge → discard copy). Verified: keepassxc-cli 2.7.12 `merge database database2`.
- Windows support via `pathlib` + `os.name` branches + `shutil.which` per-OS tool defaults + case-insensitive matching on NT.

## Confirmed architecture (src layout + layered engine; decided 2026-08-30)

`src/deconflict/` is the standard **src layout**: `src/` is the source-layout root,
`src/deconflict/` the one importable package (hatchling `packages=["src/deconflict"]`).
It prevents importing an uninstalled local copy. Not duplication.

Layers (single direction of dependency; engine is UI-free, reusable by TUI/GUI):
```
Frontends  cli.py (thin subcommands) + no-args -> interactive driver
Engine     engine.py: Action model, Context, apply(), auto_choice(), scan/analyze
           launchers.py: run external tools (meld/editor/viewer/office/keepass)
Infra      paths.py tools.py config.py(TOML) patterns.py scan.py analyze.py
           media.py cache.py resolve.py(Resolver + JSONL log)
```
Command model (mirrors git/gh/uv):
- `deconflict scan` — explicit, single-purpose, scriptable, exits
- `deconflict resolve --auto <rule>` — explicit, non-interactive, exits
- `deconflict resolve` — interactive CLI driver (scan + resolve)
- `deconflict resolve --tui` — interactive TUI driver (`--no-tui` forces CLI; `--auto` + `--tui` is an error)
- `deconflict` (bare) — interactive driver (**TUI when `textual` installed, else CLI**), internally calls scan + resolve
- `deconflict patterns | init-config`
- `deconflict config [--path FILE]` — print the effective config (like `mise config`)
- `--config FILE` option (on scan/resolve + bare) to point at an alternate config file

Domain `Action` type (the vocabulary; replaces raw strings):
- `KEEP_BASE`, `KEEP_COPY(target)`, `KEEP_BOTH`, `SKIP` — final decisions → `Engine.apply()`
- `TOOL(tool)` — run external tool then **re-prompt** for the final decision.
  Tool choice is coupled to file type: meld/editor(text) · viewer(images/video) ·
  office(ods/xlsx/docx) · keepass-merge(kdbx). Frontends own the loop; engine owns effects.

## Tooling / dev env
- `mise.toml`: tools python 3.13, uv latest; tasks in `[tasks.<name>]`.
- CI: `jdx/mise-action` + `mise run check`; matrix 3 OS × py 3.11–3.13.
- Editor/lint: ruff (`check` + `format`), teller of truth in `pyproject.toml`.
- External tools detected at runtime (never hard-required): meld/WinMerge (diff), $EDITOR, xdg-open/open (viewer), ffprobe (video), keepassxc-cli (kdbx), soffice (office), Kid3/Picard (mp3 editor hints).

## Project structure
```
.opencode/plans/            # this plan
AGENTS.md                   # points to this plan; agent conventions
mise.toml  pyproject.toml  uv.lock  .gitignore  LICENSE(MIT)  README.md
config.example.toml
.github/workflows/ci.yml
sample files/               # 12 fixtures (md/mp3/jpg/ods/xlsx/docx conflicts) — committed, never mutated
src/deconflict/
  cli.py engine.py scan.py patterns.py analyze.py media.py resolve.py
  cache.py tools.py paths.py launchers.py config.py
tests/                      # pytest; fixtures copied to tmpdir
```

## Behavior
1. **Scan**: recursive over configured dirs (config `[scan].default_dirs`, CLI `--dir`, `--pattern`), Rich progress bars re-writing output.
2. **Cache**: JSON file-cache (path → size/mtime/sha/is_text) keyed by dir set; on restart renders cached results instantly, background rescan merges; `--no-cache`, `--refresh`.
3. **Analyze** per group: size, hash, mtime (FS + name-stamp shown), content-equality, newer-by-mtime, text/binary sniff.
4. **Media diff**: mutagen (audio), Pillow (images), ffprobe (video), exiftool fallback; zip-office (ods/xlsx/docx) text via stdlib zipfile; `.kdbx` → keepass recipe.
5. **Interactive** per group: summary table (size, vs-base arrow, metadata diff per copy) + kind-aware numbered menu — keep base / keep copy / **view both files in suggested app** / terminal diff (text) / meld / $EDITOR / keep-both / skip / **quit** / **(?) tools & install hints**. Menu shows only actions supported for the file type. `--auto {base|copy|newest|bigger}` non-interactive.
6. **Resolve**: losers → backup (atomic rename when same filesystem; copy→verify→remove only on cross-device EXDEV), rename copy→base when keeping copy; per-action confirm; JSONL decisions + end summary.

## Config (`--init-config` writes; TOML)
```toml
[scan]         default_dirs=[], enabled_patterns=["nextcloud","pacman","syncthing"]
[general]      backup_dir="~/deconflict-backups", cache_dir
[tools]        diff, editor, image_viewer, office, keepass, mp3_editor  # per-OS defaults
[[scan.patterns]]  name, regex   # custom patterns
```

## README (concise)
Install (`uv tool install .` / pipx) · usage examples · patterns table · config reference · recommended external apps to install for editing/diffing · Windows note · MIT.

## Test fixtures recap (validated)
- mp3 pair: real tag-only diff (title, tag encoding) → proves metadata diff value.
- jpg pair: no EXIF → dims-only fallback + open viewer.
- md/ods/xlsx/docx pairs: content diff; office via zip text extraction.

## TodoList
- [x] Write plan to `.opencode/plans/` + create AGENTS.md
- [x] (user) rename project dir → deconflict; restart opencode
- [x] git init + scaffold: mise.toml, pyproject.toml, .gitignore, LICENSE, config.example.toml, CI, uv.lock
- [x] README.md (draft)
- [x] Restructure to layered engine: add engine.py + launchers.py + config.py; strip resolve.py; thin cli.py; delete core.py
- [x] Implement/finish modules (paths, patterns, scan, analyze, media, resolve, tools, cache)
- [x] Tests (patterns, scan, analyze, media, resolve, engine, cli) on tmpdir copies of sample files — 44 passing
- [x] uv sync; `mise run check` green (lint + format-check + pytest); verify on `sample files/`
- [x] Backup dir renamed `nc-conflict-backups` → `deconflict-backups` everywhere
- [x] mise task `devenv` = `uv tool install --editable .` (dev testing); `install` kept for non-editable
- [x] CLI polish: add `--config FILE` option + `config` print command
- [x] Config: warn loudly on TOML parse error instead of silent default fallback; fix `config.example.toml` `r"..."` raw-string (invalid TOML) bug
- [x] Cache: `deconflict cache clean [--dry-run|--all|--older-than-days]` prunes stale caches (dead scan dirs / age / explicit); `clean_cache()` in cache.py
- [x] Cache: invalidate active cache after a real resolve (`_invalidate_cache`), never on `--dry-run`
- [x] Cache: `pytest_sessionfinish` hook in tests/conftest.py prunes the shared default cache of dead-dir entries after the run
- [x] Fix pre-existing `move_to_backup` ENAMETOOLONG bug (unbounded collision re-prefix) → bounded, name-capped `_unique_target`
- [x] Tests for all of the above (cache cleanup, CLI cache clean, resolve invalidation, long-name collision) — 63 passing
- [x] Fix bare `deconflict` (no args) → interactive resolve
  - Root cause: bare mode had no config signal — with no config file there are no `default_dirs`, so it correctly reported "no directories" (exit 2) but without guidance. `invoke_without_command=True` was already in place.
  - CliRunner tests prove bare mode runs the interactive loop (config with dirs + skip), (q)uit aborts cleanly, and the no-config case now prints `init-config` guidance (exit 2).
- [x] bare `deconflict /path/` → scan + resolve + TUI (when textual installed) or CLI loop
  - `main()` intercepts argv where the first non-option token is not a known subcommand, routes to `_bare_resolve(config, dirs)` — bypassing Typer's subcommand parser which would misread the path as a command name. pyproject entry point changed from `app` to `main`. 111 tests green.
- [x] remove `tui` subcommand → `--tui/--no-tui` on `resolve`; bare mode auto-selects TUI when textual installed
  - `_tui_available()` + `_launch_tui()` extracted as helpers in cli.py. `--auto` + `--tui` = error (exit 2). Bare mode auto-picks TUI. Tests: `force_cli` fixture for existing bare-mode CLI tests; 5 new TUI-selection tests + 3 `main()` entry-point tests.
- [x] our simple resolve CLI should offer only actions supported for current file type or at least mark those that are supported. in the config we should configure the actions for each file type, e.g. which tool to use. it should also have recommendations for external tools to install (and ideally suggest a mise comment to do so)
  - `media.file_kind()` classifies groups (audio/image/video/office/kdbx/text/other); config `[file_types]` overrides which `[tools]` entry opens each kind; menu letters (d/m/e) only render for text, office/keepass folded into (v)iew.
  - `(?)tools` prints found/missing with install hints (INSTALL_HINTS in tools.py). (mise comment idea noted — these are OS app tools, hints give apt/brew.)
- [x] "view" should open file in suggested app, so no separate "(o)ffice (k)eepass" - and it should open both files and wait then.
  - `(v)iew` → `Launcher.view_commands()`: editor/office get both paths in one command; viewers one command per file; kdbx runs the keepass merge recipe; CLI pauses on Enter after open.
- [x] interactive resolve should have "(q)uit" option
  - (q)uit aborts after `N` groups, prints "aborted after N of total".
- [x] lets improve the initial "diff" table: identical flag / larger-smaller / metadata-only
  - columns: size, **vs base** (`=` or `≠ ↑/↓ <delta>`), **metadata** (same/diff + tag change detail), suggest; footer lists per-copy metadata verdicts for media. Office/audio compare extracted summaries; mp3 shows actual tag changes (`title: A → B`).
- [x] add an option to printout which internal or external tool will be used for the different actions (like "(?) print tools" maybe)
  - `(?)tools` shows kind → view tool + found/missing status + install hints for every configured tool.
- [x] when deconflict says "metadata differs", add "(m)etadata" menu option to show common + diff fields
  - `media.metadata_fields(path, kind, tools)` returns a field→value dict per kind (audio tags / image dims+EXIF / video streams / office content); `_print_metadata_diff` renders a table with base and copy as columns and common (dimmed) + differing fields as rows, section-separated. `(m)` reuses the meld key (mutually exclusive: meld is text-only, metadata diff is media-only).
- [x] overview table polish: first `role` column (base / copy N), per-row differing metadata fields in the `metadata` cell (only the differing), and removed the `suggest` column.
- [x] overview table columns: re-added `mtime`; merged size + vs-base delta into one `size` column (`3.4 MB (↓ 993 B)`); `#` column shows `#0` for base and `#N` for copies.
- [x] numbered keep actions supporting multiple copies: menu shows `(0) keep base`, `(1) keep copy #1`, ... and the resolver already backs up all non-chosen conflicts, so any copy can be kept. (`_keep_by_number` maps `#0`→base / `#N`→copy.)
- [x] truncate long metadata values (office XML) in the overview cell and `(m)etadata` table via `_truncate_meta` (≤70 chars + `… (+N more)`), so cell text stays readable.
- [x] metadata diff table headers now show the file number (`#0`, `#N`) plus a truncated filename (`_meta_diff_columns`, 40 char cap), and columns use `overflow="ellipsis"` so long names truncate instead of wrapping into tall multi-line headers.
- [x] Write CliRunner test for bare-mode interactive (monkeypatch config path, feed "s"=skip) to fix deterministically
  - `test_bare_skips_all_groups_interactively`, quit, `?`-tools, table-flags, no-dirs guidance.
- [x] `move_to_backup` fast path: atomic `os.replace` when same filesystem, EXDEV fallback to copy→verify→remove (resolve.py; tests green)
- [x] important: when launching e.g. audio player (and other documents), print full file path to console, not just "launching: /usr/bin/vlc …" and I don't know what is launched
  - `launchers._launch` now prints the whole argv with paths (space-containing args quoted); test asserts full command is shown.
- [x] the metadata detail table should always include create time, mod time, and file size (bytes + the size we have already in the overview table including the diff)
  - `_print_metadata_diff` renders ONE combined table titled "metadata diff": generic attrs (`size`/`created`/`modified`) + per-kind content fields in the SAME table (content section-separated). `size` row shows BOTH human-readable AND byte count (`1.2 MB (1,234,567 bytes)`) plus `(↑/↓ …)` diff vs the other file. `(m)etadata` always offered (attrs guarantee content); `_meta_capable` gate removed; `engine.supported_tools` always includes `VIEW_META`.
  - Attr rows (`size`/`created`/`modified`) are GRAY (dim) when the two files' values are identical, plain when they differ — verified + regression test (`test_metadata_attr_rows_dim_when_identical`: equal-size pair dimmed, differing-size pair plain, via recorded Rich console).
- [x] **enrich audio/image metadata with ExifTool**: `(m)`etadata detail for audio+image now uses `[tools].exiftool` when available (surfacing IPTC/XMP/MakerNotes + technical stream info: bitrate, sample rate, channel mode, resolution, color components, …), falling back to the lighter mutagen/PIL extractors when exiftool is absent/errors (never hard-required — AGENTS.md).
  - `media._exiftool_fields(path, exiftool, image=)` runs `exiftool -s -j`, parses JSON, normalizes keys (`Title`→`title`, `MIMEType`→`mimeType`, `MPEGAudioVersion`→`mpegAudioVersion`), folds ImageWidth/ImageHeight into `dimensions` (image), and skips fs-noise/binary/thumbnail/maker-note/redundant fields + values >200 chars and empty values. `_content_fields` is still the ONE canonical extractor feeding BOTH the overview verdict and the diff table (consistency preserved).
  - Tests: 4 mocked-subprocess unit tests (parse/normalize, dims merge, error→fallback, exiftool-vs-fallback in `_content_fields`). Verified against real samples (IMG now yields resolution/encoding/color-component fields; mp3 yields bitrate/sample-rate/channel-mode/duration).
  - **Performance note:** each exiftool run is a ~0.5s subprocess, so first `analyze` (and the test suite) is slower — but `meta_fields` is computed once and cached, so repeat runs are fast. If it ever matters, batch exiftool across all files of a scan in one process (exiftool accepts multiple paths).
- [x] when `deconflict` is running a scan, there should be some console output indicating this (and progress), when no scan is done but just cache is read, this should also be logged to console
  - `Engine.cache_used` set when the cache file exists; `_report_scan()` prints "scanning <dirs>…" or "using cached scan (<file>)…" in scan/resolve/bare. `scan(scani.on_file=...)` hook added for future progress bars.
- [x] the (?) action should also print a note how the tools to use can be configured (incl the config file path used)
  - `_print_tools` footer now prints the effective config path + which TOML sections ([tools]/[file_types]/[file_types_edit]) to edit; `EngineConfig.config_path` threaded from config load.
- [x] config file should use the standard XD... env vars (if it does not yet already)
  - `XDG_CONFIG_HOME`/`XDG_CACHE_HOME` honored for the default config file and cache dir (`config.py` + `engine._default_cache_dir()`).
- [x] when view and edit commands are identical (e.g. here for me with libreoffice), only show (e)dit, and not (v)iew (pls rename `(v)open` to `(o)pen`)
  - `Launcher.view_edit_collapse(kind)` — when VIEW and EDIT resolve to the same exe (office/editor), the menu shows only `(e)dit`. Office `(v)open` renamed `(o)pen`.
- [x] we should improve the metadata handling in general: when scanning / analyzing, metadata should be included in result, so we don't need to re-analyze. but truncate; pls recommend how to treat embedded cover art in audio, could be large ...? -> to we have generic metadata independent of file type. so we can always have the (m)etadata action available
  - `GroupAnalysis.meta_fields` holds full field dicts per file, computed once in `analyze_group`; CLI reads them (no re-extract on (m)). `(m)etadata` now always offered (generic attrs at minimum). **Embedded cover art**: mutagen tags with `APIC`/`PIC` frames are excluded from `_ID3_FIELDS` map, so images don't enter the summary — but to be safe, recommend truncating any large binary tag value (see follow-up; current tags map only over text frames).
- [x] I noticed with IMG-20181122-WA0004.jpg that the overview table says "metadata same", but meta-diff returns several differences!
  - Root cause: `metadata_equal` (via `image_summary` string) and `_image_fields` (dict) extracted differently. Unified on ONE `_content_fields(path, kind)` canonical extractor used by BOTH `metadata_equal` and the diff table — overview verdict and `(m)` rows can no longer disagree. Regression test added.
- [x] improve metadata for office documents. could it be that it currently takes the plain xml?
  - Yes — it dumped raw XML. Now `_office_fields()` reads `docProps/core.xml` (title/creator/lastModifiedBy/created/modified) + `_extract_office_text()` strips markup into readable paragraphs/cell text for docx/pptx/xlsx/ods (no angle-bracket soup).
- [x] is there a char-based diff, e.g. highlight the changes within a line?
  - `_print_text_diff` uses `SequenceMatcher` to highlight only the changed character runs within a changed line pair (`_hl_runs` + `_print_rich_line`).
- [x] the final message of resolve still says "resolved x groups, even if some where skipped and not resolved
  - `_run_interactive` counts resolved vs skipped; prints `resolved N group(s)` and `(M skipped)` when M>0.
- [x] **tool-actions matrix**: separate `(v)iew` from `(e)dit` per kind; audio `(v)iew` → audio player; add `(x)edit-meta`; universal meta-diff via ExifTool `-diff`; per-kind diff defaults; default `mp3_editor` → GUI not kid3-cli. Includes launchers.py ToolTypes (VIEW/EDIT/DIFF/VIEW_META/EDIT_META), KIND_*TOOL tables, config `[file_types_edit]`, cli menu via `_actions`, rejected-tag-default choice, tests.
  - VIEW/EDIT/DIFF/VIEW_META/EDIT_META ToolTypes exist; `KIND_VIEW/EDIT/META_EDIT_TOOL` tables keyed per kind; `tools.py` defaults prefer GUI tag editors: `mp3_editor = [kid3, easytag, picard, kid3-cli]` (kid3-cli last-resort). Menu via `_actions` exposes (v)iew/(e)dit collapsed when same exe, (d)iff, (m)etadata (always), (e)dit-meta where applicable.
  - **Tag-editor default (decision):** keep candidate list as-is — GUI preferred, CLI fallback; end users override via `[tools].mp3_editor = "..."` and config template documents it. Not hard-coding one GUI.
  - **Open follow-up (small, optional):** `--auto` rule set is `base|copy|newest|bigger`; the plan floated a metadata-based `meta`/`reject-meta` rule — NOT implemented (metadata verdict only gates (m)etadata availability, not auto-resolution). Defer unless requested.
- [x] what does actually (h)keep-both do: will it rename the copies to something not matching the pattern? is or should this be interactive?
  - Previously `Resolver.keep_both` didn't exist → AttributeError. Implemented: keeps base (untouched) and renames each conflict copy in place to `<base> (copy N).ext`, stopping it matching the pattern. Non-interactive, non-destructive, dry-run safe. (Note: renames live in the scan root rather than backup — acceptable for keep-both since nothing is discarded.)
- [x] office diff fails: launching: /usr/bin/soffice … LibreOffice 25.8.7.3 580(Build:3) Error in option: --compare
  - This build rejects `--compare`. `(d)iff` for office now diffs the extracted text in the terminal (`_print_office_diff`), which also works headlessly. soffice `--compare` removed from `launchers._diff_cmd`.
- [x] the "(?)tools" action should print the tools that are run for this very specific file / file type. do not list all tools, but just the ones we use here
  - `Launcher.tools_for(kind)` returns only the tool keys backing this kind's actions; `_print_tools` lists those (found/missing + hints) only.
- [x] add `deconflict config tools` to show tools for each filetype, just as the (?) action does
  - `config` became a Typer group: bare `config` still prints the effective config; `config tools [--path]` iterates all KINDS (audio/image/video/office/kdbx/text/other) using the SAME `_print_kind_tools` helper as the interactive (?)tools action (view/edit keys + found/missing + hints). Both honor root `--config` and `--path`. Extracted `_print_tools_footer`; fixed latent Rich markup bug where `[tools]`/`[file_types]` section names in the footer were swallowed as style tags (now escape()d — same class as the earlier `deconflict config` MarkupError).
- [x] the (m)eld action should be more generic. so (d)iff is console diff per default (depending on file type), maybe "(m)erge" instead?
  - Dropped the meld-specific text action; `(d)iff` is now the primary inline console diff (terminal for text/office). Metadata now owns hotkey `m` (`(m)etadata`), so the generic external merge would need another key — leaving external meld out of the default text menu (terminal diff + (o)pen/(e)dit cover it).
- [x] Fix `deconflict config` MarkupError crash: paths/values containing `[`/`]` (e.g. `/tmp/p/deconf/`) were interpolated into Rich markup unescaped; escape() now wraps the final rendered string of every config body line. Fixed 2026-08-30: all 93 tests green, `uv run deconflict config` works. (Note: whole rendered line is escaped — escaping only the value is insufficient because our own literal `[...]` wrapper brackets then form a rich closing tag.)
- AI tooling hooks: pre-commit (ruff+format), dependabot/renovate. Editor Copilot/Continue optional.
- Future: Textual TUI, PySide GUI.
- Office metadata: parse docx/xlsx/ods XML (document.xml/sharedStrings/content.xml) into real per-field metadata (text runs, cell values) instead of raw truncated XML — see metadata fields item above; also lets the `(m)etadata` table show meaningful common/diff rows for office.
- Multi-copy: menu keep actions now numbered per copy (resolver already handles N conflicts); consider applying the same numbering to `(v)iew`/`(m)etadata` targets.
- [ ] **metadata table header truncation**: `_meta_diff_columns` currently keeps only the START of the filename (`name[:limit-1] + "…"`). For the copy header the distinguishing part is the trailing conflict-pattern (e.g. `(conflicted copy 2025-07-27 123529).xlsx`), which gets cut off. On truncation, also include a meaningful number of chars from the END (`start…tail`), so the copy-file pattern stays visible — fit both parts + the `…` into `limit`.
- [ ] IMPORTANT: when printing the group we have to print the full path to the files as well: the "base" where the search did start should be highlighted so that the user can quickly see which sync it is from
- [ ] Add --version option and / or version command
- [ ] Add --progress option which should be enabled by default in interactive environments (when not called from a script)
- [ ] calling `deconflict --resolve` with a cache with 62 groups takes surprisingly long to start up. why? does it now read the metadata? as a first step write some log output. And check if things are really just read once: scan should read metadata, but we could also do this in parallel: this is an important architectural aspect: the tool should be able to use parallel workers, for scanning the file system but also for reading metadata. BTW: there are very fast tools for searching filesystems (fd finder and alike), so if one is present, we could also use this. this implies that we probably want 3 steps in total: first scan filesystem, then read metadata and then start resolving. The architecture needs to ensure that those steps can run in parallel: as soon as the first filename patterns are found, the metadata workers can start their job and as soon as the first one completed, we can start resolving. please make an architectural suggestion how we can support this
- [ ] bug: with ODT files the edit action opens only the copy, not both, so I cannot compare
- [ ] after using it a bit: I think we can omit the initial overview table but rather use the metadata table directly (and then remove the metadata action), I think the only thing ew need to ensure is that in the metadata table we get wrapping if file names are too long to be rendered and we need an additional row if content is identical or changed (with very brief diff if supported)
- [ ] based on the comparison we should recommend which file to keep if it is clear: like everything is identical, or one is clearly the more recent one where stuff was added and nothing removed. In this case: offer a default keep action
- [ ] do some research if there are diff tools for the different office document format a) to convert to text-based diff and b) open the GUI (e.g. an office app or a specialized tool) diff should work for office files
- [ ] Launching libreoffice etc should not block the main process, as we might wanna keep the app open. in fact we can directly go to the menu again and wait for next user action
- [ ] we now only have the text diff, but not way to open meld anymore? that seem to got lost. the actions should be clear: open and or edit (if different apps) -> open all files in group; text diff; external diff; external merge (show actions that are supported for current file type in current setup) "?" should also tell which tools are missing and make recommendations
- [ ] add a "setup" command that analyzes the local setup and makes recommendations about what to install and how
- [ ] the metadata compare table should print newer dates highlighted, and larger size also highlighted, maybe bold.
- [ ] all CLI arg `--auto recommended` that auto resolves files that have a clear recommendation what to keep (see above). Will skip others. When finished, it prints stats and if files have been skipped, it prints a recommendation to run `deconflict resolve` once again without `--auto`. As the cache is updated when the command is completed, no additional scan is required.
