"""gripsack python frontend — typed module DSL, emits IR.

Modules are plain Python using this package (plan/0001 §3.3). Evaluation
collects Module objects into a graph and emits the IR (JSON) the Rust
core consumes. The core never executes this code; it only reads the IR.

Spans (0004 §2): modules capture their declaration site so core errors
point back at your source. Two authoring styles (0007 §1): the
data-style :func:`module` and the class-style :class:`Module`.
"""

from .deps import Dependency, dep
from .entries import Dest, merge, symlink, template, tracked_copy
from ._facts import Facts, current_facts, facts
from .fetch import (
    Fetch,
    FetchKind,
    brew,
    file_fetch,
    git,
    github_release,
    pixi,
    plugin_fetch,
    tarball,
)
from .graph import IR_VERSION, clear_graph, emit_ir
from .intents import Intent, custom_hook, desktop_entry, fonts, service
from .module import Module, ModuleData, module
from .resources import CORE_RESOURCES, Resource, clear_resources, resource
from .tree import tree
from .steps import (
    Step,
    StepActionKind,
    build_step,
    config_step,
    fetch_step,
    install_step,
    run_step,
    shell_step,
    step,
)
from .conditions import When, when
from .verify import Verify, verify_binary, verify_file, verify_shell

import importlib.metadata as _meta

try:
    __version__ = _meta.version("gripsack")
except _meta.PackageNotFoundError:  # running from a source tree
    __version__ = "0.0.0-dev"

__all__ = [
    "IR_VERSION",
    "module",
    "Module",
    "ModuleData",
    "dep",
    "Dependency",
    "github_release",
    "tarball",
    "git",
    "file_fetch",
    "plugin_fetch",
    "brew",
    "pixi",
    "Fetch",
    "symlink",
    "tracked_copy",
    "tree",
    "merge",
    "template",
    "Dest",
    "service",
    "fonts",
    "desktop_entry",
    "custom_hook",
    "Intent",
    "step",
    "StepActionKind",
    "fetch_step",
    "build_step",
    "install_step",
    "config_step",
    "shell_step",
    "run_step",
    "Step",
    "verify_binary",
    "verify_file",
    "verify_shell",
    "verify_deployed",
    "Verify",
    "resource",
    "Resource",
    "CORE_RESOURCES",
    "clear_resources",
    "current_facts",
    "facts",
    "Facts",
    "when",
    "When",
    "emit_ir",
    "clear_graph",
]
