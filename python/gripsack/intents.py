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


class IntentKind(str, Enum):
    """The closed set of activation intents (0001 §3.8)."""

    SERVICE = "service"
    FONTS = "fonts"
    DESKTOP_ENTRY = "desktop_entry"
    CUSTOM_SHELL = "custom_shell"


@dataclass(frozen=True)
class Intent:
    kind: IntentKind
    trigger: Trigger
    args: dict[str, Any]

    def __post_init__(self) -> None:
        object.__setattr__(self, "kind", IntentKind(self.kind))
        object.__setattr__(self, "trigger", Trigger(self.trigger))

    def to_ir(self) -> dict[str, Any]:
        return {"trigger": self.trigger.value, "kind": self.kind.value, **self.args}


def service(
    name: str, user: bool = True, trigger: Trigger = Trigger.POST_ACTIVATE
) -> Intent:
    """A user or system service to (re)start after activation."""
    return Intent(IntentKind.SERVICE, trigger, {"name": name, "user": user})


def fonts(trigger: Trigger = Trigger.POST_LINK) -> Intent:
    """Refresh the font cache after this module's files are linked."""
    return Intent(IntentKind.FONTS, trigger, {})


def desktop_entry(trigger: Trigger = Trigger.POST_LINK) -> Intent:
    """Refresh the desktop database after linking."""
    return Intent(IntentKind.DESKTOP_ENTRY, trigger, {})


def custom_hook(script: str, trigger: Trigger = Trigger.POST_ACTIVATE) -> Intent:
    """Escape hatch — flagged, shown by `plan`."""
    return Intent(IntentKind.CUSTOM_SHELL, trigger, {"script": script})
