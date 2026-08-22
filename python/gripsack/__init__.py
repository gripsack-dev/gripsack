"""gripsack python frontend — typed module DSL, emits IR.

Modules are plain Python using this package (plan/0001 §3.3). Evaluation
collects Module objects into a graph and emits the IR (JSON) the Rust
core consumes. The core never executes this code; it only reads the IR.

Provenance (0001 §3.2): `module()` captures the caller's file and line so
core errors can point back at the user's source.
"""

from __future__ import annotations

import inspect
import json
import platform
from dataclasses import dataclass, field
from typing import Any, Optional

__version__ = "0.1.0"

IR_VERSION = 1

__all__ = [
    "IR_VERSION",
    "module",
    "dep",
    "github_release",
    "tarball",
    "git",
    "file_source",
    "plugin_source",
    "symlink",
    "tracked_copy",
    "merge",
    "template",
    "service",
    "fonts",
    "desktop_entry",
    "custom_hook",
    "emit_ir",
    "clear_graph",
]


# ---------------------------------------------------------------- sources

@dataclass(frozen=True)
class Source:
    """A typed fetcher (0001 §3.1, 0002 §2)."""

    kind: str
    args: dict[str, Any]

    def to_ir(self) -> dict[str, Any]:
        return {"kind": self.kind, **self.args}


def github_release(
    repo: str,
    asset: str,
    version: Optional[str] = None,
    sha256: Optional[str] = None,
    base_url: Optional[str] = None,
) -> Source:
    """GitHub releases; `base_url` covers GitHub Enterprise (0002 rung 1)."""
    args: dict[str, Any] = {"repo": repo, "asset": asset}
    if version is not None:
        args["version"] = version
    if sha256 is not None:
        args["sha256"] = sha256
    if base_url is not None:
        args["base_url"] = base_url
    return Source("github_release", args)


def tarball(url: str, sha256: Optional[str] = None) -> Source:
    args: dict[str, Any] = {"url": url}
    if sha256 is not None:
        args["sha256"] = sha256
    return Source("tarball", args)


def git(url: str, rev: str) -> Source:
    return Source("git", {"url": url, "rev": rev})


def file_source(path: str) -> Source:
    return Source("file", {"path": path})


def plugin_source(name: str, **args: Any) -> Source:
    """A sourcerer plugin transport (0002 §4)."""
    return Source("plugin", {"name": name, "args": args})


# ---------------------------------------------------------------- entries

@dataclass(frozen=True)
class Dest:
    """A destination with an ownership mode (0001 §3.7)."""

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


# ---------------------------------------------------------------- deps

@dataclass(frozen=True)
class Dependency:
    module: str
    edge: str = "runtime"  # "build" = ephemeral, build-only (0001 §3.1)


def dep(module: str, edge: str = "runtime") -> Dependency:
    if edge not in ("runtime", "build"):
        raise ValueError(f"edge must be 'runtime' or 'build', got {edge!r}")
    return Dependency(module, edge)


# ---------------------------------------------------------------- intents

@dataclass(frozen=True)
class Intent:
    """Activation intent — declared, translated by adapters (0001 §3.8)."""

    kind: str
    trigger: str
    args: dict[str, Any]

    def to_ir(self) -> dict[str, Any]:
        return {"trigger": self.trigger, "kind": self.kind, **self.args}


def service(name: str, user: bool = True, trigger: str = "post_activate") -> Intent:
    return Intent("service", trigger, {"name": name, "user": user})


def fonts(trigger: str = "post_link") -> Intent:
    return Intent("fonts", trigger, {})


def desktop_entry(trigger: str = "post_link") -> Intent:
    return Intent("desktop_entry", trigger, {})


def custom_hook(script: str, trigger: str = "post_activate") -> Intent:
    """Escape hatch — flagged, shown by `plan`."""
    return Intent("custom_shell", trigger, {"script": script})


# ---------------------------------------------------------------- modules

@dataclass
class Module:
    name: str
    source: Source
    build: Optional[dict[str, Any]] = None
    install: dict[str, Dest] = field(default_factory=dict)
    config: dict[str, Dest] = field(default_factory=dict)
    depends: list[Dependency] = field(default_factory=list)
    activate: list[Intent] = field(default_factory=list)
    provenance: Optional[dict[str, Any]] = None

    def to_ir(self) -> dict[str, Any]:
        ir: dict[str, Any] = {"source": self.source.to_ir()}
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
        if self.provenance:
            ir["provenance"] = self.provenance
        return ir


_GRAPH: dict[str, Module] = {}


def module(
    name: str,
    source: Source,
    build: Optional[dict[str, Any]] = None,
    install: Optional[dict[str, Dest]] = None,
    config: Optional[dict[str, Dest]] = None,
    depends: Optional[list[Dependency]] = None,
    activate: Optional[list[Intent]] = None,
) -> Module:
    """Declare a module and register it in the graph.

    Captures the caller's file/line as provenance (0001 §3.2).
    """
    frame = inspect.currentframe()
    caller = frame.f_back if frame and frame.f_back else None
    provenance = (
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
        provenance=provenance,
    )
    _GRAPH[name] = m
    return m


def clear_graph() -> None:
    """Drop all registered modules (test isolation)."""
    _GRAPH.clear()


def emit_ir(tags: Optional[list[str]] = None) -> str:
    """Serialize the registered graph as IR JSON (plan/0001 §3.2).

    Host facts are captured here — eval is the only place they are
    consulted (0001 §5).
    """
    ir = {
        "ir_version": IR_VERSION,
        "host": {
            "os": platform.system().lower(),
            "arch": platform.machine().lower(),
            "tags": tags or [],
        },
        "modules": {name: m.to_ir() for name, m in _GRAPH.items()},
    }
    return json.dumps(ir, indent=2, sort_keys=True)
