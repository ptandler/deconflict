"""Conflict-filename pattern matching. Add a matcher here + a PATTERNS entry for new formats."""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass

from .paths import casefold

NC_RE = re.compile(r"^(?P<stem>.*?) \(conflicted copy(?P<ts>[^)]*)\)(?P<ext>\.[^.]+)?$")
PACMAN_SUFFIXES = ("pacnew", "pacsave", "pacorig")
SYNCTHING_STAMP = r"[0-9A-Za-z:+_-]+"
SYNCTHING_RE = re.compile(
    rf"^(?P<stem>.*?)\.sync-conflict-(?P<stamp>{SYNCTHING_STAMP})-(?P<id>[A-Za-z0-9]+)(?P<ext>\.[^.]+)?$"
)
DPKG_SUFFIXES = ("dpkg-dist", "dpkg-old", "ucf-dist", "ucf-old", "ucf-new")
RPM_SUFFIXES = ("rpmnew", "rpmorig", "rpmold", "rpmignore")
WINDOWS_COPY_RE = re.compile(r"^(?P<stem>.*?)( - Copy| \(copy\))(?P<ext>\.[^.]+)?$")


@dataclass(frozen=True)
class ConflictMatch:
    """A filename matched as a conflict copy of `base_name`."""

    pattern: str
    base_name: str
    winner: str
    raw: str
    stamp: str | None = None

    @property
    def is_conflict(self) -> bool:
        return casefold(self.base_name) != casefold(self.raw)


@dataclass(frozen=True)
class Pattern:
    name: str
    enabled: bool
    winner: str
    match: Callable[[str], str | None] = None  # filename -> base_name or None
    rx: re.Pattern[str] | None = None

    def detect(self, name: str) -> ConflictMatch | None:
        if self.rx is not None:
            m = self.rx.fullmatch(name)
            if m is None:
                return None
            base = m.group("base")
            if casefold(base) == casefold(name):
                return None
            return ConflictMatch(
                self.name, base, self.winner, name, stamp=m.groupdict().get("stamp")
            )
        base = self.match(name) if self.match else None
        if base is None:
            return None
        if isinstance(base, tuple):  # (base_name, stamp)
            base, stamp = base
            return ConflictMatch(self.name, base, self.winner, name, stamp=stamp)
        return ConflictMatch(self.name, base, self.winner, name)


def _nc(name: str) -> tuple[str, str] | None:
    m = NC_RE.fullmatch(name)
    if m is None:
        return None
    return f"{m.group('stem')}{m.group('ext') or ''}", m.group("ts")


def _pacman(name: str) -> str | None:
    for suffix in PACMAN_SUFFIXES:
        if name.endswith(f".{suffix}"):
            return name[: -(len(suffix) + 1)]
    return None


def _syncthing(name: str) -> tuple[str, str] | None:
    m = SYNCTHING_RE.fullmatch(name)
    if m is None:
        return None
    stem = m.group("stem")
    if m.group("ext"):
        stem += m.group("ext")
    return stem, m.group("stamp")


def _dpkg(name: str) -> str | None:
    for suffix in DPKG_SUFFIXES:
        if name.endswith(f".{suffix}"):
            return name[: -(len(suffix) + 1)]
    return None


def _rpm(name: str) -> str | None:
    for suffix in RPM_SUFFIXES:
        if name.endswith(f".{suffix}"):
            return name[: -(len(suffix) + 1)]
    return None


def _windows_copy(name: str) -> str | None:
    m = WINDOWS_COPY_RE.fullmatch(name)
    if m is None:
        return None
    return f"{m.group('stem')}{m.group('ext') or ''}"


def _custom(rx: re.Pattern[str]) -> Pattern:
    return Pattern("custom", enabled=True, winner="copy", rx=rx)


def builtin_patterns() -> list[Pattern]:
    return [
        Pattern("nextcloud", True, "copy", match=_nc),
        Pattern("pacman", True, "copy", match=_pacman),
        Pattern("syncthing", True, "copy", match=_syncthing),
        Pattern("dpkg", False, "copy", match=_dpkg),
        Pattern("rpm", False, "copy", match=_rpm),
        Pattern("windows-copy", False, "copy", match=_windows_copy),
    ]


def build_patterns(
    enabled: list[str] | None = None,
    custom: list[tuple[str, str, str]] | None = None,
) -> list[Pattern]:
    """Resolve the active pattern set.

    enabled: names of built-in patterns to activate. When None, uses each
             built-in pattern's shipped default (on/off).
    custom: list of (name, base_name_regex, winner). base_name_regex must contain a
            named group 'base' that captures the non-conflict filename, and a
            named group 'stamp' (optional) plus 'winner' is "copy" or "base".
    """
    if enabled is None:
        active = {p.name for p in builtin_patterns() if p.enabled}
    else:
        active = {casefold(e) for e in enabled}
    patterns = [p for p in builtin_patterns() if casefold(p.name) in active]
    for name, rx, winner in custom or []:
        compiled = re.compile(rx)
        if "base" not in compiled.groupindex:
            raise ValueError(
                f"custom pattern '{name}' must define a named group 'base' "
                f"that captures the non-conflict filename"
            )
        patterns.append(Pattern(name, enabled=True, winner=winner, rx=compiled))
    return patterns


def match_name(name: str, patterns: list[Pattern]) -> ConflictMatch | None:
    """Return the first pattern that classifies `name` as a conflict file, else None."""
    for p in patterns:
        if not p.enabled:
            continue
        m = p.detect(name)
        if m is not None:
            return m
    return None
