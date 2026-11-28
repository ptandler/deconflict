"""Textual TUI frontend for deconflict (reached via bare `deconflict` or `resolve --tui`)."""

from __future__ import annotations

from .app import DeconflictApp

__all__ = ["DeconflictApp"]
