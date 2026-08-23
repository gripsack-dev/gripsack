"""Steps — the phase building blocks inside a module (0007).

Most modules never write steps: the declarative fields (`fetch`,
`build`, `install`, `config`) are expanded into the conventional
pipeline by the core. Declare steps explicitly when you need control —
ordering beyond the default chain, resource locks, retries, or a custom
action:

>>> from gripsack import step, shell_step, fetch_step, tarball
>>> steps = [
...     fetch_step(tarball("https://example.invalid/h.tar.xz")),
...     shell_step("patch -p1 < fix.patch", id="patch", needs=["fetch"]),
... ]

`steps` and the declarative fields are mutually exclusive per module
(the core rejects both-shapes with E103).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Optional

from .entries import Dest
from .fetch import Fetch
from .resources import validate_resource_refs
from .verify import Verify


class StepActionKind(str, Enum):
    """The closed set of step actions the engine interprets."""

    FETCH = "fetch"
    BUILD = "build"
    INSTALL = "install"
    CONFIG_DEPLOY = "config_deploy"
    INTENT = "intent"
    VERIFY = "verify"
    RUN = "run"
    CUSTOM_SHELL = "custom_shell"


class Phase(str, Enum):
    """Reporting tag for a step (0007 §2) — never a scheduling barrier."""

    FETCH = "fetch"
    BUILD = "build"
    INSTALL = "install"
    CONFIG = "config"
    VERIFY = "verify"
    ACTIVATE = "activate"
    CUSTOM = "custom"


@dataclass(frozen=True)
class Step:
    """One node in a module's execution DAG.

    Attributes:
        id: module-scoped identifier; cross-module refs use
            ``"module:step"`` form.
        action: typed action dict — build with the helpers
            (:func:`fetch_step`, :func:`build_step`, :func:`shell_step`)
            rather than by hand.
        needs: sibling step ids or ``"module:step"`` refs that must
            finish first.
        resources: named resources to acquire before running (see
            :mod:`gripsack.resources`). Unknown names raise at eval time.
        phase: reporting tag — never a scheduling barrier.
        verify: smoke contract run right after the action; failure means
            step failed.
        retries: retry count override. Default: engine policy — retries
            only for fetch actions, 0 otherwise.
    """

    id: str
    action: dict[str, Any]
    needs: list[str] = field(default_factory=list)
    resources: list[str] = field(default_factory=list)
    phase: Optional[Phase] = None
    verify: Optional[Verify] = None
    retries: Optional[int] = None

    def __post_init__(self) -> None:
        validate_resource_refs(self.resources, f"step {self.id!r}")
        StepActionKind(self.action.get("kind", ""))
        if self.phase is not None:
            object.__setattr__(self, "phase", Phase(self.phase))

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
    """Create a fully custom step. Prefer the typed helpers when they fit."""
    return Step(id, action, needs or [], resources or [], phase, verify, retries)


def fetch_step(
    fetch: Fetch,
    id: str = "fetch",
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
    retries: Optional[int] = None,
) -> Step:
    """Fetch a fetch spec.

    Primitives auto-declare their contention domain in the core (pixi →
    ``pixi-lock``, …) — ``resources`` here is for your own shared state
    (0007 §4). Fetch steps retry by default.
    """
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
    """Run a build spec, e.g. ``{"kind": "cargo_install"}``."""
    return Step(
        id, {"kind": "build", "spec": spec}, needs or [], resources or [], "build", verify
    )


def install_step(
    entries: dict[str, Dest],
    id: str = "install",
    needs: Optional[list[str]] = None,
) -> Step:
    """Deploy built artifacts to their destinations."""
    return Step(
        id,
        {
            "kind": "install",
            "entries": [
                {"from": src, "to": d.to, "mode": d.mode}
                for src, d in entries.items()
            ],
        },
        needs or [],
        [],
        "install",
    )


def config_step(
    entries: dict[str, Dest],
    id: str = "config",
    needs: Optional[list[str]] = None,
) -> Step:
    """Deploy config files per their ownership modes (0001 §3.7)."""
    return Step(
        id,
        {
            "kind": "config_deploy",
            "entries": [
                {"from": src, "to": d.to, "mode": d.mode}
                for src, d in entries.items()
            ],
        },
        needs or [],
        [],
        "config",
    )


def run_step(
    argv: list[str],
    id: str = "run",
    needs: Optional[list[str]] = None,
    env: Optional[dict[str, str]] = None,
    cwd: Optional[str] = None,
    outputs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
    verify: Optional[Verify] = None,
    retries: Optional[int] = None,
) -> Step:
    """A structured action — the rung between primitives and shell
    (0007 §3): argv/env/cwd as data, no shell interpretation, declared
    ``outputs`` make it cacheable (0008 §4).

    >>> run_step(["make", "install"], outputs=["bin/hx"]).action["kind"]
    'run'
    """
    action: dict[str, Any] = {"kind": "run", "argv": argv}
    if env:
        action["env"] = env
    if cwd:
        action["cwd"] = cwd
    if outputs:
        action["outputs"] = outputs
    return Step(
        id, action, needs or [], resources or [], Phase.CUSTOM, verify, retries
    )


def shell_step(
    script: str,
    id: str,
    needs: Optional[list[str]] = None,
    resources: Optional[list[str]] = None,
    phase: Optional[Phase] = Phase.CUSTOM,
    verify: Optional[Verify] = None,
    retries: Optional[int] = None,
    outputs: Optional[list[str]] = None,
) -> Step:
    """The last rung, not the default (0007 §3): declared, flagged in
    `plan`. Declared ``outputs`` restore caching/satisfaction (0008 §4);
    without them the step always runs.

    >>> shell_step("make install", id="mk", needs=["build"]).action
    {'kind': 'custom_shell', 'script': 'make install'}
    """
    action: dict[str, Any] = {"kind": "custom_shell", "script": script}
    if outputs:
        action["outputs"] = outputs
    return Step(
        id, action, needs or [], resources or [], phase, verify, retries
    )
