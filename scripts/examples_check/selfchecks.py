"""Behavioral calibration for required workspace return values."""
from __future__ import annotations

from .evaluate import run_example
from .extract import Block


def selfcheck_workspace(entry: dict, block: Block, ctx) -> list[str]:
    """Drop the root factory's return; require the same real pipeline to reject it."""
    mutated = block.code.replace("() => workspace(", "() => void workspace(", 1)
    if mutated == block.code:
        return [f"selfcheck {entry['id']}: missing-return mutation no longer applies"]
    negative_ctx = dict(ctx, base=ctx["base"] / "selfcheck" / entry["id"])
    negative_ctx["base"].mkdir(parents=True, exist_ok=True)
    mutated_block = Block(block.file, block.line, block.title, block.fenced, mutated)
    if run_example(entry, mutated_block, negative_ctx):
        return []
    return [f"selfcheck {entry['id']}: workspace factory missing its return was accepted"]
