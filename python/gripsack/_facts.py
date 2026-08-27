"""Host facts — captured at eval, the only place they exist (0001 §5).

The runner sets tags from the host entrypoint, then modules evaluate
against the shared `facts` object — the pytest-fixture pattern: declare
what you need, the harness injects it.

>>> from gripsack import facts
>>> facts.os in ("linux", "darwin")
True
>>> facts.has("gui")  # depends on the host's tags
False
"""

from __future__ import annotations

import os
import platform
import sys
from dataclasses import dataclass, field
from typing import Optional


def detect_libc() -> str:
    """e.g. "glibc-2.36", "musl", "darwin". Binary asset selection
    depends on it."""
    if sys.platform == "darwin":
        return "darwin"
    name, version = platform.libc_ver()
    if name == "musl":
        # normalize: the musl/glibc axis is what asset selection keys
        # on; version formats differ per probe (parity corpus, docker)
        return "musl"
    if name:
        return f"{name}-{version}"
    musl_loader = f"/lib/ld-musl-{platform.machine()}.so.1"
    if os.path.exists(musl_loader):
        return "musl"
    return "unknown"


@dataclass(frozen=True)
class Facts:
    """The curated host fact set: os, arch, libc, tags. Everything else
    belongs in eval-time code or host tags — by design (0001 §5)."""

    os: str
    arch: str
    libc: str
    tags: tuple[str, ...] = field(default_factory=tuple)

    def has(self, tag: str) -> bool:
        """Whether the host declared `tag`.

        >>> Facts("linux", "x86_64", "glibc-2.36", ("gui",)).has("gui")
        True
        """
        return tag in self.tags

    @property
    def is_linux(self) -> bool:
        return self.os == "linux"

    @property
    def is_macos(self) -> bool:
        return self.os == "darwin"


def _auto() -> Facts:
    return Facts(
        os=platform.system().lower(),
        arch=platform.machine().lower(),
        libc=detect_libc(),
    )


#: The shared facts object. The eval runner replaces it after reading
#: the host entrypoint; modules read it freely.
facts: Facts = _auto()


def _set_tags(tags: list[str]) -> None:
    """Called by the eval runner once the host's tags are known."""
    global facts
    facts = Facts(os=facts.os, arch=facts.arch, libc=facts.libc, tags=tuple(tags))


def current_facts(tags: Optional[list[str]] = None) -> dict:
    f = _auto() if tags is not None else facts
    if tags is not None:
        f = Facts(os=f.os, arch=f.arch, libc=f.libc, tags=tuple(tags))
    return {
        "os": f.os,
        "arch": f.arch,
        "libc": f.libc,
        "tags": list(f.tags),
    }
