"""Host facts — captured at eval, the only place they exist (0001 §5)."""

from __future__ import annotations

import platform
from typing import Optional


def current_facts(tags: Optional[list[str]] = None) -> dict:
    return {
        "os": platform.system().lower(),
        "arch": platform.machine().lower(),
        "tags": tags or [],
    }
