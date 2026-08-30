"""Analyze tests: content-equality, size/hash, text sniff, mtime winner."""

from __future__ import annotations

from deconflict.analyze import analyze_group
from deconflict.patterns import build_patterns
from deconflict.scan import scan

PATTERNS = build_patterns()


def _analyze_readme(sample_dir):
    result = scan([sample_dir], PATTERNS)
    g = next(g for g in result.groups if g.base.name == "Readme.md")
    return g, analyze_group(g, g.base, g.conflicts)


def test_copies_differ(sample_dir):
    _g, ga = _analyze_readme(sample_dir)
    assert ga.all_equal is False
    assert len(set(ga.hashes.values())) == 2


def test_identical_copies_are_equal(tmp_path):
    (tmp_path / "a.txt").write_text("same\n")
    (tmp_path / "a (conflicted copy 2020-01-01 000000).txt").write_text("same\n")
    result = scan([tmp_path], PATTERNS)
    g = next(g for g in result.groups if g.base.name == "a.txt")
    ga = analyze_group(g, g.base, g.conflicts)
    assert ga.all_equal is True


def test_media_files_are_not_text(sample_dir):
    result = scan([sample_dir], PATTERNS)
    img = next(g for g in result.groups if g.base.name.startswith("IMG"))
    ga = analyze_group(img, img.base, img.conflicts)
    assert ga.all_text is False


def test_markdown_is_text(sample_dir):
    _g, ga = _analyze_readme(sample_dir)
    assert ga.all_text is True


def test_sizes_and_hashes_populated(sample_dir):
    _g, ga = _analyze_readme(sample_dir)
    assert len(ga.sizes) == 2
    assert all(len(h) == 64 for h in ga.hashes.values())
