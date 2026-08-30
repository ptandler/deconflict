"""Engine: UI-free domain logic reusable by any frontend (CLI, TUI, GUI).

The engine owns the entities and the filesystem effects. Frontends own the
interaction loop: they present a group, ask for an Action, and either call
`apply()` (final decision) or `launch_tool()` (run an external tool, then re-ask).
"""

from __future__ import annotations

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


@dataclass
class EngineConfig:
    dirs: list[Path] = field(default_factory=list)
    enabled_patterns: list[str] = field(
        default_factory=lambda: ["nextcloud", "pacman", "syncthing"]
    )
    custom_patterns: list[tuple[str, str, str]] = field(default_factory=list)
    backup_dir: str | Path = "~/deconflict-backups"
    cache_dir: str | Path = "~/.cache/deconflict"
    dry_run: bool = False
    follow_symlinks: bool = False


@dataclass
class Engine:
    cfg: EngineConfig

    def __post_init__(self) -> None:
        self.patterns = build_patterns(self.cfg.enabled_patterns, self.cfg.custom_patterns)
        self.resolver = Resolver(
            Path(str(self.cfg.backup_dir)).expanduser(), dry_run=self.cfg.dry_run
        )
        self.tools: Tools = detect()
        self.cache: FileCache | None = None
        if self.cfg.cache_dir and self.cfg.dirs:
            self.cache = FileCache(self.cfg.cache_dir, cache_key(self.cfg.dirs))

    def scan(self) -> ScanResult:
        result = scan(self.cfg.dirs, self.patterns, self.cfg.follow_symlinks)
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
        return [(g, analyze_group(g, g.base, g.conflicts)) for g in result.groups]

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
            self.resolver.keep_both(label, group.base, group.conflicts, log)
            return []
        return []

    @staticmethod
    def _label(group: ConflictGroup) -> str:
        """Short, filesystem-safe subdir label for a group's backups."""
        return group.base.name if group.base else group.key

    def launch_tool(
        self, group: ConflictGroup, ga: GroupAnalysis, tool: ToolType, target: Path
    ) -> None:
        """Run an external tool on `target` and wait for it to exit (blocking)."""
        launcher = Launcher(self.tools)
        launcher.run(group, ga, tool, target)

    def suggest(self, path: Path) -> ToolType:
        """The tool a frontend should suggest for `path`, based on its type."""
        return Launcher(self.tools).suggested(path)


def path_of(a) -> Path:
    return a.path
