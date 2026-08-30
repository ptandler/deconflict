"""Pattern matching tests, using only synthetic filenames (no fixtures needed)."""

from __future__ import annotations

import pytest

from deconflict.patterns import build_patterns, match_name


def test_nextcloud_match():
    m = match_name("Readme (conflicted copy 2024-12-01 213224).md", build_patterns())
    assert m is not None
    assert m.pattern == "nextcloud"
    assert m.base_name == "Readme.md"
    assert m.stamp == "2024-12-01 213224"


def test_nextcloud_no_ext():
    m = match_name("config (conflicted copy 2020-01-01 000000)", build_patterns())
    assert m is not None
    assert m.base_name == "config"


def test_base_is_not_conflict():
    assert match_name("Readme.md", build_patterns()) is None


def test_pacman():
    for name, base in [
        ("nginx.conf.pacnew", "nginx.conf"),
        ("resolv.conf.pacsave", "resolv.conf"),
        ("foo.pacorig", "foo"),
    ]:
        m = match_name(name, build_patterns(["pacman"]))
        assert m is not None and m.base_name == base, name


def test_pacman_disabled_by_default_setting():
    pats = build_patterns(enabled=["nextcloud"])
    assert match_name("x.pacnew", pats) is None


def test_syncthing():
    cases = {
        "file.txt.sync-conflict-20240427-150142-JKHLKL": "file.txt",
        "file.sync-conflict-20240427-150142-JKHLKL.txt": "file.txt",
    }
    for name, base in cases.items():
        m = match_name(name, build_patterns(["syncthing"]))
        assert m is not None and m.base_name == base, name


def test_windows_copy_disabled():
    pats = build_patterns()
    assert match_name("foo - Copy.txt", pats) is None
    assert match_name("foo (copy).txt", pats) is None


def test_custom_pattern():
    pats = build_patterns(custom=[("backupbak", r"^(?P<base>.+)\.bak$", "copy")])
    m = match_name("config.toml.bak", pats)
    assert m is not None
    assert m.base_name == "config.toml"


def test_custom_regex_requires_named_base_group():
    with pytest.raises(ValueError):
        build_patterns(custom=[("bad", r"^(.+)\.bak$", "copy")])
