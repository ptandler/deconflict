"""CLI tests via Typer's CliRunner (no interactive stdin; use --auto --dry-run)."""

from __future__ import annotations

import shutil
from pathlib import Path

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


def test_bare_defaults_to_tui_when_available(sample_dir, tmp_path, monkeypatch):
    """Bare `deconflict` launches the TUI when textual is installed."""
    import deconflict.cli as cli_mod
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    calls: list[tuple] = []
    monkeypatch.setattr(cli_mod, "_tui_available", lambda: True)
    monkeypatch.setattr(
        cli_mod, "_launch_tui", lambda engine, groups: calls.append((engine, groups))
    )
    result = runner.invoke(app, [])
    assert result.exit_code == 0
    assert len(calls) == 1
    engine, groups = calls[0]
    assert len(groups) == 6


def test_resolve_tui_flag_launches_tui(sample_dir, monkeypatch):
    """`resolve --tui` launches the TUI app instead of the CLI loop."""
    import deconflict.cli as cli_mod

    calls: list[tuple] = []
    monkeypatch.setattr(cli_mod, "_tui_available", lambda: True)
    monkeypatch.setattr(
        cli_mod, "_launch_tui", lambda engine, groups: calls.append((engine, groups))
    )
    result = runner.invoke(app, ["resolve", str(sample_dir), "--tui"])
    assert result.exit_code == 0
    assert len(calls) == 1


def test_resolve_tui_missing_extra_errors(sample_dir, monkeypatch):
    """`resolve --tui` without textual installed exits 1 with install guidance."""
    import deconflict.cli as cli_mod

    monkeypatch.setattr(cli_mod, "_tui_available", lambda: False)
    result = runner.invoke(app, ["resolve", str(sample_dir), "--tui"])
    assert result.exit_code == 1
    assert "textual" in result.stdout
    assert "--extra tui" in result.stdout


def test_resolve_auto_tui_conflict(sample_dir, monkeypatch):
    """`--auto` and `--tui` are mutually exclusive -> exit 2 before any action."""
    import deconflict.cli as cli_mod

    monkeypatch.setattr(cli_mod, "_tui_available", lambda: True)
    result = runner.invoke(app, ["resolve", str(sample_dir), "--auto", "newest", "--tui"])
    assert result.exit_code == 2
    assert "mutually exclusive" in result.stdout


def test_resolve_no_tui_forces_cli_loop(sample_dir, tmp_path, monkeypatch):
    """`resolve --no-tui` runs the CLI loop even when the TUI is installed."""
    import deconflict.cli as cli_mod
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    monkeypatch.setattr(cli_mod, "_tui_available", lambda: True)
    result = runner.invoke(app, ["resolve", "--no-tui", "--dry-run"], input="s\n" * 10)
    assert result.exit_code == 0
    assert "resolved 0 group(s) (6 skipped)" in result.stdout


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


def test_config_tools_lists_all_kinds(tmp_path):
    """`config tools` shows the (?)tools view for every file kind."""
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text("[scan]\n")
    result = runner.invoke(app, ["config", "tools", "--path", str(cfgfile)])
    assert result.exit_code == 0
    for kind in ("audio", "image", "video", "office", "kdbx", "text", "other"):
        assert f"file kind: {kind}" in result.stdout
    assert "view:" in result.stdout and "edit:" in result.stdout
    assert "configure tools in" in result.stdout


def test_config_tools_respects_file_types_override(tmp_path):
    """A [file_types] override changes the view tool key shown for that kind."""
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text('[file_types]\naudio = "my_audio_player"\n')
    result = runner.invoke(app, ["config", "tools", "--path", str(cfgfile)])
    assert result.exit_code == 0
    assert "view: my_audio_player" in result.stdout


def test_config_tools_accepts_global_config_flag(tmp_path):
    """The root --config flag also feeds `config tools`."""
    cfgfile = tmp_path / "cfg.toml"
    cfgfile.write_text('[file_types]\nimage = "my_viewer"\n')
    result = runner.invoke(app, ["--config", str(cfgfile), "config", "tools"])
    assert result.exit_code == 0
    assert "view: my_viewer" in result.stdout
    assert "file kind: image" in result.stdout


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


def _home_cfg(tmp_path, sample_dir) -> str:
    """A config for the bare-mode tests: default dirs = sample copy."""
    cfgfile = tmp_path / "home" / ".config" / "deconflict" / "config.toml"
    cfgfile.parent.mkdir(parents=True)
    cfgfile.write_text(f'[scan]\ndefault_dirs=["{sample_dir}"]\n')
    return str(cfgfile)


def test_bare_skips_all_groups_interactively(sample_dir, tmp_path, monkeypatch, force_cli):
    """`deconflict` with no subcommand runs the interactive loop (todo: bare mode)."""
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    files_before = {p.name: p.read_bytes() for p in sample_dir.iterdir() if p.is_file()}
    result = runner.invoke(app, [], input="s\n" * 10)
    assert result.exit_code == 0
    # item: final message distinguishes skipped from actually-resolved groups
    assert "resolved 0 group(s) (6 skipped)" in result.stdout
    # skipping must not touch any file
    files_after = {p.name: p.read_bytes() for p in sample_dir.iterdir() if p.is_file()}
    assert files_after == files_before


def test_interactive_quit_prints_summary(sample_dir, tmp_path, monkeypatch, force_cli):
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    result = runner.invoke(app, [], input="q\n")
    assert "aborted after 0 of 6 group(s)" in result.stdout


