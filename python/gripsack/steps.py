"""Steps — the phase building blocks inside a module (0007).

Most modules never write steps: the declarative fields (`fetch`,
`build`, `install`, `config`) are expanded into the conventional
pipeline by the core. Declare steps explicitly when you need control —
ordering beyond the default chain, resource locks, retries, or a custom
action.

`steps` and the declarative fields are mutually exclusive per module
(the core rejects both-shapes with E103).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional

from .fetch import Fetch
from .verify import Verify


@dataclass(frozen=True)
class Step:
    """One node in a module's execution DAG."""

    id: str
    action: dict[str, Any]
    needs: list[str] = field(default_factory=list)
    resources: list[str] = field(default_factory=list)
    phase: Optional[str] = None
    verify: Optional[Verify] = None
    retries: Optional[int] = None

    def to_ir(self) -> dict[str, Any]:
        ir: dict[str, Any] = {"id": self.id, "action": self.action}
        if self.needs:
            ir["needs"] = self.needs
        if self.resources:
            ir["resources"] = self.resources
        if self.phase:
            ir["phase"] = self.phase
        if self.verify:
            ir["verify"] = self.verify.to_ir()
        if self.retries is not None:
            ir["retries"] = self.retries
        return ir


def step(
    id: str,
    action: dict[str, Any],
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
    phase: Optional[str] = None,
    verify: Optional[Verify] = None,
    retries: Optional[int] = None,
) -> Step:
    return Step(id, action, needs or [], resources or [], phase, verify, retries)


def fetch_step(
    fetch: Fetch,
    id: str = "fetch",
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
    retries: Optional[int] = None,
) -> Step:
    """Fetch a fetch spec. Primitives auto-declare their contention
    domain in the core (pixi → `pixi-lock`, …) — `resources` here is for
    your own shared state (0007 §4). Fetch steps retry by default."""
    return Step(
        id,
        {"kind": "fetch", "fetch": fetch.to_ir()},
        needs or [],
        resources or [],
        "fetch",
        None,
        retries,
    )


def build_step(
    spec: dict[str, Any],
    id: str = "build",
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
    verify: Optional[Verify] = None,
) -> Step:
    return Step(id, {"kind": "build", "spec": spec}, needs or [], resources or [], "build", verify)


def shell_step(
    script: str,
    id: str,
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
    phase: Optional[str] = "custom",
    verify: Optional[Verify] = None,
    retries: Optional[int] = None,
) -> Step:
    """The honest escape hatch: declared, flagged in `plan`, busts
    fine-grained caching. Pair it with a verify contract (0007 §3)."""
    return Step(
        id,
        {"kind": "custom_shell", "script": script},
        needs or [],
        resources or [],
        phase,
        verify,
        retries,
    )
