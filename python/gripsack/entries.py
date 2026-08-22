"""Deployment destinations with ownership modes (0001 §3.7)."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Dest:
    """A destination with an ownership mode."""

    to: str
    mode: str


def symlink(to: str) -> Dest:
    """Store-owned, read-only; edits go through the module."""
    return Dest(to, "owned")


def tracked_copy(to: str) -> Dest:
    """Copied from the store; drift detected on next apply."""
    return Dest(to, "tracked_copy")


def merge(to: str) -> Dest:
    """Managed block merged into a file other tools also write."""
    return Dest(to, "merge")


def template(to: str) -> Dest:
    """Rendered at activation from module variables."""
    return Dest(to, "template")
