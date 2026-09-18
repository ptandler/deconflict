"""Scan tests against tmp copies of the real sample fixtures."""

from __future__ import annotations

from testdata import (
    EXPECTED_GROUPS,
    EXPECTED_NEXTCLOUD_GROUPS,
    NEXTCLOUD_BASE_NAMES,
)

from deconflict.patterns import build_patterns
from deconflict.scan import group_conflicts, scan

PATTERNS = build_patterns()


def test_scan_finds_all_nextcloud_groups(sample_dir):
    result = scan([sample_dir], PATTERNS)
    names = sorted(g.base.name for g in result.groups if g.pattern == "nextcloud")
    assert len([g for g in result.groups if g.pattern == "nextcloud"]) == EXPECTED_NEXTCLOUD_GROUPS
    for name in NEXTCLOUD_BASE_NAMES:
        assert name in names


def test_group_has_one_conflict_each(sample_dir):
    result = scan([sample_dir], PATTERNS)
    for g in result.groups:
        assert len(g.conflicts) == 1


def test_group_files_include_base_first(sample_dir):
    result = scan([sample_dir], PATTERNS)
    g = next(g for g in result.groups if g.base.name == "Readme.md")
    assert g.files[0] == g.base
    assert len(g.files) == 2


def test_group_conflicts(sample_dir):
    all_files = list(sample_dir.rglob("*"))
    groups = group_conflicts(all_files, PATTERNS)
    assert len(groups) == EXPECTED_GROUPS


def test_no_conflicts_in_empty_dir(tmp_path):
    (tmp_path / "Readme.md").write_text("x")
    result = scan([tmp_path], PATTERNS)
    assert not result
