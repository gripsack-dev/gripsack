"""Module dependency edges (0001 §3.1)."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


class Edge(str, Enum):
    """Dependency edge kind.

    ``Edge.BUILD`` marks an ephemeral, build-only dependency — present
    while building, referenced by no generation, GC'd afterward:

    >>> dep("rust", edge=Edge.BUILD)
    Dependency(module='rust', edge=<Edge.BUILD: 'build'>)

    Plain strings are coerced — and typos rejected:

    >>> dep("rust", edge="buld")
    Traceback (most recent call last):
        ...
    ValueError: 'buld' is not a valid Edge
    """

    RUNTIME = "runtime"
    BUILD = "build"


@dataclass(frozen=True)
class Dependency:
    module: str
    edge: Edge = Edge.RUNTIME

    def __post_init__(self) -> None:
        # Coerce plain strings (raises ValueError on typos) so both
        # dep("x", edge=Edge.BUILD) and dep("x", edge="build") are safe.
        object.__setattr__(self, "edge", Edge(self.edge))


def dep(module: str, edge: Edge = Edge.RUNTIME) -> Dependency:
    """Declare a dependency on another module.

    >>> dep("git")
    Dependency(module='git', edge=<Edge.RUNTIME: 'runtime'>)
    """
    return Dependency(module, edge)
