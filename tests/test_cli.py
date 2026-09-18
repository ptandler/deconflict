"""CLI tests via Typer's CliRunner (no interactive stdin; use --auto --dry-run)."""

from __future__ import annotations

import shutil
from pathlib import Path

from testdata import EXPECTED_GROUPS
from typer.testing import CliRunner

from deconflict import __version__
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


def test_resolve_auto_recommended(tmp_path):
    """G5: --auto recommended resolves only clear-winner groups and prints a skip hint."""
    d = tmp_path / "scan"
    d.mkdir()
    (d / "same.txt").write_text("hello")
    (d / "same (conflicted copy 2020-01-01 000000).txt").write_text("hello")
    # copy drops a line (not a superset) => no recommendation -> skipped
    (d / "navy.txt").write_text("a\nb\n")
    (d / "navy (conflicted copy 2020-01-01 000000).txt").write_text("b\n")
    result = runner.invoke(app, ["resolve", str(d), "--auto", "recommended", "--dry-run"])
    assert result.exit_code == 0
    assert "resolved 1 group(s) (1 skipped)" in result.stdout
    assert "hint: re-run" in result.stdout


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
    assert len(groups) == EXPECTED_GROUPS


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
    assert "resolved 0 group(s) (3 skipped)" in result.stdout


def test_main_bare_with_dir_launches_tui(sample_dir, monkeypatch, capsys):
    """`deconflict <dir>` (entry point) runs the bare driver against that dir."""
    import sys

    import deconflict.cli as cli_mod

    calls: list[tuple] = []
    monkeypatch.setattr(sys, "argv", ["deconflict", str(sample_dir)])
    monkeypatch.setattr(cli_mod, "_tui_available", lambda: True)
    monkeypatch.setattr(
        cli_mod, "_launch_tui", lambda engine, groups: calls.append((engine, groups))
    )
    cli_mod.main()
    assert len(calls) == 1
    engine, groups = calls[0]
    assert len(groups) == EXPECTED_GROUPS


def test_main_subcommand_still_dispatches(sample_dir, monkeypatch, capsys):
    """`main()` leaves known-subcommand invocations to Typer (no interception)."""
    import sys

    import pytest

    import deconflict.cli as cli_mod

    monkeypatch.setattr(
        sys, "argv", ["deconflict", "resolve", str(sample_dir), "--auto", "newest", "--dry-run"]
    )
    with pytest.raises(SystemExit) as exc:
        cli_mod.main()
    assert exc.value.code == 0
    out = capsys.readouterr().out
    assert "dry-run" in out


def test_main_bare_config_flag(sample_dir, tmp_path, monkeypatch):
    """`deconflict --config <f> <dir>` via main() honours the config + dir."""
    import sys

    import deconflict.cli as cli_mod

    cfgfile = tmp_path / "alt.toml"
    cfgfile.write_text(f'[scan]\ndefault_dirs=["{sample_dir}"]\n')
    calls: list[tuple] = []
    monkeypatch.setattr(sys, "argv", ["deconflict", "--config", str(cfgfile)])
    monkeypatch.setattr(cli_mod, "_tui_available", lambda: True)
    monkeypatch.setattr(
        cli_mod, "_launch_tui", lambda engine, groups: calls.append((engine, groups))
    )
    cli_mod.main()
    assert len(calls) == 1
    engine, groups = calls[0]
    assert len(groups) == EXPECTED_GROUPS


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
    assert "resolved 0 group(s) (3 skipped)" in result.stdout
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
    """The primary per-group view shows the metadata-diff table + full paths
    (no overview/#-role/suggest columns)."""
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    result = runner.invoke(app, [], input="q\n")
    # metadata-diff table is the primary view, not the old #/size/mtime/metadata overview
    assert "metadata diff" in result.stdout
    assert "full paths" in result.stdout
    assert "#0" in result.stdout
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
    """First group is the mp3 (metadata differs); the metadata-diff table is SHOWN
    automatically as the primary view (the old (m)etadata action was removed)."""
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    result = runner.invoke(app, [], input="s\n" * 10)
    assert result.exit_code == 0
    # (m)etadata action no longer offered — table is now auto-shown
    assert "(m)etadata" not in result.stdout
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
    """An office copy's metadata table shows truncated field values, not raw XML/reports."""
    import deconflict.config as config_mod

    monkeypatch.setattr(config_mod, "DEFAULT_CFG_PATH", Path(_home_cfg(tmp_path, sample_dir)))
    # reach the xlsx group (group 3): skip 2, then skip the rest, quit
    result = runner.invoke(app, [], input="s\ns\ns\ns\ns\ns\nq\n")
    assert result.exit_code == 0
    assert "metadata diff" in result.stdout
    # long office field values are truncated with the …(+N more) marker
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


def test_version_flag():
    result = runner.invoke(app, ["--version"])
    assert result.exit_code == 0
    assert result.stdout.strip() == f"deconflict {__version__}"


