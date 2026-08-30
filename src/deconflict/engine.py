"""Engine: UI-free domain logic reusable by any frontend (CLI, TUI, GUI).

The engine owns the entities and the filesystem effects. Frontends own the
interaction loop: they present a group, ask for an Action, and either call
`apply()` (final decision) or `launch_tool()` (run an external tool, then re-ask).
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path

from .analyze import GroupAnalysis, analyze_group
from .cache import FileCache, cache_key
from .launchers import Launcher, ToolType
from .patterns import build_patterns
from .resolve import Resolved, Resolver
from .scan import ConflictGroup, ScanResult, scan
from .tools import Tools, detect


class ActionKind(str, Enum):
    KEEP_BASE = "base"
    KEEP_COPY = "copy"
    KEEP_BOTH = "both"
    SKIP = "skip"
    TOOL = "tool"


@dataclass(frozen=True)
class Action:
    """The resolution vocabulary. `target` is the winning file for KEEP_COPY or the
    file a TOOL should operate on; `tool` names the external tool for TOOL."""

    kind: ActionKind
    target: Path | None = None
    tool: ToolType | None = None

    @classmethod
    def keep_base(cls) -> Action:
        return cls(ActionKind.KEEP_BASE)

    @classmethod
    def keep_copy(cls, target: Path) -> Action:
        return cls(ActionKind.KEEP_COPY, target=target)

    @classmethod
    def keep_both(cls) -> Action:
        return cls(ActionKind.KEEP_BOTH)

    @classmethod
    def skip(cls) -> Action:
        return cls(ActionKind.SKIP)

    @classmethod
    def tool_action(cls, tool_type: ToolType, target: Path) -> Action:
        return cls(ActionKind.TOOL, target=target, tool=tool_type)


def _default_cache_dir() -> str:
    """XDG_CACHE_HOME (or ~/.cache) based default cache root, as a str path."""
    return str(Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "deconflict")


@dataclass
class EngineConfig:
    dirs: list[Path] = field(default_factory=list)
    enabled_patterns: list[str] = field(
        default_factory=lambda: ["nextcloud", "pacman", "syncthing"]
    )
    custom_patterns: list[tuple[str, str, str]] = field(default_factory=list)
    backup_dir: str | Path = "~/deconflict-backups"
    cache_dir: str | Path = field(default_factory=_default_cache_dir)
    dry_run: bool = False
    follow_symlinks: bool = False
    file_type_tools: dict[str, str] = field(default_factory=dict)  # kind -> [tools] view key
    file_type_edit_tools: dict[str, str] = field(default_factory=dict)  # kind -> [tools] edit key
    config_path: Path | None = None  # effective config file (None = built-in defaults)


@dataclass
class Engine:
    cfg: EngineConfig
    tools: Tools | None = None  # inject for hermetic tests; else auto-detect

    def __post_init__(self) -> None:
        self.patterns = build_patterns(self.cfg.enabled_patterns, self.cfg.custom_patterns)
        self.resolver = Resolver(
            Path(str(self.cfg.backup_dir)).expanduser(), dry_run=self.cfg.dry_run
        )
        self.tools: Tools = self.tools or detect()
        self.cache: FileCache | None = None
        self.cache_used = False  # True when this scan reused a cached result
        if self.cfg.cache_dir and self.cfg.dirs:
            self.cache = FileCache(self.cfg.cache_dir, cache_key(self.cfg.dirs))
            self.cache_used = self.cache.path.exists()

    def scan(self, on_file=None) -> ScanResult:
        result = scan(self.cfg.dirs, self.patterns, self.cfg.follow_symlinks, on_file=on_file)
        if self.cache is not None and self.cfg.dirs:
            self._refresh_cache(result)
        return result

    def _refresh_cache(self, result: ScanResult) -> None:
        from .analyze import _info

        touched = False
        for group in result.groups:
            for path in group.files:
                try:
                    st = path.stat()
                except OSError:
                    continue
                if self.cache.get(path, st.st_size, st.st_mtime) is None:
                    info = _info(path)
                    self.cache.put(path, st.st_size, st.st_mtime, info.sha, info.is_text)
                    touched = True
        if touched:
            self.cache.save()

    def analyze(self, result: ScanResult) -> list[tuple[ConflictGroup, GroupAnalysis]]:
        return [
            (g, analyze_group(g, g.base, g.conflicts, tools=self.tools.found))
            for g in result.groups
        ]

    def auto_choice(self, ga: GroupAnalysis, rule: str) -> Action:
        """Map a --auto rule (base|copy|newest|bigger) to an Action."""
        items = [a for a in ([ga.base] if ga.base else []) + ga.copies if a]
        if rule == "base":
            return Action.keep_base()
        if rule == "copy":
            return self._copy_or_skip(items)
        if rule == "newest" and items:
            winner = max(items, key=lambda a: a.info.mtime)
            return self._for_winner(ga, winner)
        if rule == "bigger" and items:
            winner = max(items, key=lambda a: a.info.size)
            return self._for_winner(ga, winner)
        return Action.skip()

    @staticmethod
    def _copy_or_skip(items) -> Action:
        if not items:
            return Action.skip()
        return Action.keep_copy(path_of(items[0]))

    @staticmethod
    def _for_winner(ga: GroupAnalysis, winner) -> Action:
        if ga.base and winner is ga.base:
            return Action.keep_base()
        return Action.keep_copy(winner.path)

    def apply(self, group: ConflictGroup, ga: GroupAnalysis, action: Action) -> list[Resolved]:
        """Apply a final-decision Action, producing filesystem effects. TOOL/SKIP by caller."""
        log = self.resolver.backup_root / "decision-log.jsonl"
        label = self._label(group)
        if action.kind is ActionKind.KEEP_BASE:
            if group.base is None:
                return []
            return self.resolver.keep_base(label, group.base, group.conflicts, log)
        if action.kind is ActionKind.KEEP_COPY:
            chosen = action.target or (group.conflicts[0] if group.conflicts else None)
            if chosen is None:
                return []
            others = [c for c in group.conflicts if c.resolve() != chosen.resolve()]
            return self.resolver.keep_copy(label, group.base, chosen, others, log)
        if action.kind is ActionKind.KEEP_BOTH:
            return self.resolver.keep_both(label, group.base, group.conflicts, log)
        return []

    @staticmethod
    def _label(group: ConflictGroup) -> str:
        """Short, filesystem-safe subdir label for a group's backups."""
        return group.base.name if group.base else group.key

    def launcher(self) -> Launcher:
        return Launcher(self.tools, self.cfg.file_type_tools, self.cfg.file_type_edit_tools)

    def launch_tool(
        self, group: ConflictGroup, ga: GroupAnalysis, tool: ToolType, target: Path
    ) -> None:
        """Run an external tool on `target` and wait for it to exit (blocking)."""
        self.launcher().run(group, ga, tool, target)

    def suggest(self, path: Path) -> ToolType:
        """The tool a frontend should suggest for `path`, based on its type."""
        return self.launcher().suggested(path)

    def available_view(self, ga: GroupAnalysis) -> bool:
        return self.launcher().view_available(ga.kind)

    def available_edit(self, ga: GroupAnalysis) -> bool:
        return self.launcher().edit_available(ga.kind)

    def available_diff(self, ga: GroupAnalysis) -> bool:
        return self.launcher().diff_available(ga.kind)

    def available_edit_meta(self, ga: GroupAnalysis) -> bool:
        return self.launcher().edit_meta_available(ga.kind)

    def supported_tools(self, ga: GroupAnalysis) -> list[ToolType]:
        """Tool-action letters available for this group's file kind."""
        kind = ga.kind
        tools: list[ToolType] = [ToolType.VIEW]
        if self.available_edit(ga) and kind not in ("text", "other", "office", "kdbx"):
            tools.append(ToolType.EDIT)
        if self.available_diff(ga):
            tools.append(ToolType.DIFF)
        # (m)etadata is always offered: the generic file attributes (size/created/modified)
        # are extracted for every kind, so there is always something to show.
        tools.append(ToolType.VIEW_META)
        if self.available_edit_meta(ga):
            tools.append(ToolType.EDIT_META)
        return tools

    def tool_status(self) -> list[tuple[str, str | None, list[str], str | None]]:
        """(name, found-path, candidates, install-hint) for every configured tool."""
        return [
            (n, self.tools.get(n), self.tools.candidates(n), self.tools.hint(n))
            for n in self.tools.names()
        ]


def path_of(a) -> Path:
    return a.path
