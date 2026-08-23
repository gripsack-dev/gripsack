"""Declared resources — closing the namespace so typos are errors (0007 §4).

Resources are named, host-global mutexes (or pools) that steps acquire
before running. The core has built-ins for known contention domains
(`network`, `pixi-lock`, `cargo-lock`); anything else must be declared
in your env repo, which is what this module is for.

Declare before creating steps that use them:

>>> from gripsack import resource
>>> PIXI = resource("pixi.lock")

Then reference by marker (typo-proof) or by name (validated):

>>> from gripsack import step
>>> s = step("sync", {"kind": "custom_shell", "script": "pixi install"},
...        resources=["pixi.lock"])

An unknown name raises immediately at eval time — before the core ever
sees your IR:

>>> step("bad", {"kind": "custom_shell", "script": "true"}, resources=["cargo.lokc"])
Traceback (most recent call last):
    ...
ValueError: unknown resource 'cargo.lokc' ...
"""

from __future__ import annotations

from dataclasses import dataclass

#: Built-in contention domains the core knows how to serialize or
#: throttle. Mirrors `KNOWN_RESOURCES` in `crates/gripsack-ir` — keep
#: the two in sync (IR changes touch all sides: `.agents/skills/gripsack-ir`).
CORE_RESOURCES = frozenset({"network", "pixi-lock", "cargo-lock"})


@dataclass(frozen=True)
class Resource:
    """A declared resource marker. Create with :func:`resource`."""

    name: str


_REGISTRY: dict[str, Resource] = {}


def resource(name: str) -> Resource:
    """Declare a resource and return its marker.

    >>> r = resource("company-registry.lock")
    >>> r.name
    'company-registry.lock'
    """
    if not name:
        raise ValueError("resource name must not be empty")
    r = Resource(name)
    _REGISTRY[name] = r
    return r


def declared_resources() -> list[Resource]:
    """All resources declared so far in this eval, sorted by name."""
    return [_REGISTRY[name] for name in sorted(_REGISTRY)]


def clear_resources() -> None:
    """Drop all declared resources (test isolation)."""
    _REGISTRY.clear()


def validate_resource_refs(refs: list[str], owner: str) -> None:
    """Raise ``ValueError`` if any ref is neither declared nor built-in."""
    for ref in refs:
        if ref not in _REGISTRY and ref not in CORE_RESOURCES:
            known = sorted([*_REGISTRY, *CORE_RESOURCES])
            raise ValueError(
                f"unknown resource {ref!r} in {owner} — "
                f"declare it first with resource({ref!r}). Known: {', '.join(known)}"
            )
