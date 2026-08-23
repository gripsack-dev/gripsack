"""Conditional modules: `when` predicates over host facts (0001 §5).

>>> from gripsack import when  # noqa: F401  (lives in conditions.py)
>>> w = when(os="linux", tags=["gui"])
>>> w.matches_facts(os="linux", tags=("gui",))
True
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional, TypeVar

from ._facts import Facts

T = TypeVar("T")


@dataclass(frozen=True)
class When:
    """A predicate over host facts — every given condition must match.

    Data style: `module("steam", when=when(os="linux", tags=["gui"]))`.
    Class style, as a decorator:

    >>> @when(os="linux")
    ... class Steam: ...
    """

    os: Optional[str] = None
    arch: Optional[str] = None
    libc: Optional[str] = None
    tags: tuple[str, ...] = ()
    not_tags: tuple[str, ...] = ()

    def matches(self, f: Facts) -> bool:
        if self.os is not None and f.os != self.os:
            return False
        if self.arch is not None and f.arch != self.arch:
            return False
        if self.libc is not None and f.libc != self.libc:
            return False
        if any(not f.has(t) for t in self.tags):
            return False
        return not any(f.has(t) for t in self.not_tags)

    def matches_facts(self, os: str = "", tags: tuple[str, ...] = ()) -> bool:
        """Test helper: match against a hand-built fact set."""
        return self.matches(Facts(os or "linux", "x86_64", "glibc", tags))

    def __call__(self, cls: T) -> T:
        """Decorator path: attach the condition to a Module subclass."""
        cls.when = self  # type: ignore[attr-defined]
        return cls


def when(
    os: Optional[str] = None,
    arch: Optional[str] = None,
    libc: Optional[str] = None,
    tags: Optional[list[str]] = None,
    not_tags: Optional[list[str]] = None,
) -> When:
    """Build a condition over host facts."""
    return When(os, arch, libc, tuple(tags or ()), tuple(not_tags or ()))
