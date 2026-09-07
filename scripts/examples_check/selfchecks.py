"""Negative selfchecks: the harness must FAIL when the thing it
guards for is broken — the same calibration discipline as the model
gate's deliberately failing configs.

  selfcheck_factory — the missing-`return` regression in a module
  factory: modules silently vanish from the evaluated IR (falsy
  entries drop by design), so the behavioral expectation fails.

  pin_canary — the installed node_modules package must be what
  answers: a marker package with ir_version 999 has to surface in the
  envelope. If resolution silently used the embedded copy, every
  example would still pass while testing the wrong artifact.
"""

from __future__ import annotations

import json

from .evaluate import frontend_eval, run_example
from .extract import Block, CheckFailure
from .fixtures import CORE_PKG_DIR, fresh_home

# A minimal but honest marker package (the frontend's own pin-parity
# test pattern): enough API surface for the driver, with an IR marker
# that proves WHICH copy answered.
CANARY_PKG_JSON = (
    '{"name":"@gripsack/core","version":"0.0.0-canary","type":"module",'
    '"main":"index.js"}'
)
CANARY_INDEX = """export const parseInputs = (_t) => ({
  host: "canary", facts: { os: "linux", arch: "x86_64", libc: null, hostname: "c" },
  tags: [], probes: {}, settings: {},
});
export const createProbeBuilder = () => ({
  probe: { executable: () => false, file_exists: () => false },
  requests: [],
});
export const emitIr = (_env, _facts, _tags) =>
  JSON.stringify({ ir_version: 999, pin: "canary" }, null, 2);
export const defineEnv = (fn) => fn;
export const mergeTags = (a, b) => [...(a ?? []), ...b];
export const module = (name, spec) => ({ __gripsack: "module", name, ir: spec });
export const symlink = (to) => ({ to, mode: "owned" });
"""
CANARY_HOST = (
    'import { defineEnv, module } from "@gripsack/core";\n'
    "\n"
    'const m = module("canary", {});\n'
    "\n"
    "export default defineEnv((ctx) => ({ modules: [m] }));\n"
)


def selfcheck_factory(entry: dict, block: Block, ctx) -> list[str]:
    """The missing-return regression, proven by checker behavior.

    Inject the defect (drop the `return` before `module(`), run the
    SAME pipeline, and require it to FAIL — the factory's modules must
    vanish from the evaluated IR (falsy entries drop silently). No
    source-text matching of the doc is involved in detection.
    """
    mutated = block.code.replace("return module(", "module(", 1)
    if mutated == block.code:
        return [
            f"selfcheck {entry['id']}: mutation did not apply — the "
            "example no longer contains a `return module(` factory body; "
            "update the selfcheck"
        ]
    mutated_block = Block(block.file, block.line, block.title, block.fenced, mutated)
    negative_ctx = dict(ctx, base=ctx["base"] / "selfcheck" / entry["id"])
    negative_ctx["base"].mkdir(parents=True, exist_ok=True)
    failures = run_example(entry, mutated_block, negative_ctx)
    if failures:
        return []  # negative detected — the guard holds
    return [
        f"selfcheck {entry['id']}: a factory missing `return module(...)` "
        "was ACCEPTED — the executable-docs check would not catch the "
        "regression it exists for"
    ]


def pin_canary(ctx) -> list[str]:
    base = ctx["base"] / "selfcheck" / "pin-canary"
    repo = base / "repos" / "canary"
    (repo / "hosts").mkdir(parents=True, exist_ok=True)
    (repo / "env.toml").write_text('[env]\nname = "canary"\n', encoding="utf-8")
    (repo / "hosts" / "canary.ts").write_text(CANARY_HOST, encoding="utf-8")
    pkg = repo / CORE_PKG_DIR
    pkg.mkdir(parents=True)
    (pkg / "package.json").write_text(CANARY_PKG_JSON, encoding="utf-8")
    (pkg / "index.js").write_text(CANARY_INDEX, encoding="utf-8")
    home = fresh_home(base, "canary")
    inputs = {"version": 1, "host": "canary", "facts": ctx["facts"],
              "tags": [], "probes": {}, "settings": {}}
    try:
        envelope = frontend_eval(ctx["sdk_src"], ctx["deno"], repo, inputs, home)
        ir = envelope.get("ir", {})
        if ir.get("ir_version") != 999 or ir.get("pin") != "canary":
            return [
                "pin canary: the installed node_modules package did NOT "
                f"answer (ir: {json.dumps(ir)[:120]}) — the packaged SDK "
                "is being bypassed; executable-docs runs would test the "
                "embedded copy instead of the package"
            ]
    except CheckFailure as e:
        return [f"pin canary: eval failed: {e}"]
    return []
