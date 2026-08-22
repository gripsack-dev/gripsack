"""Verify checks — smoke contracts, not a test framework (0007 §verify)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional


@dataclass(frozen=True)
class Verify:
    kind: str
    args: dict[str, Any]

    def to_ir(self) -> dict[str, Any]:
        return {"kind": self.kind, **self.args}


def verify_binary(path: str, args: Optional[list[str]] = None) -> Verify:
    """A built binary runs (default sanity: `--version`-style)."""
    ir_args: dict[str, Any] = {"path": path}
    if args:
        ir_args["args"] = args
    return Verify("binary_runs", ir_args)


def verify_file(path: str) -> Verify:
    return Verify("file_exists", {"path": path})


def verify_shell(script: str) -> Verify:
    return Verify("shell", {"script": script})
