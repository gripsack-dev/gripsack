"""The module graph: registry and IR emission (0001 §3.2)."""

from __future__ import annotations

import json
from typing import TYPE_CHECKING, Optional

from .facts import current_facts
from .module import ModuleData, _collect_pipeline
from .resources import declared_resources

if TYPE_CHECKING:
    from .module import Module

IR_VERSION = 1

_GRAPH: dict[str, ModuleData] = {}
_CLASSES: dict[str, tuple[type["Module"], Optional[dict]]] = {}


def register(m: ModuleData) -> None:
    """Register a module. Duplicate names are an eval-time error with
    both declaration sites (the IR map can only ever hold one)."""
    if m.name in _GRAPH or m.name in _CLASSES:
        prev = _GRAPH.get(m.name)
        prev_span = (
            prev.span
            if prev
            else (_CLASSES[m.name][1] if m.name in _CLASSES else None)
        )
        where = (
            f" (first declared at {prev_span['file']}:{prev_span['line']})"
            if prev_span
            else ""
        )
        raise ValueError(f"duplicate module {m.name!r}{where}")
    _GRAPH[m.name] = m


def register_class(cls: type["Module"], span: Optional[dict]) -> None:
    """Register a class-style module; instantiated lazily at emit time."""
    name = cls.name or cls.__name__.lower()
    if name in _GRAPH or name in _CLASSES:
        raise ValueError(f"duplicate module {name!r}")
    _CLASSES[name] = (cls, span)


def clear_graph() -> None:
    """Drop all registered modules (test isolation)."""
    _GRAPH.clear()
    _CLASSES.clear()


def emit_ir(tags: Optional[list[str]] = None) -> str:
    """Serialize the registered graph as IR JSON."""
    modules = dict(_GRAPH)
    for name, (cls, span) in _CLASSES.items():
        instance = cls()
        steps, verify = _collect_pipeline(instance)
        modules[name] = ModuleData(
            name=name, steps=steps, verify=verify, span=span
        )
    ir: dict = {
        "ir_version": IR_VERSION,
        "host": current_facts(tags),
        "modules": {name: m.to_ir() for name, m in modules.items()},
    }
    resources = declared_resources()
    if resources:
        ir["resources"] = [{"name": r.name} for r in resources]
    return json.dumps(ir, indent=2, sort_keys=True)