def test_interactive_tools_prompt(sample_dir, tmp_path, monkeypatch, force_cli):
    """(?)tools prints the tool map, then the loop continues."""
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    result = runner.invoke(app, [], input="?\ns\n" * 10)
    assert result.exit_code == 0
    assert "file kind:" in result.stdout
    assert "tools:" in result.stdout


def test_interactive_table_has_flags(sample_dir, tmp_path, monkeypatch, force_cli):
    """The group table shows #-role, size(+delta), mtime, and metadata (no suggest)."""
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    result = runner.invoke(app, [], input="q\n")
    assert "mtime" in result.stdout
    assert "metadata" in result.stdout
    assert "suggest" not in result.stdout
    assert "vs base" not in result.stdout


def test_bare_no_dirs_explains(tmp_path, monkeypatch):
    """Bare mode without dirs/config exits 2 with guidance (was 'no directories')."""
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(tmp_path / "missing.toml"))
    result = runner.invoke(app, [])
    assert result.exit_code == 2
    assert "no directories" in result.stdout
    assert "init-config" in result.stdout


def test_interactive_metadata_menu_option(sample_dir, tmp_path, monkeypatch, force_cli):
    """First group is the mp3 (metadata differs) so (m)etadata shows a base/copy table."""
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    result = runner.invoke(app, [], input="m\n" + "s\n" * 10)
    assert result.exit_code == 0
    assert "(m)etadata" in result.stdout
    # item: attrs + content are ONE combined table titled "metadata diff"
    assert "metadata diff" in result.stdout
    # common + diff fields both appear as table rows
    assert "artist" in result.stdout
    assert "title" in result.stdout
    # single table also shows size/created/modified file attrs
    assert "created" in result.stdout and "modified" in result.stdout and "size" in result.stdout
    # item: size shows BOTH human-readable and byte count
    assert "bytes" in result.stdout


def test_truncate_meta():
    from deconflict.cli import _truncate_meta

    short = "hello"
    assert _truncate_meta(short) == short
    long = "x" * 100
    out = _truncate_meta(long)
    assert len(out) < len(long)
    assert "+" in out and "more" in out
    assert _truncate_meta(None) == "∅"


def test_interactive_office_metadata_cell_truncated(sample_dir, tmp_path, monkeypatch, force_cli):
    """An office copy's metadata cell shows truncated content, not raw XML."""
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    # reach the xlsx group (group 3): skip 2, then 'm' metadata? no — check overview cell:
    result = runner.invoke(app, [], input="s\ns\ns\ns\ns\ns\nq\n")
    assert result.exit_code == 0
    assert "content:" in result.stdout
    assert "+" in result.stdout and "more" in result.stdout


def test_metadata_header_truncation_keeps_recordside_tail():
    """item: truncating a long metadata-header filename must keep a meaningful
    number of chars from the END — the copy file pattern (date+ext) stays visible."""
    from deconflict.cli import _truncate_name

    # short name passes through untouched
    assert _truncate_name("a.txt", 40) == "a.txt"
    long = "Vorlage_Transkription (conflicted copy 2026-01-08 165356).docx"
    out = _truncate_name(long, 40)
    # total printed width respects the limit (head + "…" + tail)
    assert len(out) <= 40
    # the trailing conflict-pattern chars are preserved
    assert out.endswith("165356).docx")
    # and a leading fragment is also kept, bridged by an ellipsis
    assert out.startswith("Vorla") and "…" in out


def test_metadata_attr_rows_dim_when_identical(sample_dir, tmp_path):
    """item: the size/created/modified attr rows are GRAY (dim) when the two
    files' values are equal, and plain when they differ — matching how content
    rows are dimmed. Deterministic: builds pairs with known equal/differing size."""
    import re
    from io import StringIO

    from rich.console import Console

    import deconflict.cli as cli
    from deconflict.engine import Engine, EngineConfig

    # base + a conflict copy that is byte-identical (size equal -> dim)
    d = tmp_path / "metascan"
    d.mkdir()
    (d / "same.txt").write_text("hello world")
    shutil.copy2(d / "same.txt", d / "same (conflicted copy 2020-01-01 000000).txt")
    # base + a conflict copy with different content (size differs -> plain)
    (d / "diff.txt").write_text("hello world")
    (d / "diff (conflicted copy 2020-01-01 000000).txt").write_text("hello world, and more!")

    engine = Engine(
        EngineConfig(dirs=[d], backup_dir=tmp_path / "backup", cache_dir=tmp_path / "cache")
    )
    analyzed = engine.analyze(engine.scan())
    by_base = {g.base.name: ga for g, ga in analyzed if g.base is not None}

    def render(ga) -> str:
        rec = Console(record=True, file=StringIO(), force_terminal=True, color_system="standard")
        old, cli.console = cli.console, rec
        try:
            cli._print_metadata_diff(ga, 0)
        finally:
            cli.console = old
        return rec.export_text(styles=True)

    def is_dim(line: str) -> bool:
        return "2m" in line or "2;" in line

    def strip_ansi(line: str) -> str:
        return re.sub(r"\x1b\[[0-9;]*m", "", line)

    def size_row(out: str) -> str:
        # the size row is the only one containing "bytes", so it uniquely identifies it
        return next(line for line in out.splitlines() if "bytes" in strip_ansi(line))

    # equal bytes -> size row dimmed
    assert is_dim(size_row(render(by_base["same.txt"])))
    # differing bytes -> size row plain
    assert not is_dim(size_row(render(by_base["diff.txt"])))
