"""Behavioral calibration for workspace return values and packaged SDK selection."""
from __future__ import annotations

import json

from .evaluate import frontend_eval, run_example
from .extract import Block, CheckFailure
from .fixtures import CORE_PKG_DIR, fresh_home

CANARY_PKG_JSON = json.dumps({
    "name": "@gripsack/core", "version": "0.0.0-canary", "type": "module",
    "exports": {".": "./index.js", "./advanced": "./advanced.js"},
})
CANARY_INDEX = """export const defineWorkspace = (fn) => fn;
export const workspace = (spec) => ({ ...spec, pin: "authoring-canary" });
"""
CANARY_ADVANCED = """export const IR_VERSION = 9;
export const parseInputs = JSON.parse;
export const createProbeBuilder = () => ({ probe: {}, requests: [] });
export const emitWorkspaceIr = (value) => JSON.stringify({
  ir_version: 999, pin: value.pin, compiler: "compiler-canary",
});
"""
CANARY_WORKSPACE = """import { defineWorkspace, workspace } from "@gripsack/core";
export default defineWorkspace(() => workspace({ outputs: [] }));
"""


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


def pin_canary(ctx) -> list[str]:
    base = ctx["base"] / "selfcheck" / "pin-canary"
    repo = base / "repos" / "canary"
    repo.mkdir(parents=True)
    (repo / "gripsack.ts").write_text(CANARY_WORKSPACE, encoding="utf-8")
    pkg = repo / CORE_PKG_DIR
    pkg.mkdir(parents=True)
    for name, content in (("package.json", CANARY_PKG_JSON),
                          ("index.js", CANARY_INDEX), ("advanced.js", CANARY_ADVANCED)):
        (pkg / name).write_text(content, encoding="utf-8")
    inputs = {"version": 1, "host": "canary", "facts": ctx["facts"],
              "tags": [], "probes": {}, "settings": {}}
    try:
        envelope = frontend_eval(ctx["sdk_src"], ctx["deno"], repo, inputs,
                                 fresh_home(base, "canary"))
        ir = envelope.get("ir", {})
        if ir != {"ir_version": 999, "pin": "authoring-canary", "compiler": "compiler-canary"}:
            return [f"pin canary: packaged authoring/compiler API bypassed: {json.dumps(ir)[:200]}"]
    except CheckFailure as e:
        return [f"pin canary: eval failed: {e}"]
    return []
