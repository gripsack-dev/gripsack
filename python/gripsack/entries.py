"""Deployment destinations with ownership modes (0001 §3.7)."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


class Ownership(str, Enum):
    """Who owns a deployed file — and what happens when it drifts.

    >>> symlink("~/.config/helix/config.toml").mode
    <Ownership.OWNED: 'owned'>
    """

    #: Store-owned symlink, read-only; edits go through the module.
    OWNED = "owned"
    #: Copied from the store; drift detected on next apply.
    TRACKED_COPY = "tracked_copy"
    #: Managed block merged into a file other tools also write.
    MERGE = "merge"
    #: Rendered at activation from module variables.
    TEMPLATE = "template"


@dataclass(frozen=True)
class Dest:
    """A destination with an ownership mode."""

    to: str
    mode: Ownership

    def __post_init__(self) -> None:
        object.__setattr__(self, "mode", Ownership(self.mode))


def symlink(to: str) -> Dest:
    """Store-owned, read-only; edits go through the module."""
    return Dest(to, Ownership.OWNED)


def tracked_copy(to: str) -> Dest:
    """Copied from the store; drift detected on next apply."""
    return Dest(to, Ownership.TRACKED_COPY)


def merge(to: str) -> Dest:
    """Managed block merged into a file other tools also write."""
    return Dest(to, Ownership.MERGE)


def template(to: str) -> Dest:
    """Rendered at activation from module variables."""
    return Dest(to, Ownership.TEMPLATE)
