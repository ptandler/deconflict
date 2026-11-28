# AGENTS.md — deconflict

Interactive resolver for "conflicted copy"-style sync files (Nextcloud, pacman `.pacnew/.pacsave/.pacorig`, Syncthing `.sync-conflict-*`). Rust at the repository root is the main implementation, version 0.3.0; Python under `python/` is frozen as the behavioral reference.

## Main plan
Read `plans/plan-rust-migration.md` first — decisions, structure, behavior and current tasks. `.opencode/plans/2026-08-30-deconflict.md` is absent in this checkout. This file is the quick reference for conventions.
For each task you work: when starting and finishing the task, please update plan; you might want to include important results, also at intermediate steps. be brief and concise! Whenever the user enters or decides a requirement, record it in the current plan.

## Setup & commands (mise + uv)
```
mise install                 # Rust toolchain; Python 3.13 + uv also available
cargo fmt --all
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release
mise run check               # Rust fmt/clippy/tests (CI does same)
mise run python-check        # frozen Python reference tests
```

## Layout
```
src/             Rust lib.rs (UI-free core) + CLI/TUI frontend modules
tests/           Rust core, cache, resolve, CLI and PTY TUI tests
Cargo.toml       Rust package and binary, version 0.3.0
python/          frozen Python reference (do not modify); pyproject.toml, src/, tests/
sample-files/    shared fixtures at repo root; Rust tests copy from it, never mutate
plans/           main migration plan and related plans
```

## Conventions
- Work in Rust at the repo root; do not modify Python sources or tests under `python/` — Python is the read-only compatibility reference.
- `sample-files/` stays at the repo root by decision; do not mutate it in tests, and add new sample groups there only when requested. Python tests still pointing at `python/sample-files` are part of the reference to be retired.

- Rust core stays UI-free; see `.github/instructions/rust.instructions.md` for module boundaries and quality gates.
- Historical Python conventions below apply to the frozen reference only.
- Type hints everywhere; `from __future__ import annotations`; line length 100 (ruff).
- One concern per module. Never import Rich/Typer in non-UI modules (`engine.py` must stay UI-free).
- New conflict format: add matcher in `patterns.py` + entry in `PATTERNS`. Custom regex via config.
- Loser semantics: Nextcloud copy = local changes, base = server version. Pacman `.pacnew` = new upstream winner. Keep generic (pattern declares base/winner).
- Destructive ops ONLY through `src/resolve.rs` (backup OUTSIDE scan roots, serialize mutations); never delete sources directly. `--dry-run` must not touch the filesystem.
- Media: audio=mutagen, images=Pillow, video=ffprobe, office=stdlib zipfile text. External tools via `tools.py` (`shutil.which`, per-OS defaults) — never hard-require.
- Windows: pathlib only, `os.name` branches in `tools.py`, case-insensitive filename matching on NT.
- Decision log: JSONL in backup dir. Cache: JSON in cache dir.
- Keep docs (AGENTS.md, plans, README, CHANGELOG) precise and brief.

## Updating the Changelog

- Edit `CHANGELOG.md`
- Keep `[Unreleased]` section for things done since last release 
- Follow [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) format

## Versioning

This project uses [Semantic Versioning](https://semver.org/):
- **MAJOR** — Breaking changes
- **MINOR** — New features (backward compatible)
- **PATCH** — Bug fixes (backward compatible)

Current version defined in: `Cargo.toml` (`python/pyproject.toml` mirrors it for the frozen reference)
