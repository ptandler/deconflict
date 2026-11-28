# deconflict — help me solve sync file conflicts!

Interactive resolver for **“conflicted copy”-style sync conflict files**. Supports:

- [Nextcloud](https://nextcloud.com/) `(conflicted copy YYYY-MM-DD HHMMSS)`
- [pacman](https://wiki.archlinux.org/title/Pacman) `.pacnew/.pacsave/.pacorig`
- [Syncthing](https://syncthing.net/) `.sync-conflict-*`

The Rust implementation is the main application (version 0.3.0). Pattern matching is generic, so custom formats can be added through config. Viewers, editors, diff tools and metadata extraction are optional; external tools are detected at runtime and never required.

## Install

```sh
git clone https://github.com/ptandler/deconflict.git
cd deconflict
mise install
cargo install --path .        # or: cargo build --release
```

## Usage

Run `deconflict init-config` once, edit `~/.config/deconflict/config.toml`, then resolve conflicts:

```sh
deconflict                       # scan configured dirs, then interactive resolution/TUI
deconflict ~/Sync                # scan a specific dir
deconflict scan --progress       # list conflicts with live progress
deconflict resolve --tui         # full-screen TUI
deconflict resolve --no-tui      # CLI resolver
deconflict resolve --dry-run     # preview without touching the filesystem
deconflict config tools          # show detected external tools
deconflict --help                # all commands and options
```

## TUI

- `q` quits immediately; `Q` waits for a running scan (only shown while work is pending).
- `h` opens keyboard shortcuts; Tab/Shift-Tab switch tabs.
- Mouse clicks on tabs and action buttons are supported.
- File tools are offered on the `files` tab and act on the selected file.
- Images have an explicit `images` tab using the kitty graphics protocol when available.
- Audio opens one file at a time; GUI players are preferred.

## Resolution

For each conflict group choose base, a copy, keep both, skip, diff, view/edit, metadata or quit. Losers are **moved** to a backup directory outside the scan roots (`~/deconflict-backups/…` by default), never hard-deleted. A JSONL decision log records every choice. `--dry-run` never touches the filesystem.

## Config

`deconflict init-config` writes a documented template to `~/.config/deconflict/config.toml`. `[scan.default_dirs]` and `[scan.enabled_patterns]` are the usual starting points. `[file_types]` maps text, audio, image, video, office and kdbx files to view tools; `[tools]` contains external tool paths.

Built-in patterns: `nextcloud`, `pacman`, `syncthing` (enabled); `dpkg`, `rpm`, `windows-copy` (opt-in). Custom regex patterns go under `[[scan.patterns]]`.

## Development

```sh
mise install
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
mise run check        # all Rust checks (CI does the same)
mise run python-check # frozen Python reference tests (non-gating in CI)
```

The shared CLI contract can be run against the Rust binary after `cargo build --release`:

```sh
DECONFLICT_EXECUTABLE="$PWD/target/release/deconflict" \
  uv run --directory python --extra dev python -m pytest tests/test_executable_contract.py -q
```

Publishing is tag-driven (`.github/workflows/publish.yml`): pushing `v0.3.0` publishes the crate to crates.io and the frozen Python reference to PyPI.

The Python implementation under `python/` is frozen and used only as a behavioral reference. Do not modify it when implementing Rust features.

## Timing breakdown

The final status row reports `discovery`, `analysis`, `waiting` and `total`. All values exclude time explicitly paused by CLI prompts. `discovery` spans scan start to the last discovery event; `analysis` spans the first to the last analysis event; the two windows may overlap. `waiting` is the residual `total - max(discovery, analysis)`: time outside both measured windows, typically interactive decision time in the TUI or other scheduler stalls. It is derived, not a separately measured idle timer.

## Windows

Path handling uses `pathlib`-equivalent Rust APIs; external tool defaults are per-OS and never hard-required. Filename matching is case-insensitive on NT.

## License

MIT
