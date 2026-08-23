"""Verify checks — smoke contracts, not a test framework (0007 §verify)."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Any, Optional


class VerifyKind(str, Enum):
    """The closed set of verify checks (0007 §verify)."""

    BINARY_RUNS = "binary_runs"
    FILE_EXISTS = "file_exists"
    SHELL = "shell"
    FILE_DEPLOYED = "file_deployed"


@dataclass(frozen=True)
class Verify:
    kind: VerifyKind
    args: dict[str, Any]

    def __post_init__(self) -> None:
        object.__setattr__(self, "kind", VerifyKind(self.kind))

    def to_ir(self) -> dict[str, Any]:
        return {"kind": self.kind.value, **self.args}


def verify_binary(path: str, args: Optional[list[str]] = None) -> Verify:
    """A built binary runs (default sanity: `--version`-style)."""
    ir_args: dict[str, Any] = {"path": path}
    if args:
        ir_args["args"] = args
    return Verify(VerifyKind.BINARY_RUNS, ir_args)


def verify_file(path: str) -> Verify:
    return Verify(VerifyKind.FILE_EXISTS, {"path": path})


def verify_shell(script: str) -> Verify:
    return Verify(VerifyKind.SHELL, {"script": script})


def verify_deployed(path: str) -> Verify:
    """Check a deployed *destination* — for config-only modules, where
    payload-relative verifies don't apply."""
    return Verify(VerifyKind.FILE_DEPLOYED, {"path": path})
