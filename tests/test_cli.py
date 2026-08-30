"""CLI tests via Typer's CliRunner (no interactive stdin; use --auto --dry-run)."""

from __future__ import annotations

from typer.testing import CliRunner

from deconflict.cli import app

runner = CliRunner()


def test_patterns():
    result = runner.invoke(app, ["patterns"])
    assert result.exit_code == 0
    assert "nextcloud" in result.stdout
    assert "syncthing" in result.stdout


def test_scan_lists_groups(sample_dir):
    result = runner.invoke(app, ["scan", str(sample_dir)])
    assert result.exit_code == 0
    assert "conflict group(s)" in result.stdout


def test_scan_no_conflicts(tmp_path):
    (tmp_path / "only.txt").write_text("nope")
    result = runner.invoke(app, ["scan", str(tmp_path)])
    assert result.exit_code == 0
    assert "no conflicts found" in result.stdout


def test_resolve_auto_dry_run(sample_dir):
    result = runner.invoke(app, ["resolve", str(sample_dir), "--auto", "newest", "--dry-run"])
    assert result.exit_code == 0
    assert "dry-run" in result.stdout


def test_init_config_writes(tmp_path):
    target = tmp_path / "cfg" / "config.toml"
    result = runner.invoke(app, ["init-config", "--path", str(target)])
    assert result.exit_code == 0
    assert target.exists()
    # refusing to overwrite
    result2 = runner.invoke(app, ["init-config", "--path", str(target)])
    assert result2.exit_code == 1


def test_generated_template_parses(tmp_path):
    """Regression: template must be valid TOML with its fields loadable."""
    import tomllib

    target = tmp_path / "cfg" / "config.toml"
    runner.invoke(app, ["init-config", "--path", str(target)])
    raw = tomllib.loads(target.read_text(encoding="utf-8"))
    # default_dirs present, and the example custom pattern's regex is intact
    assert "default_dirs" in raw["scan"]
    example = raw["scan"]["patterns"][0]
    assert example["base_name"] == r"(.+)\.bak$"


def test_config_command_defaults(tmp_path, monkeypatch):
    monkeypatch.setattr("deconflict.config.DEFAULT_CFG_PATH", tmp_path / "missing.toml")
    result = runner.invoke(app, ["config"])
    assert result.exit_code == 0
    assert "config file:" in result.stdout
    assert "built-in defaults" in result.stdout
    assert "backup_dir" in result.stdout


def test_config_command_effective(tmp_path):
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text('[scan]\nenabled_patterns=["nextcloud"]\n')
    result = runner.invoke(app, ["config", "--path", str(cfgfile)])
    assert result.exit_code == 0
    assert "cfg.toml" in result.stdout
    assert "nextcloud" in result.stdout


def test_malformed_config_warns(tmp_path):
    """A config that can't parse must warn, not silently fall back to defaults."""
    import warnings

    from deconflict.config import load

    cfgfile = tmp_path / "bad.toml"
    cfgfile.write_text("[scan]\ndefault_dirs = [\n")  # truncated array -> invalid TOML
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        cfg = load(cfgfile)
    assert cfg.engine.dirs == []  # falls back to defaults
    assert any("could not parse config" in str(w.message) for w in caught)


def test_scan_with_config_flag(sample_dir, tmp_path):
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text(f'[scan]\ndefault_dirs=["{sample_dir}"]\n')
    result = runner.invoke(app, ["scan", "--config", str(cfgfile)])
    assert result.exit_code == 0
    assert "conflict group(s)" in result.stdout


def test_scan_with_global_config_flag(sample_dir, tmp_path):
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text(f'[scan]\ndefault_dirs=["{sample_dir}"]\n')
    result = runner.invoke(app, ["--config", str(cfgfile), "scan"])
    assert result.exit_code == 0
    assert "conflict group(s)" in result.stdout


def test_cache_clean_removes_stale(tmp_path):
    import json

    alive_dir = tmp_path / "scan"
    alive_dir.mkdir()
    cache = tmp_path / "cache"
    cache.mkdir()
    (cache / "dead.json").write_text(json.dumps({"/gone/nope.txt": {}}), encoding="utf-8")
    (cache / "alive.json").write_text(json.dumps({str(alive_dir / "ok.txt"): {}}), encoding="utf-8")
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text(f'[general]\ncache_dir="{cache}"\n')
    result = runner.invoke(app, ["cache", "clean", "--config", str(cfgfile)])
    assert result.exit_code == 0
    assert not (cache / "dead.json").exists()  # stale pruned
    assert (cache / "alive.json").exists()  # live kept
    assert "freed" in result.stdout


def test_cache_clean_dry_run_keeps_files(tmp_path):
    import json

    cache = tmp_path / "cache"
    cache.mkdir()
    (cache / "dead.json").write_text(json.dumps({"/gone/a.txt": {}}), encoding="utf-8")
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text(f'[general]\ncache_dir="{cache}"\n')
    result = runner.invoke(app, ["cache", "clean", "--dry-run", "--config", str(cfgfile)])
    assert result.exit_code == 0
    assert "would remove" in result.stdout
    assert (cache / "dead.json").exists()


def test_cache_invalidated_after_resolve(sample_dir, tmp_path):
    """A real resolve drops the now-stale cache so the next scan recomputes."""
    from deconflict.cache import cache_key

    cache = tmp_path / "cache"
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text(f'[scan]\ndefault_dirs=["{sample_dir}"]\n[general]\ncache_dir="{cache}"\n')
    scan = runner.invoke(app, ["scan", "--config", str(cfgfile)])
    assert scan.exit_code == 0
    key = cache_key([sample_dir])
    assert (cache / f"{key}.json").exists()  # cache populated by the scan
    resolved = runner.invoke(app, ["resolve", "--config", str(cfgfile), "--auto", "newest"])
    assert resolved.exit_code == 0
    assert not (cache / f"{key}.json").exists()  # invalidated after resolve


def test_cache_kept_after_dry_run_resolve(sample_dir, tmp_path):
    """A --dry-run resolve must NOT touch the cache."""
    from deconflict.cache import cache_key

    cache = tmp_path / "cache"
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text(f'[scan]\ndefault_dirs=["{sample_dir}"]\n[general]\ncache_dir="{cache}"\n')
    runner.invoke(app, ["scan", "--config", str(cfgfile)])
    key = cache_key([sample_dir])
    assert (cache / f"{key}.json").exists()
    runner.invoke(app, ["resolve", "--config", str(cfgfile), "--auto", "newest", "--dry-run"])
    assert (cache / f"{key}.json").exists()  # untouched
