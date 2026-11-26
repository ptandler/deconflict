---
description: "Use when writing or refactoring Rust code in this workspace. Covers architecture, CLI boundaries, testing, and maintainability."
applyTo: "rust/**/*.rs"
---

# Rust maintainability instructions

## Architecture

- Keep `src/main.rs` limited to process startup and command dispatch. Put argument types, config loading, and command implementations in focused modules.
- Keep UI-free logic in `src/lib.rs` modules. The CLI may depend on core types, but core modules must not depend on Clap or terminal rendering.
- Prefer one concern per module. Split a module when it repeatedly exceeds roughly 500 lines or mixes discovery, presentation, configuration, or mutation.
- Keep public APIs small and explicit. Use `pub(crate)` unless external reuse or integration tests require `pub`; avoid exporting implementation details.
- Use typed inputs and errors instead of `Result<T, String>` when a module can represent failure structurally. Preserve useful context at module boundaries.
- One config source of truth: parse/effective-config logic belongs in `config.rs`, path expansion in `paths.rs`, and CLI overrides in a CLI-specific module.
- Commands belong in `cli/` or `commands/` modules by concern (`scan`, `resolve`, `patterns`, `cache`, `setup/config`). Dispatch in `main.rs` should stay thin.
- Put progress rendering behind a UI-oriented trait or observer. The core scanner should emit typed events, not terminal/control-code output.
- Keep filesystem mutation serialization and backup safety in `resolve.rs`; cache writes in `cache.rs`; discovery/scheduling in `scan.rs`.

## Modules and dependencies

- Follow the Rust module system; use `mod` declarations and separate files for implementation, not large catch-all modules.
- Keep dependency direction clear: `main -> cli -> core`; never let core modules depend on the binary crate.
- Prefer constructors/builders and immutable configuration objects for shared setup.
- Reuse existing types instead of duplicating CLI/config/cache representations.
- Do not add dependencies for a problem already solved by an existing small dependency in the project; evaluate maintenance, MSRV, license, API stability, and platform support.

## Progress and cache

- Scanner events are typed and UI-neutral. Add a purpose-specific event for discovery, queued work, completion, and errors.
- Progress observers must tolerate repeated events, out-of-order completion, cancellation, and no terminal.
- Indeterminate work must not claim a known percentage. Use `indicatif` for terminal redraw, 250–600 ms refresh, and width-aware truncation.
- Cached progress estimates are hints, not guarantees. Revalidate them against actual scan totals.
- Keep cache JSON formats compatible with the Python implementation unless a documented migration is complete.
- Sidecars or metadata additions must be optional, removable, and ignored by stale-cache cleanup.
- `--dry-run` and `--no-cache` must never write cache files or metadata sidecars.

## Testing

- Unit tests in `src/...` test private implementation details and fast isolated behavior.
- Integration tests in `rust/tests/` test public library APIs and executable contracts, with separate files for scan, resolve, cache, and media concerns.
- Move CLI-only pure formatting and middle-truncation helpers out of `main.rs` and test them in the owning module or `rust/tests/cli.rs`.
- Fixture tests must copy `sample-files/` into temporary directories; never mutate repository fixtures.
- Test success, dry-run, cancellation, partial failure, missing files, changed files, symlinks, and cache invalidation.
- Test terminal and redirected progress behavior separately: no control sequences for piped output, one redraw line for TTY output.
- Add regression tests before fixing bugs in scanner scheduling or cache invalidation.

## Safety and correctness

- Mutation operations must remain backup-first, revalidate identities, serialize mutations, and log only successful decisions.
- Never allow resolution after an incomplete or cancelled scan.
- Prefer `?` propagation and typed error conversion over ad-hoc `unwrap/expect`, especially in I/O and filesystem paths.
- `expect/unwrap` is acceptable only for invariants proven by construction or tests, and must have a clear comment.
- Minimize unsafe code; do not use unsafe solely for convenience.
- Preserve deterministic output ordering despite parallel discovery and analysis.
- Cancellation must stop scheduling and not mutate sources.

## Quality gates

- Run `cargo fmt --all -- --check`.
- Run `cargo test --manifest-path rust/Cargo.toml`.
- Run `cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings`.
- Run release build and executable contracts for CLI changes.
- Fix all warnings; do not suppress lints without a reason tied to this crate’s compatibility constraints.
- Update the migration plan for meaningful progress, blockers, and validation results.
