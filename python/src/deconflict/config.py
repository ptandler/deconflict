"""Configuration load (TOML) and init-config template handling."""

from __future__ import annotations

import importlib.resources
import os
import warnings
from dataclasses import dataclass, field
from pathlib import Path

from .engine import EngineConfig

_TEMPLATE_NAME = "config.example.toml"  # shipped inside the package (init-config source)


@dataclass
class Config:
    engine: EngineConfig = field(default_factory=EngineConfig)
    tools: dict[str, str] = field(default_factory=dict)
    source: Path | None = None  # config file this was loaded from (None = defaults)

    @property
    def custom_patterns(self) -> list[tuple[str, str, str]]:
        return self.engine.custom_patterns


def _xdg_config_dir() -> Path:
    """XDG_CONFIG_HOME (or ~/.config) for the per-user config file."""
    return Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))


def _xdg_cache_dir() -> Path:
    """XDG_CACHE_HOME (or ~/.cache) as the default cache root."""
    return Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))


DEFAULT_CFG_PATH = _xdg_config_dir() / "deconflict" / "config.toml"
DEFAULT_CACHE_DIR = str(_xdg_cache_dir() / "deconflict")


def default_path() -> Path:
    return DEFAULT_CFG_PATH


def load(path: Path | None = None) -> Config:
    """Load config from TOML, falling back to defaults.

    Never raises for a missing file. If the file exists but can't be read or
    parsed, warns (so a typo doesn't silently discard every setting) and falls
    back to defaults.
    """
    path = (Path(path) if path else DEFAULT_CFG_PATH).expanduser()
    cfg = Config(source=path if path.exists() else None)
    if not path.exists():
        cfg.engine.config_path = None
        return cfg
    try:
        import tomllib

        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        warnings.warn(
            f"could not parse config {path}: {exc}. Using defaults instead.",
            UserWarning,
            stacklevel=2,
        )
        return cfg
    cfg = _from_dict(raw)
    cfg.source = path
    cfg.engine.config_path = path
    return cfg


def _from_dict(raw: dict) -> Config:
    scan = raw.get("scan", {})
    gen = raw.get("general", {})
    custom = [
        (p["name"], p["base_name"], p.get("winner", "copy"))
        for p in scan.get("patterns", [])
        if isinstance(p, dict) and "name" in p and "base_name" in p
    ]
    engine = EngineConfig(
        dirs=[Path(d) for d in scan.get("default_dirs", [])],
        enabled_patterns=list(scan.get("enabled_patterns", ["nextcloud", "pacman", "syncthing"])),
        custom_patterns=custom,
        backup_dir=gen.get("backup_dir", "~/deconflict-backups"),
        cache_dir=gen.get("cache_dir", DEFAULT_CACHE_DIR),
        file_type_tools=dict(raw.get("file_types", {})),
        file_type_edit_tools=dict(raw.get("file_types_edit", {})),
    )
    return Config(engine=engine, tools=dict(raw.get("tools", {})))


def write_init_config(path: Path | None = None) -> Path:
    """Write the packaged config template to `path` (default user config).

    The template ships as package data (`deconflict/config.example.toml`), so
    this works from an installed wheel as well as a source checkout. Refuses to
    overwrite an existing file.
    """
    path = path or DEFAULT_CFG_PATH
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise FileExistsError(path)
    template = importlib.resources.files(__package__ or "deconflict") / _TEMPLATE_NAME
    path.write_text(template.read_text(encoding="utf-8"), encoding="utf-8")
    return path
