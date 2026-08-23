"""Module declaration — two authoring styles (0007 §1).

**Data style** — :func:`module` builds a module from declarative fields;
the core expands them into the conventional pipeline:

>>> from gripsack import module, github_release, symlink
>>> m = module(
...     "helix",
...     fetch=github_release(repo="helix-editor/helix", asset="h.tar.xz"),
...     install={"bin/hx": symlink("~/.local/bin/hx")},
... )

**Class style** — subclass :class:`Module` and override phase methods;
each returns a step or a list of steps, and the pipeline chains them in
order (within a phase and across phase boundaries) without you writing
``needs`` by hand:

>>> from gripsack import Module, fetch_step, install_step, symlink
>>> class Helix(Module):
...     def fetch(self):
...         return fetch_step(github_release(
...             repo="helix-editor/helix", asset="h.tar.xz"))
...     def install(self):
...         return install_step({"bin/hx": symlink("~/.local/bin/hx")})

.. warning::
    Phase methods run **at eval time only**. They build the IR — they
    never run while your system is being built. Anything that must
    happen at build time belongs in a step action, not in a method body.
"""

from __future__ import annotations

import inspect
from dataclasses import dataclass, field
from typing import Any, ClassVar, Optional, Union

from .deps import Dependency
from .entries import Dest
from .fetch import Fetch
from .intents import Intent
from .steps import Step
from .verify import Verify

#: Pipeline order for the class style (0007 §verify).
PIPELINE_PHASES = ("fetch", "build", "install", "config", "verify", "activate")


@dataclass
class ModuleData:
    """The module as IR data — what both authoring styles produce."""

    name: str
    fetch: Optional[Fetch] = None
    build: Optional[dict[str, Any]] = None
    install: dict[str, Dest] = field(default_factory=dict)
    config: dict[str, Dest] = field(default_factory=dict)
    depends: list[Dependency] = field(default_factory=list)
    activate: list[Intent] = field(default_factory=list)
    steps: Optional[list[Step]] = None
    verify: Optional[Verify] = None
    retries: Optional[int] = None
    span: Optional[dict[str, Any]] = None

    def to_ir(self) -> dict[str, Any]:
        ir: dict[str, Any] = {}
        if self.fetch:
            ir["fetch"] = self.fetch.to_ir()
        if self.build:
            ir["build"] = self.build
        if self.install:
            ir["install"] = [
                {"from": src, "to": d.to, "mode": d.mode}
                for src, d in self.install.items()
            ]
        if self.config:
            ir["config"] = [
                {"from": src, "to": d.to, "mode": d.mode}
                for src, d in self.config.items()
            ]
        if self.depends:
            ir["depends"] = [
                {"module": d.module, "edge": d.edge} for d in self.depends
            ]
        if self.activate:
            ir["activate"] = [i.to_ir() for i in self.activate]
        if self.steps is not None:
            ir["steps"] = [s.to_ir() for s in self.steps]
        if self.verify:
            ir["verify"] = self.verify.to_ir()
        if self.retries is not None:
            ir["retries"] = self.retries
        if self.span:
            ir["span"] = self.span
        return ir


def _caller_span(depth: int) -> Optional[dict[str, Any]]:
    frame = inspect.currentframe()
    for _ in range(depth):
        if frame and frame.f_back:
            frame = frame.f_back
    if frame is None:
        return None
    return {"file": frame.f_code.co_filename, "line": frame.f_lineno}


