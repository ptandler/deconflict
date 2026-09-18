"""Shared fixtures: copy the real `sample-files/` fixtures into a tmp dir."""

from __future__ import annotations

import contextlib
import shutil
from pathlib import Path

import pytest

from deconflict.cache import clean_cache
from deconflict.engine import EngineConfig
from deconflict.paths import expand

SAMPLE = Path(__file__).resolve().parent.parent / "sample-files"


def _default_cache_dir() -> Path:
    """The cache dir an engine uses when the config doesn't override it."""
    return expand(str(EngineConfig().cache_dir))


def _copy_all(dst: Path) -> list[Path]:
    dst.mkdir(parents=True, exist_ok=True)
    for src in sorted(SAMPLE.rglob("*")):
        if src.is_file():
            rel = src.relative_to(SAMPLE)
            target = dst / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, target)
    return list(dst.rglob("*"))


@pytest.fixture
def sample_dir(tmp_path: Path) -> Path:
    """Copy every sample file into a fresh tmp subdir; returns that scan root.

    The scan root is `tmp_path/scan`, so callers can park a backup/cache dir at
    `tmp_path/...` and keep it OUTSIDE the scan root (as production requires).
    """
    root = tmp_path / "scan"
    _copy_all(root)
    return root


@pytest.fixture
def samples_map(sample_dir: Path) -> dict[str, Path]:
    """Name -> path map of every fixture in the tmp copy (both base and conflict)."""
    return {p.name: p for p in sample_dir.iterdir() if p.is_file()}


@pytest.fixture
def force_cli(monkeypatch: pytest.MonkeyPatch) -> None:
    """Bare-mode tests exercise the CLI loop, not the Textual TUI.

    The dev environment installs the optional `tui` extra, so bare `deconflict`
    would otherwise launch the TUI. Tests that verify the CLI interactive loop
    opt into this fixture (which stubs `_tui_available`); tests that verify the
    TUI selection themselves override `_tui_available`/`_launch_tui`.
    """
    import deconflict.cli as cli_mod

    monkeypatch.setattr(cli_mod, "_tui_available", lambda: False)


def pytest_sessionfinish(session, exitstatus) -> None:
    """After all tests, prune the shared default cache of dead-dir entries.

    CLI tests that use the default config write scan caches into
    ~/.cache/deconflict keyed on pytest tmp dirs. Once pytest culls those dirs
    the caches are dead; this keeps the shared cache from growing unboundedly.
    Only the default stale-dir prune runs — the live config cache is untouched.
    """
    with contextlib.suppress(Exception):
        # cleanup is best-effort; never let it fail the test session
        clean_cache(_default_cache_dir())
