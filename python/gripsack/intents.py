"""Activation intents — declared, translated by adapters (0001 §3.8)."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Any


class Trigger(str, Enum):
    """When an intent runs (0001 §3.8)."""

    POST_LINK = "post_link"
    POST_ACTIVATE = "post_activate"
    ON_REMOVE = "on_remove"


@dataclass(frozen=True)
class Intent:
    kind: str
    trigger: Trigger
    args: dict[str, Any]

    def __post_init__(self) -> None:
        object.__setattr__(self, "trigger", Trigger(self.trigger))

    def to_ir(self) -> dict[str, Any]:
        return {"trigger": self.trigger.value, "kind": self.kind, **self.args}


def service(
    name: str, user: bool = True, trigger: Trigger = Trigger.POST_ACTIVATE
) -> Intent:
    """A user or system service to (re)start after activation."""
    return Intent("service", trigger, {"name": name, "user": user})


def fonts(trigger: Trigger = Trigger.POST_LINK) -> Intent:
    """Refresh the font cache after this module's files are linked."""
    return Intent("fonts", trigger, {})


def desktop_entry(trigger: Trigger = Trigger.POST_LINK) -> Intent:
    """Refresh the desktop database after linking."""
    return Intent("desktop_entry", trigger, {})


def custom_hook(script: str, trigger: Trigger = Trigger.POST_ACTIVATE) -> Intent:
    """Escape hatch — flagged, shown by `plan`."""
    return Intent("custom_shell", trigger, {"script": script})