def module(
    name: str,
    fetch: Optional[Fetch] = None,
    build: Optional[dict[str, Any]] = None,
    install: Optional[dict[str, Dest]] = None,
    config: Optional[dict[str, Dest]] = None,
    depends: Optional[list[Dependency]] = None,
    activate: Optional[list[Intent]] = None,
    steps: Optional[list[Step]] = None,
    verify: Optional[Verify] = None,
    retries: Optional[int] = None,
) -> ModuleData:
    """Declare a module from declarative fields (data style).

    Args:
        name: module name — other modules reference it in ``dep(name)``.
        fetch: how to obtain the payload; ``None`` for dotfiles-only
            modules (0006 §2 level 1).
        build: build spec, e.g. ``{"kind": "cargo_install"}``.
        install: built artifacts → destinations with ownership modes.
        config: config files → destinations with ownership modes.
        depends: module dependencies; ``dep("rust", edge="build")`` for
            ephemeral build-only deps.
        activate: activation intents (services, fonts, …).
        steps: explicit pipeline — mutually exclusive with the
            declarative fields (E103).
        verify: module-level smoke contract, run pre-flip.
        retries: retry default for this module's steps.

    Captures the caller's file/line as the module's span (0004 §2).
    """
    from .graph import register

    m = ModuleData(
        name=name,
        fetch=fetch,
        build=build,
        install=install or {},
        config=config or {},
        depends=depends or [],
        activate=activate or [],
        steps=steps,
        verify=verify,
        retries=retries,
        span=_caller_span(2),
    )
    register(m)
    return m


StepsResult = Union[Step, list[Step], None]


class Module:
    """Base class for the class authoring style (0007 §1).

    Subclass and override any of the phase methods — :meth:`fetch`,
    :meth:`build`, :meth:`install`, :meth:`config`, :meth:`verify`,
    :meth:`activate`. Each returns a step, a list of steps, or nothing.
    The pipeline chains the phases in order and sequences steps within
    each phase, filling only *empty* ``needs`` — explicit ``needs`` you
    set always win.

    Concrete subclasses register themselves at definition time; set
    ``abstract = True`` to define a shared base that should not be
    registered itself:

    >>> from gripsack import Module, fetch_step, tarball, shell_step
    >>> class Patched(Module):
    ...     abstract = True
    ...     def patch_script(self):
    ...         return "patch -p1 < fix.patch"
    ...     def build(self):
    ...         return shell_step(self.patch_script(), id="patch")

    Attributes:
        name: module name; defaults to the class name lowercased.
        abstract: if True, the subclass is a base and is not registered.
    """

    name: ClassVar[Optional[str]] = None
    abstract: ClassVar[bool] = False

    def fetch(self) -> StepsResult:
        """Obtain the payload — e.g. ``fetch_step(github_release(...))``."""
        return []

    def build(self) -> StepsResult:
        """Transform the fetched payload — e.g. ``build_step({...})``."""
        return []

    def install(self) -> StepsResult:
        """Deploy built artifacts — e.g. ``install_step({...})``."""
        return []

    def config(self) -> StepsResult:
        """Deploy config files — e.g. ``config_step({...})``."""
        return []

    def verify(self) -> StepsResult:
        """Smoke contract, run pre-flip — e.g. a binary-runs check."""
        return []

    def activate(self) -> StepsResult:
        """Activation intents — services, fonts, desktop entries."""
        return []

    def __init_subclass__(cls, **kwargs: Any) -> None:
        super().__init_subclass__(**kwargs)
        if cls.__dict__.get("abstract"):
            return
        from .graph import register

        span = _caller_span(2)
        instance = cls()
        steps = _collect_pipeline(instance)
        register(
            ModuleData(
                name=cls.name or cls.__name__.lower(),
                steps=steps,
                span=span,
            )
        )


def _normalize(result: StepsResult, phase: str) -> list[Step]:
    if result is None:
        return []
    steps = [result] if isinstance(result, Step) else list(result)
    out = []
    for s in steps:
        if s.phase is None:
            s = Step(
                s.id, s.action, s.needs, s.resources, phase, s.verify, s.retries
            )
        out.append(s)
    return out


def _collect_pipeline(instance: Module) -> list[Step]:
    """Gather phase methods into a chained, explicit step list.

    Sequencing rule: within a phase and across phase boundaries, a step
    with empty ``needs`` needs the previous step; explicit ``needs``
    are never rewritten.
    """
    chained: list[Step] = []
    for phase in PIPELINE_PHASES:
        steps = _normalize(getattr(instance, phase)(), phase)
        for s in steps:
            if not s.needs and chained:
                s = Step(
                    s.id,
                    s.action,
                    [chained[-1].id],
                    s.resources,
                    s.phase,
                    s.verify,
                    s.retries,
                )
            chained.append(s)
    return chained
