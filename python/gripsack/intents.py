"""Activation intents — declared, translated by adapters (0001 §3.8)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class Intent:
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