def test_version_command():
    result = runner.invoke(app, ["version"])
    assert result.exit_code == 0
    assert result.stdout.strip() == f"deconflict {__version__}"


def test_metadata_diff_highlights_newer_and_larger(tmp_path):
    """G2: metadata diff table bolds the larger size cell and bold-cyans the
    newer modified-date cell. Deterministic via os.utime."""
    import re
    from io import StringIO

    from rich.console import Console

    import deconflict.cli as cli
    from deconflict.engine import Engine, EngineConfig

    d = tmp_path / "hl"
    d.mkdir()
    base = d / "report.txt"
    copy = d / "report (conflicted copy 2020-01-01 000000).txt"
    base.write_text("x" * 100)
    copy.write_text("x" * 500)  # larger AND newer
    import os

    os.utime(base, (1_000_000, 1_000_000))
    os.utime(copy, (2_000_000, 2_000_000))

    engine = Engine(
        EngineConfig(dirs=[d], backup_dir=tmp_path / "backup", cache_dir=tmp_path / "cache")
    )
    analyzed = engine.analyze(engine.scan())
    ga = next(ga for g, ga in analyzed if g.base is not None and g.base.name == "report.txt")

    rec = Console(record=True, file=StringIO(), force_terminal=True, color_system="standard")
    old, cli.console = cli.console, rec
    try:
        cli._print_metadata_diff(ga, 0)
    finally:
        cli.console = old
    out = rec.export_text(styles=True)

    def strip_ansi(line: str) -> str:
        return re.sub(r"\x1b\[[0-9;]*m", "", line)

    size_line = next(line for line in out.splitlines() if "bytes" in strip_ansi(line))
    assert "1m" in size_line  # bold -> larger copy highlighted
    mod_line = next(line for line in out.splitlines() if "modified" in strip_ansi(line))
    assert "1;36" in mod_line  # bold cyan -> newer copy highlighted


def test_scan_shows_full_paths(sample_dir):
    """G1: scan table must print full file paths (not just basenames)."""
    result = runner.invoke(app, ["scan", str(sample_dir), "--no-progress"])
    assert result.exit_code == 0
    resolved = str(sample_dir.resolve())
    assert resolved in result.stdout
    # basename-only output would not contain the parent dir
    assert "conflict group(s)" in result.stdout


def test_scan_progress_flag_forced(sample_dir):
    """G1: --progress forces the progress bar even when stdout is not a TTY."""
    result = runner.invoke(app, ["scan", str(sample_dir), "--progress"])
    assert result.exit_code == 0
    assert "conflict group(s)" in result.stdout


def test_setup_command():
    """G6: `deconflict setup` analyzes the local setup and prints status + next steps."""
    result = runner.invoke(app, ["setup"])
    assert result.exit_code == 0
    assert "deconflict" in result.stdout
    assert "external tools" in result.stdout
    assert "next steps" in result.stdout
    assert "init-config" in result.stdout


def test_resolve_prints_stage_timing(sample_dir):
    """G7: resolve prints stage timing (scan/analyze) so a slow startup is attributable."""
    result = runner.invoke(app, ["resolve", str(sample_dir), "--dry-run"], input="q\n")
    assert result.exit_code == 0
    assert "scan " in result.stdout
    assert ", analyze " in result.stdout
    assert "total" in result.stdout


def _rec_group(tmp_path: str, base_text: str, copy_text: str, *, copy_newer: bool = True):
    """Build a scan dir with one text group; return (engine, GroupAnalysis)
    with deterministic mtimes."""
    import os

    from deconflict.engine import Engine, EngineConfig

    d = Path(tmp_path) / "scan"
    d.mkdir()
    base = d / "doc.txt"
    copy = d / "doc (conflicted copy 2020-01-01 000000).txt"
    base.write_text(base_text)
    copy.write_text(copy_text)
    os.utime(base, (1_000_000, 1_000_000))
    if copy_newer:
        os.utime(copy, (2_000_000, 2_000_000))
    engine = Engine(
        EngineConfig(
            dirs=[d], backup_dir=Path(tmp_path) / "backup", cache_dir=Path(tmp_path) / "cache"
        )
    )
    analyzed = engine.analyze(engine.scan())
    ga = next(ga for g, ga in analyzed if g.base is not None)
    return engine, ga


def test_recommend_all_identical_keeps_base(tmp_path):
    """G3: when all files are identical in content, recommend keep base."""
    from deconflict.cli import _recommend

    _engine, ga = _rec_group(tmp_path, "same", "same")
    rec = _recommend(ga)
    assert rec is not None
    label, action = rec
    assert action.kind.value == "base"
    assert "identical" in label


def test_recommend_strictly_newer_copy(tmp_path):
    """G3: when the copy is strictly newer AND text-superset, recommend keep that copy."""
    from deconflict.cli import _recommend

    _engine, ga = _rec_group(tmp_path, "line1\nline2", "line1\nline2\nline3\n")
    rec = _recommend(ga)
    assert rec is not None
    label, action = rec
    assert action.kind.value == "copy"
    assert "copy" in label


