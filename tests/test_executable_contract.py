"""Black-box CLI contract. Set DECONFLICT_EXECUTABLE to test another executable.

The Python default runs the installed package with the current interpreter. The
override is a single executable path, not a shell command.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest


@pytest.fixture
def cli(tmp_path: Path):
    executable = os.environ.get("DECONFLICT_EXECUTABLE")
    command = [executable] if executable else [sys.executable, "-m", "deconflict"]
    env = os.environ.copy()
    env.update(
        XDG_CONFIG_HOME=str(tmp_path / "xdg-config"),
        XDG_CACHE_HOME=str(tmp_path / "xdg-cache"),
        NO_COLOR="1",
        COLUMNS="200",
    )
    config = tmp_path / "config.toml"
    config.write_text(
        "[general]\n"
        f"backup_dir = {json.dumps(str(tmp_path / 'backup'))}\n"
        f"cache_dir = {json.dumps(str(tmp_path / 'cache'))}\n",
        encoding="utf-8",
    )

    def run(*args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [*command, *map(str, args)],
            cwd=tmp_path,
            env=env,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    return run, config


@pytest.mark.parametrize(
    ("pattern", "copy"),
    [
        ("nextcloud", "item (conflicted copy 2020-01-01 000000).txt"),
        ("pacman", "item.txt.pacnew"),
        ("syncthing", "item.sync-conflict-20200101-000000-ABC123.txt"),
    ],
)
def test_scan_builtin_patterns(cli, tmp_path: Path, pattern: str, copy: str):
    run, config = cli
    scan = tmp_path / "scan"
    scan.mkdir()
    (scan / "item.txt").write_text("base", encoding="utf-8")
    (scan / copy).write_text("copy", encoding="utf-8")
    result = run("scan", scan, "--config", config, "--pattern", pattern)
    assert result.returncode == 0, result.stderr
    assert "1 conflict group(s)" in result.stdout
    assert pattern in result.stdout
    assert "item.txt" in result.stdout


def test_patterns_lists_enabled_defaults(cli):
    run, _ = cli
    result = run("patterns", "--enabled")
    assert result.returncode == 0, result.stderr
    for pattern in ("nextcloud", "pacman", "syncthing"):
        assert pattern in result.stdout
    # Python builds this listing from enabled patterns, even without --enabled.
    assert "dpkg" not in result.stdout


@pytest.mark.parametrize("choice", ["base", "copy"])
def test_resolution_dry_run_and_backup(cli, tmp_path: Path, choice: str):
    run, config = cli
    scan = tmp_path / "scan"
    scan.mkdir()
    base = scan / "item.txt"
    copy = scan / "item (conflicted copy 2020-01-01 000000).txt"
    base.write_text("base content", encoding="utf-8")
    copy.write_text("copy content", encoding="utf-8")
    os.utime(base, (1_600_000_000, 1_600_000_000))
    os.utime(copy, (1_600_000_100, 1_600_000_100))
    backup = tmp_path / "backup"

    args = ("resolve", scan, "--config", config, "--auto", "base" if choice == "base" else "newest")
    dry = run(*args, "--dry-run")
    assert dry.returncode == 0, dry.stderr
    assert "dry-run" in dry.stdout
    assert base.read_text(encoding="utf-8") == "base content"
    assert copy.read_text(encoding="utf-8") == "copy content"
    assert not backup.exists()
    cache_files = list((tmp_path / "cache").rglob("*.json"))
    if os.environ.get("DECONFLICT_EXECUTABLE"):
        assert not cache_files
    else:
        assert cache_files  # Python's known dry-run discrepancy.

    result = run(*args)
    assert result.returncode == 0, result.stderr
    assert base.read_text(encoding="utf-8") == f"{choice} content"
    assert not copy.exists()
    loser = copy if choice == "base" else base
    backups = [p for p in backup.rglob(loser.name) if p.is_file()]
    assert len(backups) == 1
    assert backups[0].read_text(encoding="utf-8") == (
        "copy content" if choice == "base" else "base content"
    )
    log = (backup / "decision-log.jsonl").read_text(encoding="utf-8")
    entries = [json.loads(line) for line in log.splitlines()]
    assert len(entries) == 1
    assert entries[0]["action"] == f"keep_{choice}"
    assert entries[0]["base"] == str(base)
    assert any(d["moved_to"] == str(backups[0]) for d in entries[0]["decisions"])


def test_init_config_setup_config_tools_and_cache(cli, tmp_path: Path):
    run, _ = cli

    custom = tmp_path / "custom.toml"
    init = run("init-config", "--path", custom)
    assert init.returncode == 0, init.stderr
    assert "wrote config:" in init.stdout
    assert custom.is_file()
    custom.write_text(
        "[scan]\n"
        'enabled_patterns = ["nextcloud", "pacman", "syncthing"]\n'
        "[general]\n"
        f"cache_dir = {json.dumps(str(tmp_path / 'cache'))}\n"
        f"backup_dir = {json.dumps(str(tmp_path / 'backup'))}\n",
        encoding="utf-8",
    )

    setup = run("setup", "--config", custom)
    assert setup.returncode == 0, setup.stderr
    assert "deconflict" in setup.stdout
    assert "external tools:" in setup.stdout

    config = run("config", "tools", "--path", custom)
    assert config.returncode == 0, config.stderr
    assert "text" in config.stdout

    cache = tmp_path / "rust-cache-contract"
    cache.mkdir()
    (cache / "old.json").write_text("{}", encoding="utf-8")
    cache_config = tmp_path / "cache.toml"
    cache_config.write_text(
        "[general]\n"
        f"cache_dir = {json.dumps(str(cache))}\n"
        f"backup_dir = {json.dumps(str(tmp_path / 'backup'))}\n",
        encoding="utf-8",
    )
    dry = run("cache", "clean", "--config", cache_config, "--dry-run")
    assert dry.returncode == 0, dry.stderr
    assert (cache / "old.json").is_file()

    clean = run("cache", "clean", "--config", cache_config)
    assert clean.returncode == 0, clean.stderr
    assert not (cache / "old.json").exists()


def test_auto_copy_observed_base_selection_discrepancy(cli, tmp_path: Path):
    """Python currently selects the base for --auto copy and fails to rename it."""
    if os.environ.get("DECONFLICT_EXECUTABLE"):
        pytest.skip("Python-specific observed discrepancy")
    run, config = cli
    scan = tmp_path / "scan"
    scan.mkdir()
    base = scan / "item.txt"
    copy = scan / "item (conflicted copy 2020-01-01 000000).txt"
    base.write_text("base", encoding="utf-8")
    copy.write_text("copy", encoding="utf-8")
    result = run("resolve", scan, "--config", config, "--auto", "copy")
    assert result.returncode != 0
    assert "cannot rename" in result.stderr
    assert base.read_text(encoding="utf-8") == "base"
    assert not copy.exists()
    backups = list((tmp_path / "backup").rglob(copy.name))
    assert len(backups) == 1
    assert backups[0].read_text(encoding="utf-8") == "copy"
    assert not (tmp_path / "backup" / "decision-log.jsonl").exists()
