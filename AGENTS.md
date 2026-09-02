# AGENTS.md — deconflict

Interactive resolver for "conflicted copy"-style sync files (Nextcloud, pacman `.pacnew/.pacsave/.pacorig`, Syncthing `.sync-conflict-*`). CLI = Typer + Rich, TUI = Textual (optional extra); core logic is UI-free so a PySide GUI could reuse it too.

## Main plan
Read `.opencode/plans/2026-08-30-deconflict.md` first — it contains the decisions, structure, behavior, and current todo list. This file is the quick reference for conventions.
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
```

## Layout
```
src/deconflict/  cli.py(typer) engine.py(UI-free) scan.py patterns.py analyze.py
                 media.py resolve.py cache.py tools.py paths.py launchers.py config.py
                 render.py(Rich renderables) actions.py(shared action matrix)
                 tui/   app.py views.py actions.py (Textual, optional)
tests/           pytest; fixtures = COPIES of "sample files/" in tmpdir — never write into "sample files/"
.opencode/plans/ project plans (main plan + tui-specific plan)
```

## Conventions
- Type hints everywhere; `from __future__ import annotations`; line length 100 (ruff).
- One concern per module. Never import Rich/Typer in non-UI modules (`engine.py` must stay UI-free).
- New conflict format: add matcher in `patterns.py` + entry in `PATTERNS`. Custom regex via config.
- Loser semantics: Nextcloud copy = local changes, base = server version. Pacman `.pacnew` = new upstream winner. Keep generic (pattern declares base/winner).
- Destructive ops ONLY via `resolve.py::Resolver.move_to_backup()` → backup OUTSIDE scan roots; never `os.remove`. `--dry-run` must not touch the filesystem.
- Media: audio=mutagen, images=Pillow, video=ffprobe, office=stdlib zipfile text. External tools via `tools.py` (`shutil.which`, per-OS defaults) — never hard-require.
- Windows: pathlib only, `os.name` branches in `tools.py`, case-insensitive filename matching on NT.
- Decision log: JSONL in backup dir. Cache: JSON in cache dir.
- Keep docs (AGENTS.md, plans, README) precise and brief.