def test_recommend_no_clear_winner(tmp_path):
    """G3: no recommendation when the copy is strictly-newer but REMOVES content."""
    from deconflict.cli import _recommend

    # newest copy drops a line (not a superset) => no clear winner
    _engine, ga = _rec_group(tmp_path, "line1\nline2", "line2")
    assert _recommend(ga) is None


def test_recommend_whitespace_only_base_is_superseded(tmp_path):
    """Recommend the copy when the base is effectively empty (blank/whitespace only)."""
    from deconflict.analyze import recommend

    _engine, ga = _rec_group(tmp_path, "   \n", "line1 adds real content\n")
    rec = recommend(ga)
    assert rec is not None
    assert rec.is_base is False


def test_recommend_newer_but_smaller_audio_is_rejected(tmp_path):
    """G2 (09-02): a newer binary/media copy that is strictly SMALLER is NOT
    recommended — possible metadata/content loss (e.g. the Fröhlicher Kreis mp3)."""
    import os

    from deconflict.analyze import recommend
    from deconflict.engine import Engine, EngineConfig

    d = Path(tmp_path) / "scan"
    d.mkdir()
    base = d / "song.mp3"
    copy = d / "song (conflicted copy 2020-01-01 000000).mp3"
    # binary-ish content so kind != text; base bigger than copy
    base.write_bytes(b"x" * 5000)
    copy.write_bytes(b"y" * 1000)
    os.utime(base, (1_000_000, 1_000_000))
    os.utime(copy, (2_000_000, 2_000_000))
    engine = Engine(
        EngineConfig(
            dirs=[d], backup_dir=Path(tmp_path) / "backup", cache_dir=Path(tmp_path) / "cache"
        )
    )
    ga = next(ga for g, ga in engine.analyze(engine.scan()) if g.base is not None)
    assert ga.kind != "text"
    assert recommend(ga) is None  # newer but smaller -> not recommended


def test_recommend_newer_and_larger_keeps_office(tmp_path):
    """G2 (09-02): a newer, LARGER copy is still recommended (Kostenauflistung case)."""
    import os

    from deconflict.analyze import recommend
    from deconflict.engine import Engine, EngineConfig

    d = Path(tmp_path) / "scan"
    d.mkdir()
    base = d / "tabelle.xlsx"
    copy = d / "tabelle (conflicted copy 2020-01-01 000000).xlsx"
    base.write_bytes(b"x" * 1000)
    copy.write_bytes(b"y" * 2000)
    os.utime(base, (1_000_000, 1_000_000))
    os.utime(copy, (2_000_000, 2_000_000))
    engine = Engine(
        EngineConfig(
            dirs=[d], backup_dir=Path(tmp_path) / "backup", cache_dir=Path(tmp_path) / "cache"
        )
    )
    ga = next(ga for g, ga in engine.analyze(engine.scan()) if g.base is not None)
    rec = recommend(ga)
    assert rec is not None
    assert rec.is_base is False


def test_consider_reports_reasoning_when_no_clear_winner(tmp_path):
    """consider() logs newest/largest + why no recommendation, even on a no-winner group."""
    import os

    from deconflict.analyze import consider
    from deconflict.engine import Engine, EngineConfig

    d = Path(tmp_path) / "scan"
    d.mkdir()
    base = d / "song.mp3"
    copy = d / "song (conflicted copy 2020-01-01 000000).mp3"
    base.write_bytes(b"x" * 5000)
    copy.write_bytes(b"y" * 1000)  # newer but smaller
    os.utime(base, (1_000_000, 1_000_000))
    os.utime(copy, (2_000_000, 2_000_000))
    engine = Engine(
        EngineConfig(
            dirs=[d], backup_dir=Path(tmp_path) / "backup", cache_dir=Path(tmp_path) / "cache"
        )
    )
    ga = next(ga for g, ga in engine.analyze(engine.scan()) if g.base is not None)
    cons = consider(ga)
    assert cons.recommendation is None
    joined = " ".join(cons.notes)
    assert "newest is" in joined
    assert "largest is" in joined
    assert "smaller than base" in joined  # explicit reason why no recommendation
    assert "no clear recommendation" in joined


def test_recommend_enter_picks_default(tmp_path):
    """G3: the menu surfaces the recommendation as an Enter default, and empty input picks it."""
    from unittest.mock import patch

    import deconflict.cli as cli

    engine, ga = _rec_group(tmp_path, "same", "same")
    rec = cli._recommend(ga)
    assert rec is not None

    # menu text advertises the Enter default
    menu = cli._menu_text(engine, ga, rec)
    assert "(Enter) keep" in menu

    # a bare Enter in the prompt returns the recommended action (empty branch
    # returns `rec[1]` before the (unused here) group is touched)
    with patch("builtins.input", return_value=""):
        out = cli._prompt(engine, None, ga, rec)
    assert out is rec[1]
