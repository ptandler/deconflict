# deconflict - help me solving sync file conflicts!

Interactive resolver for **"conflicted copy"-style sync conflict files**.
Supports:
- [Nextcloud](https://nextcloud.com/) `(conflicted copy YYYY-MM-DD HHMMSS)`
- [pacman](https://wiki.archlinux.org/title/Pacman) `.pacnew/.pacsave/.pacorig`
- [Syncthing](https://syncthing.net/) `.sync-conflict-*`

It's Pattern-driven and generic, so custom formats can be added via the config.

It is similar to the `pacdiff` for pacman. In order to be helpful with other file formats and not just plain text, it
supports several viewers, editors, diff tools, and also extracts metadata (images, audio, video) 
and also shows metadata diffs.

## Install

Clone this repo.

```sh
mise install             # a very convenient way to install `python` and `uv` 
uv sync                  # install python dependencies
uv tool install .        # install deconflict to PATH (or: instead of uv use `pipx install .`)
```

## Usage

First you should create your config with `deconflict init-config` and then edit the generated
`~/.config/deconflict/config.toml`. Most likely you want to configure `default_dirs` so that you don't have to pass paths via CLI, and maybe you want also to set `enabled_patterns`.

Then you can simply do `deconflict` and resolve conflicts one by one.

## Commands

```sh
deconflict                            # run `scan`and interactive resolution (TUI if available)
deconflict ~/Sync                     # scan specific dirs in bare interactive mode
deconflict scan                       # scan configured dirs and list conflicts
deconflict scan ~/Sync --pattern pacman
deconflict scan --no-cache            # bypass the file cache
deconflict scan --refresh             # clear cache and rebuild metadata
deconflict scan --progress            # show live scan progress
deconflict resolve                    # interactive per-group resolution
deconflict resolve --tui              # launch the Textual TUI
deconflict resolve --no-tui           # force the CLI resolver
deconflict resolve --auto newest      # base|copy|newest|bigger|recommended
deconflict resolve --dry-run          # preview without touching the filesystem
deconflict init-config                # write ~/.config/deconflict/config.toml
deconflict config                     # show effective configuration
deconflict config tools               # show tools used for each file kind
deconflict setup                      # analyze config and external tools
deconflict patterns --enabled         # list enabled conflict patterns
deconflict cache clean                # remove stale cache entries
deconflict cache clean --dry-run      # preview cache cleanup
deconflict cache clean --all          # remove every cache file
deconflict cache clean --older-than-days 30
deconflict version                    # print the version
deconflict --help                     # all commands and options
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

Run `deconflict init-config` for a documented template at
`~/.config/deconflict/config.toml`. See
[`src/deconflict/config.example.toml`](src/deconflict/config.example.toml) (shipped in the
package, so it works from an install).
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
