# AGENTS.md — deconflict

Interactive resolver for "conflicted copy"-style sync files (Nextcloud, pacman `.pacnew/.pacsave/.pacorig`, Syncthing `.sync-conflict-*`). Rust WIP in `rust/` is the active implementation; the Python implementation is frozen as the behavioral reference.

## Main plan
Read `plans/plan-rust-migration.md` first — decisions, structure, behavior and current tasks. `.opencode/plans/2026-08-30-deconflict.md` is absent in this checkout. This file is the quick reference for conventions.
For each task you work: when starting and finishing the task, please update plan; you might want to include important results, also at intermediate steps. be brief and concise!

## Setup & commands (mise + uv)
```
mise install                 # python 3.13 + uv (from mise.toml)
uv sync                      # .venv + uv.lock
uv run deconflict {patterns|init-config|scan|resolve|--help|--version}
uv run ruff check .          # lint
uv run ruff format .         # format
uv run pytest                # tests
mise run check               # lint + format-check + pytest (CI does same)
cargo fmt --manifest-path rust/Cargo.toml --all
cargo test --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
cargo build --manifest-path rust/Cargo.toml --release
```

## Layout
```
rust/src/        lib.rs(UI-free core: patterns/scan/analysis/cache/resolve/media/tools)
                 cli.rs commands/ progress.rs report.rs interactive/ (CLI frontend)
rust/tests/      core, cache, resolve and executable-contract tests
rust/Cargo.toml  Rust package and binary
sample_files/    new Rust-only sample groups (sample-files/ stays frozen for Python contracts)
src/deconflict/  frozen Python reference; read for behavior, do not modify
tests/           pytest executable contracts; fixtures = COPIES of "sample-files/" in tmpdir
plans/           main migration plan and related plans
```

## Conventions
- Work in `rust/` only; do not modify Python sources or tests — Python is the read-only compatibility reference.
- Do not modify `sample-files/` (frozen Python fixture); add new Rust sample groups under `sample_files/` only when requested.
- Rust core stays UI-free; see `.github/instructions/rust.instructions.md` for module boundaries and quality gates.
- Historical Python conventions below apply to the frozen reference only.
- Type hints everywhere; `from __future__ import annotations`; line length 100 (ruff).
- One concern per module. Never import Rich/Typer in non-UI modules (`engine.py` must stay UI-free).
- New conflict format: add matcher in `patterns.py` + entry in `PATTERNS`. Custom regex via config.
- Loser semantics: Nextcloud copy = local changes, base = server version. Pacman `.pacnew` = new upstream winner. Keep generic (pattern declares base/winner).
- Destructive ops ONLY through `rust/src/resolve.rs` (backup OUTSIDE scan roots, serialize mutations); never delete sources directly. `--dry-run` must not touch the filesystem.
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

Current version defined in: `pyproject.toml`
