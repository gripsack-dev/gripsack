"""Host facts — captured at eval, the only place they exist (0001 §5)."""

from __future__ import annotations

import platform
import sys
from typing import Optional


def detect_libc() -> str:
    """e.g. "glibc-2.36", "musl", "darwin". Binary asset selection
    depends on it."""
    if sys.platform == "darwin":
        return "darwin"
    name, version = platform.libc_ver()
    if name:
        return f"{name}-{version}"
    # musl doesn't report via libc_ver
    import os

    musl_loader = f"/lib/ld-musl-{platform.machine()}.so.1"
    if os.path.exists(musl_loader):
        return "musl"
    return "unknown"


def current_facts(tags: Optional[list[str]] = None) -> dict:
    return {
        "os": platform.system().lower(),
        "arch": platform.machine().lower(),
        "tags": tags or [],
        "libc": detect_libc(),
    }
