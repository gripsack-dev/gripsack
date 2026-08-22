"""Module declaration and span capture (0004 §2)."""

from __future__ import annotations

import inspect
from dataclasses import dataclass, field
from typing import Any, Optional

from .deps import Dependency
from .entries import Dest
from .fetch import Fetch
from .intents import Intent
from .steps import Step
from .verify import Verify


@dataclass
class Module:
    name: str
    fetch: Optional[Fetch]
    build: Optional[dict[str, Any]] = None
    install: dict[str, Dest] = field(default_factory=dict)
    config: dict[str, Dest] = field(default_factory=dict)
    depends: list[Dependency] = field(default_factory=list)
    activate: list[Intent] = field(default_factory=list)
    steps: Optional[list[Step]] = None
    verify: Optional[Verify] = None
    retries: Optional[int] = None
    span: Optional[dict[str, Any]] = None

    def to_ir(self) -> dict[str, Any]:
        ir: dict[str, Any] = {}
        if self.fetch:
            ir["fetch"] = self.fetch.to_ir()
        if self.build:
            ir["build"] = self.build
        if self.install:
            ir["install"] = [
                {"from": src, "to": d.to, "mode": d.mode}
                for src, d in self.install.items()
            ]
        if self.config:
            ir["config"] = [
                {"from": src, "to": d.to, "mode": d.mode}
                for src, d in self.config.items()
            ]
        if self.depends:
            ir["depends"] = [
                {"module": d.module, "edge": d.edge} for d in self.depends
            ]
        if self.activate:
            ir["activate"] = [i.to_ir() for i in self.activate]
        if self.steps is not None:
            ir["steps"] = [s.to_ir() for s in self.steps]
        if self.verify:
            ir["verify"] = self.verify.to_ir()
        if self.retries is not None:
            ir["retries"] = self.retries
        if self.span:
            ir["span"] = self.span
        return ir


def module(
    name: str,
    fetch: Optional[Fetch] = None,
    build: Optional[dict[str, Any]] = None,
    install: Optional[dict[str, Dest]] = None,
    config: Optional[dict[str, Dest]] = None,
    depends: Optional[list[Dependency]] = None,
    activate: Optional[list[Intent]] = None,
    steps: Optional[list[Step]] = None,
    verify: Optional[Verify] = None,
    retries: Optional[int] = None,
) -> Module:
    """Declare a module and register it in the graph.

    `fetch` may be None for dotfiles-only modules (0006 §2 level 1).
    `steps` gives full control of the pipeline and is mutually exclusive
    with the declarative fields (0007 §1 — the core rejects both-shapes
    with E103). Captures the caller's file/line as its span (0004 §2).
    """
    from .graph import register

    frame = inspect.currentframe()
    caller = frame.f_back if frame and frame.f_back else None
    span = (
        {"file": caller.f_code.co_filename, "line": caller.f_lineno}
        if caller
        else None
    )
    m = Module(
        name=name,
        fetch=fetch,
        build=build,
        install=install or {},
        config=config or {},
        depends=depends or [],
        activate=activate or [],
        steps=steps,
        verify=verify,
        retries=retries,
        span=span,
    )
    register(m)
    return m
