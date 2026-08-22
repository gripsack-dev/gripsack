"""gripsack python frontend — typed module DSL, emits IR.

Modules are plain Python using this package (plan/0001 §3.3). Evaluation
collects Module objects into a graph and emits the IR (JSON) the Rust
core consumes. The core never executes this code; it only reads the IR.

Spans (0004 §2): `module()` captures the caller's file and line so core
errors can point back at the user's source.
"""

from .deps import Dependency, dep
from .entries import Dest, merge, symlink, template, tracked_copy
from .facts import current_facts
from .graph import IR_VERSION, clear_graph, emit_ir
from .intents import Intent, custom_hook, desktop_entry, fonts, service
from .module import Module, module
from .sources import (
    Source,
    file_source,
    git,
    github_release,
    plugin_source,
    tarball,
)
from .steps import Step, build_step, fetch_step, shell_step, step

__version__ = "0.1.0"

__all__ = [
    "IR_VERSION",
    "module",
    "Module",
    "dep",
    "Dependency",
    "github_release",
    "tarball",
    "git",
    "file_source",
    "plugin_source",
    "Source",
    "symlink",
    "tracked_copy",
    "merge",
    "template",
    "Dest",
    "service",
    "fonts",
    "desktop_entry",
    "custom_hook",
    "Intent",
    "step",
    "fetch_step",
    "build_step",
    "shell_step",
    "Step",
    "current_facts",
    "emit_ir",
    "clear_graph",
]
