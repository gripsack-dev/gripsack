"""Steps — the phase building blocks inside a module (0007).

Most modules never write steps: the declarative fields (`source`,
`build`, `install`, `config`) are expanded into the conventional
pipeline by the core. Declare steps explicitly when you need control —
ordering beyond the default chain, resource locks, or a custom action.

`steps` and the declarative fields are mutually exclusive per module
(the core rejects both-shapes with E103).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional

from .sources import Source


@dataclass(frozen=True)
class Step:
    """One node in a module's execution DAG."""

    id: str
    action: dict[str, Any]
    needs: list[str] = field(default_factory=list)
    resources: list[str] = field(default_factory=list)
    phase: Optional[str] = None

    def to_ir(self) -> dict[str, Any]:
        ir: dict[str, Any] = {"id": self.id, "action": self.action}
        if self.needs:
            ir["needs"] = self.needs
        if self.resources:
            ir["resources"] = self.resources
        if self.phase:
            ir["phase"] = self.phase
        return ir


def step(
    id: str,
    action: dict[str, Any],
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
    phase: Optional[str] = None,
) -> Step:
    return Step(id, action, needs or [], resources or [], phase)


def fetch_step(
    source: Source,
    id: str = "fetch",
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
) -> Step:
    """Fetch a typed source. Primitives auto-declare their contention
    domain in the core (pixi → `pixi-lock`, …) — `resources` here is for
    your own shared state (0007 §4)."""
    return Step(
        id,
        {"kind": "fetch", "source": source.to_ir()},
        needs or [],
        resources or [],
        "fetch",
    )


def build_step(
    spec: dict[str, Any],
    id: str = "build",
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
) -> Step:
    return Step(id, {"kind": "build", "spec": spec}, needs or [], resources or [], "build")


def shell_step(
    script: str,
    id: str,
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
    phase: Optional[str] = "custom",
) -> Step:
    """The honest escape hatch: declared, flagged in `plan`, busts
    fine-grained caching (0007 §3)."""
    return Step(
        id,
        {"kind": "custom_shell", "script": script},
        needs or [],
        resources or [],
        phase,
    )
