# Plan: deconflict — interactive sync-conflict resolver

**Date:** 2026-08-30 · **Status:** core implemented & tested (75 green)

## Goal
Interactive CLI to find and resolve "conflicted copy"-style sync conflict files:
Nextcloud `(conflicted copy YYYY-MM-DD HHMMSS)`, pacman `.pacnew/.pacsave/.pacorig`,
Syncthing `.sync-conflict-*`. Generic, pattern-driven, future TUI/GUI-ready.

## Decisions (agreed)
- Name: **deconflict** (PyPI + command + module). Package runs from any dir; repo dir will be renamed (rename happens on restart).
- Stack: Python >=3.11 (dev 3.13) · Typer (CLI) · Rich (colors/progress/live) · ruff (lint+format) · pytest · hatchling · uv + mise · GitHub Actions (MIT).
- Logic UI-free in `engine.py` → later Textual TUI (`tui` subcommand) / PySide GUI reuse it.
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
- `deconflict resolve` / `deconflict` — **interactive driver that internally calls scan + resolve**
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
- [x] Write CliRunner test for bare-mode interactive (monkeypatch config path, feed "s"=skip) to fix deterministically
  - `test_bare_skips_all_groups_interactively`, quit, `?`-tools, table-flags, no-dirs guidance.
- [x] `move_to_backup` fast path: atomic `os.replace` when same filesystem, EXDEV fallback to copy→verify→remove (resolve.py; tests green)
- [ ] Commit this session's batch (resolve fast-path, kind-aware menu, view/quit/tools, table flags) via caveman-commit skill

## Open follow-ups
- AI tooling hooks: pre-commit (ruff+format), dependabot/renovate. Editor Copilot/Continue optional.
- Future: Textual TUI, PySide GUI.
- Multiple conflicts per base (group currently assumes ≤1 copy in menu; model supports many).
