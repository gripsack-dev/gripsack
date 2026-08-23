"""The module graph: registry and IR emission (0001 §3.2)."""

from __future__ import annotations

import json
from typing import Optional

from .facts import current_facts
from .module import ModuleData
from .resources import declared_resources

IR_VERSION = 1

_GRAPH: dict[str, ModuleData] = {}


def register(m: ModuleData) -> None:
    _GRAPH[m.name] = m


def clear_graph() -> None:
    """Drop all registered modules (test isolation)."""
    _GRAPH.clear()


def emit_ir(tags: Optional[list[str]] = None) -> str:
    """Serialize the registered graph as IR JSON."""
    ir: dict = {
        "ir_version": IR_VERSION,
        "host": current_facts(tags),
        "modules": {name: m.to_ir() for name, m in _GRAPH.items()},
    }
    resources = declared_resources()
    if resources:
        ir["resources"] = [{"name": r.name} for r in resources]
    return json.dumps(ir, indent=2, sort_keys=True)
