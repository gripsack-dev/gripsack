"""Module declaration and span capture (0004 §2)."""

from __future__ import annotations

import inspect
from dataclasses import dataclass, field
from typing import Any, Optional

from .deps import Dependency
from .entries import Dest
from .intents import Intent
from .sources import Source
from .steps import Step


@dataclass
class Module:
    name: str
    source: Optional[Source]
    build: Optional[dict[str, Any]] = None
    install: dict[str, Dest] = field(default_factory=dict)
    config: dict[str, Dest] = field(default_factory=dict)
    depends: list[Dependency] = field(default_factory=list)
    activate: list[Intent] = field(default_factory=list)
    steps: Optional[list[Step]] = None
    span: Optional[dict[str, Any]] = None

    def to_ir(self) -> dict[str, Any]:
        ir: dict[str, Any] = {}
        if self.source:
            ir["source"] = self.source.to_ir()
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
        if self.span:
            ir["span"] = self.span
        return ir


def module(
    name: str,
    source: Optional[Source] = None,
    build: Optional[dict[str, Any]] = None,
    install: Optional[dict[str, Dest]] = None,
    config: Optional[dict[str, Dest]] = None,
    depends: Optional[list[Dependency]] = None,
    activate: Optional[list[Intent]] = None,
    steps: Optional[list[Step]] = None,
) -> Module:
    """Declare a module and register it in the graph.

    `source` may be None for dotfiles-only modules (0006 §2 level 1).
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
        source=source,
        build=build,
        install=install or {},
        config=config or {},
        depends=depends or [],
        activate=activate or [],
        steps=steps,
        span=span,
    )
    register(m)
    return m
