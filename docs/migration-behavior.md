# Python migration baseline

This inventory describes the reference implementation, not a promise to retain
known safety discrepancies. Existing unit tests are in `tests/`; executable
contracts complement them and interactive terminal parity needs PTY tests.

- CLI: `patterns`, `init-config`, `config` and `config tools`, `setup`, `scan`,
  `resolve`, `cache clean`, `version` and `--version`; bare invocation scans and
  resolves, preferring the TUI. Resolve supports interactive and automatic choices,
  dry-run, configuration and pattern overrides.
- TUI: group selection, files/diff/log views, keyboard and mouse actions, tool
  launch and resolution share the UI-free engine. The CLI has its own interactive
  loop. See `tests/test_cli.py` and the TUI tests for workflows.
- Configuration: TOML selects directories, enabled and custom Python-regex
  patterns, backup/cache directories, external tools and per-type tool mappings.
  XDG configuration and cache defaults apply. Missing or invalid config falls
  back to defaults; invalid config emits a warning.
- Discovery groups Nextcloud, pacman and Syncthing conflicts by default; optional
  patterns and custom regexes are available. Files are sorted, roots deduplicated
  by spelling, and symlinks are skipped by default. Overlapping roots and missing
  bases deserve explicit migration tests.
- Analysis uses SHA-256, timestamps, size and text sniffing; audio, image, video,
  office and KeePass formats have type-specific behavior. External programs are
  optional and detected at runtime; see `tests/test_media.py` and `tests/test_tools.py`.
- Resolution backs up losers and appends JSONL decisions; keep-both renames copies.
  Same-device backup uses rename, with a copy-and-verify cross-device fallback.
  Cache files are JSON and are invalidated after real resolution.
- Safety discrepancy: `Engine.scan()` refreshes the cache even when dry-run is
  set, contradicting the documented no-write guarantee. Preserve the guarantee,
  not this behavior, in the Rust implementation.
- Safety discrepancy: with an existing base, `--auto copy` selects the base,
  backs up the conflict and then fails to rename the base; no decision is logged.
  The intended copy selection must not reproduce this failure.
- Platform: path handling and tool defaults include Windows-specific behavior;
  matching is case-insensitive on Windows. The Rust implementation must validate
  its own platform behavior rather than infer it from Linux tests.