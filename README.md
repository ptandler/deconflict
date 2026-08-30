# deconflict

Interactive resolver for **"conflicted copy"-style sync conflict files**:
Nextcloud `(conflicted copy YYYY-MM-DD HHMMSS)`, pacman `.pacnew/.pacsave/.pacorig`,
and Syncthing `.sync-conflict-*`. Pattern-driven and generic, so custom formats can be
added via the config. Core logic is UI-free, so a future Textual TUI / PySide GUI can reuse it.

## Install

```sh
uv tool install .        # or: pipx install .
```

Runs from any directory. Python >= 3.11.

## Usage

```sh
deconflict                      # interactive resolve (scans configured dirs then menus)
deconflict scan                 # scan configured dirs, list conflict groups, exit
deconflict scan ~/Sync          # scan a specific dir recursively
deconflict scan --pattern nextcloud   # restrict to one pattern group
deconflict resolve              # interactive per-group resolution
deconflict resolve --auto newest      # non-interactive: base|copy|newest|bigger
deconflict resolve --dry-run    # preview without touching the filesystem
deconflict init-config          # write ~/.config/deconflict/config.toml
deconflict patterns             # list recognized conflict patterns
deconflict --help               # all commands & options
```

## Patterns

| group | enabled | matches |
|-------|---------|---------|
| `nextcloud` | on | `name (conflicted copy YYYY-MM-DD HHMMSS)` |
| `pacman` | on | `*.pacnew`, `*.pacsave`, `*.pacorig` |
| `syncthing` | on | `*.sync-conflict-*`, `*.sync-conflict-*-EXT` |
| `dpkg` | off | `*.dpkg-dist`, `*.dpkg-old`, `*.ucf-*` |
| `rpm` | off | `*.rpmnew`, `*.rpmorig`, `*.rpmold`, `*.rpmignore` |
| `windows-copy` | off | `* - Copy`, `* (copy)` |

Custom regex patterns go under `[[scan.patterns]]` in the config.

## How conflicts are resolved

For each group you choose (menu adapts to the file type): **(b)ase / (c)opy /
(v)iew both files in the suggested app / (d)iff in terminal / (m)eld / (e)ditor /
(h)keep-both / (s)kip / (q)uit / (?)tools.** View opens both the base and the
conflict copy in the app configured for that kind (editor, office, viewer,
KeePass merge for `.kdbx`).

Losers are **moved** to a backup dir outside the scan roots (`~/deconflict-backups/…`
by default) — never hard-deleted. A JSONL decision log records every choice.
`--dry-run` never touches the filesystem.

## Config

Run `deconflict --init-config` for a documented template at
`~/.config/deconflict/config.toml`. See `config.example.toml`.
`[file_types]` maps each file kind (text, audio, image, video, office, kdbx)
to the `[tools]` entry that `(v)iew` uses.

## Recommended external apps

Auto-detected at runtime (never required): **meld**/WinMerge (visual diff), **$EDITOR**,
**eog**/appropriate viewer, **ffprobe** (video), **keepassxc-cli** (KeePass merge),
**soffice** (office docs), **Kid3/Picard** (mp3 tag editing).

## Development

```sh
mise install      # python 3.13 + uv
uv sync           # .venv + uv.lock
uv run deconflict ...   # run the CLI
mise run check     # ruff check + format-check + pytest
```

## Windows

Uses only `pathlib` operations; `os.name` branches select per-OS tool defaults;
filename matching is case-insensitive on NT.

## License

MIT
