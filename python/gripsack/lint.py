"""Eval-time linting (0011): registered ``griplint-*`` plugins validate
config files before anything stages.

The frontend is the plugin host: it resolves the ``[linters]`` registry
in env.toml, sends each linter its module's config files (post-``tree()``
expansion) plus the tool version from the host lockfile, and collects
diagnostics in the core's shared shape. The frontend *serializes* — only
the core renders (0009 §2).
"""

from __future__ import annotations

import json
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING, Any, Optional

try:
    import tomllib
except ModuleNotFoundError:  # python 3.10
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ModuleNotFoundError:
        tomllib = None  # type: ignore[assignment]

if TYPE_CHECKING:
    from .module import ModuleData

#: Frontend-originated lint codes (0011 §5) — E5xx, clear of the core's
#: reserved E0xx range and of plugin-namespaced codes.
UNREGISTERED_LINTER = "E501"
BAD_REGISTRATION = "E502"
MISSING_EXECUTABLE = "E503"

@dataclass
class Diagnostic:
    """The core's diagnostic shape (0009 §4), serialized as-is."""

    code: str
    severity: str  # "error" | "warning"
    message: str
    labels: list[dict[str, Any]] = field(default_factory=list)
    help: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "code": self.code,
            "severity": self.severity,
            "message": self.message,
            "labels": self.labels,
        }
        if self.help is not None:
            out["help"] = self.help
        return out


def _label(span: Optional[dict[str, Any]], note: str) -> dict[str, Any]:
    return {"span": span, "note": note}


def load_registry(repo: Path) -> dict[str, dict[str, Any]]:
    """The ``[linters]`` table from env.toml (0010 §3)."""
    env_toml = repo / "env.toml"
    if not env_toml.exists() or tomllib is None:
        return {}
    data = tomllib.loads(env_toml.read_text())
    return data.get("linters", {})


def _tool_versions(repo: Path, host: Optional[str]) -> dict[str, str]:
    """Module → pinned tool version, from the host lockfile (0011 §3)."""
    if not host:
        return {}
    lock = repo / "locks" / f"{host}.lock"
    if not lock.exists():
        return {}
    try:
        data = json.loads(lock.read_text())
    except json.JSONDecodeError:
        return {}
    out = {}
    for name, entry in data.get("modules", {}).items():
        version = (entry.get("resolved") or {}).get("version")
        if version:
            out[name] = version
    return out


def _resolve_exe(name: str, registration: dict[str, Any]) -> tuple[Optional[str], Optional[Diagnostic]]:
    """path wins over package (0010 §3); package form means the pinned
    console script sits next to the running python (the provisioned venv)."""
    path = registration.get("path")
    package = registration.get("package")
    if path and package:
        return None, Diagnostic(
            BAD_REGISTRATION,
            "error",
            f"linter {name!r} declares both `package` and `path` — pick one",
            [_label(None, f"[linters.{name}] in env.toml")],
        )
    if path:
        return str(path), None
    if package:
        exe = Path(sys.executable).parent / f"griplint-{name}"
        if exe.exists():
            return str(exe), None
        return None, Diagnostic(
            MISSING_EXECUTABLE,
            "error",
            f"linter {name!r} is registered as package {package!r} but "
            f"griplint-{name} was not found next to {sys.executable}",
            help="provisioning installs registered packages; a GRIPSACK_PYTHON "
            "bypass means you must install the linter yourself",
        )
    return None, Diagnostic(
        BAD_REGISTRATION,
        "error",
        f"linter {name!r} needs `package` or `path` in env.toml [linters]",
    )


def _from_plugin(raw: dict[str, Any], module_name: str, module_span: Optional[dict[str, Any]]) -> Diagnostic:
    """Coerce a plugin diagnostic; a label-less diagnostic gets the
    module-callsite label so it still points somewhere useful (0011 §6)."""
    labels = raw.get("labels") or []
    if not labels and module_span:
        labels = [_label(module_span, f"module {module_name!r} requested this lint")]
    return Diagnostic(
        code=str(raw.get("code", "griplint/?")),
        severity=str(raw.get("severity", "error")),
        message=str(raw.get("message", "(no message)")),
        labels=labels,
        help=raw.get("help"),
    )


def _run_linter(
    exe: str,
    name: str,
    paths: list[str],
    tool_version: Optional[str],
    module_name: str,
    module_span: Optional[dict[str, Any]],
) -> list[Diagnostic]:
    """One NDJSON exchange (0009 §2): request on stdin, diagnostic and
    response messages on stdout. Death is never silent (0009 §2.5)."""
    request = json.dumps({"op": "lint", "paths": paths, "tool_version": tool_version})
    try:
        proc = subprocess.run(
            [exe],
            input=request + "\n",
            capture_output=True,
            text=True,
            timeout=120,
        )
    except OSError as e:
        return [
            Diagnostic(
                f"griplint-{name}/E01",
                "error",
                f"cannot run {exe}: {e}",
                [_label(module_span, f"module {module_name!r} requested this lint")],
            )
        ]
    diagnostics: list[Diagnostic] = []
    responded = False
    for line in proc.stdout.splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("type") == "diagnostic":
            diagnostics.append(_from_plugin(msg.get("diagnostic", {}), module_name, module_span))
        elif msg.get("type") == "response":
            responded = True
    if not responded:
        tail = "\n".join(proc.stderr.strip().splitlines()[-3:])
        labels = [_label(None, f"stderr tail:\n{tail}" if tail else "no stderr")]
        if module_span:
            labels.append(_label(module_span, f"module {module_name!r} requested this lint"))
        diagnostics.append(
            Diagnostic(
                f"griplint-{name}/E02",
                "error",
                f"linter exited {proc.returncode} without a response",
                labels,
            )
        )
    return diagnostics


def run_lints(repo: Path, host: Optional[str], modules: list[ModuleData]) -> list[Diagnostic]:
    """Lint every module that declares ``lint=`` against the registry."""
    out: list[Diagnostic] = []
    wanted = [m for m in modules if m.lint]
    if not wanted:
        return out
    if tomllib is None:
        return [
            Diagnostic(
                BAD_REGISTRATION,
                "error",
                "linting requires tomllib — python 3.11+, or add `tomli` to [eval] deps",
            )
        ]
    registry = load_registry(repo)
    versions = _tool_versions(repo, host)
    for m in wanted:
        assert m.lint is not None
        registration = registry.get(m.lint)
        if registration is None:
            out.append(
                Diagnostic(
                    UNREGISTERED_LINTER,
                    "error",
                    f"module {m.name!r} lints with {m.lint!r}, which is not registered",
                    [_label(m.span, "lint requested here")],
                    help=f"add [linters.{m.lint}] to env.toml (0010 §3)",
                )
            )
            continue
        exe, error = _resolve_exe(m.lint, registration)
        if error is not None:
            if m.span:
                error.labels.append(_label(m.span, "lint requested here"))
            out.append(error)
            continue
        assert exe is not None
        paths = sorted(str(repo / p) for p in m.config if (repo / p).is_file())
        out.extend(_run_linter(exe, m.lint, paths, versions.get(m.name), m.name, m.span))
    return out
