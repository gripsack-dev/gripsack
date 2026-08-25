"""Deployment destinations with ownership modes (0001 §3.7)."""

from __future__ import annotations

from dataclasses import dataclass, field
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
    #: Template variables (mode ``template`` only) — ``{{ name }}`` in
    #: the payload is substituted at deploy time. Compute per-host
    #: values at eval time with :func:`gripsack.facts`.
    vars: dict[str, str] = field(default_factory=dict)
    #: Comment prefix for the managed block (mode ``merge`` only);
    #: ``None`` infers it from the destination extension.
    marker: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "mode", Ownership(self.mode))


def symlink(to: str) -> Dest:
    """Store-owned, read-only; edits go through the module."""
    return Dest(to, Ownership.OWNED)


def tracked_copy(to: str) -> Dest:
    """Copied from the store; drift detected on next apply."""
    return Dest(to, Ownership.TRACKED_COPY)


def merge(to: str, marker: str | None = None) -> Dest:
    """Managed block merged into a file other tools also write.

    >>> merge("~/.bashrc").mode
    <Ownership.MERGE: 'merge'>
    >>> merge("~/.config/x.jsonc", marker="//").marker
    '//'
    """
    return Dest(to, Ownership.MERGE, marker=marker)


def template(to: str, vars: dict[str, str] | None = None) -> Dest:
    """Rendered at deploy time from ``{{ name }}`` placeholders.

    >>> template("~/.config/git/id", vars={"email": "a@b.c"}).vars
    {'email': 'a@b.c'}
    """
    return Dest(to, Ownership.TEMPLATE, vars=dict(vars or {}))
