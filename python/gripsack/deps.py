"""Module dependency edges (0001 §3.1)."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Dependency:
    module: str
    edge: str = "runtime"  # "build" = ephemeral, build-only


def dep(module: str, edge: str = "runtime") -> Dependency:
    if edge not in ("runtime", "build"):
        raise ValueError(f"edge must be 'runtime' or 'build', got {edge!r}")
    return Dependency(module, edge)
