"""Golden IR snapshot corpus (plan/0013 D1) — successor of the
dual-frontend parity corpus: with one frontend there is no cross-language
shadow, but the regression value stays. Every fixture env under
fixtures/envs/ is evaluated by the SAME driver invocation the core uses
(plan/0013 D2) with a FIXED inputs file, and the emitted envelope is
diffed byte-exact against the snapshot under fixtures/golden/.

Determinism: facts come from the inputs file (never this host), tags are
fixed, and module order is the host entrypoint's import order (sorted).

Span normalization: provenance spans (`span` keys) carry authoring
file:line:col — they move when a fixture is edited without changing the
IR's meaning, so every `span` key is stripped recursively and the rest
is compared canonically (sorted keys, indent 2). Everything else —
module fields, entry ordering, probe_requests — must match exactly.

Regenerate after an INTENDED IR or fixture change:

    REGEN_GOLDEN=1 pytest e2e/test_golden.py

and review the snapshot diff like any generated artifact.
"""

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from conftest import GRIP, grip, make_env_repo

FIXTURES = Path(__file__).parent / "fixtures" / "envs"
GOLDEN = Path(__file__).parent / "fixtures" / "golden"

# Fixed host inputs (plan/0013 D4): the corpus must not depend on the
# machine it runs on. hostname never reaches the IR; it is part of the
# envelope's facts regardless.
INPUTS = {
    "version": 1,
    "host": "testhost",
    "facts": {
        "os": "linux",
        "arch": "x86_64",
        "libc": "glibc-2.36",
        "hostname": "box",
    },
    "tags": [],
    "probes": {},
    "settings": {},
}


def strip_spans(node):
    """Remove every `span` key, recursively — the only normalization."""
    if isinstance(node, dict):
        return {k: strip_spans(v) for k, v in node.items() if k != "span"}
    if isinstance(node, list):
        return [strip_spans(v) for v in node]
    return node


def canonical(envelope: dict) -> str:
    return json.dumps(
        {
            "ir": strip_spans(envelope["ir"]),
            "diagnostics": strip_spans(envelope.get("diagnostics", [])),
            "probe_requests": strip_spans(envelope.get("probe_requests", [])),
        },
        indent=2,
        sort_keys=True,
    ) + "\n"


def find_deno() -> str | None:
    return os.environ.get("GRIPSACK_DENO") or shutil.which("deno")


def materialize_frontend(sandbox: Path) -> Path:
    """Force the core to materialize the embedded frontend by running
    any eval command once; return its ts-<version>/ directory."""
    repo = make_env_repo(
        sandbox / "probe",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("c", {
  config: { "x.toml": trackedCopy("~/.config/x.toml") },
});
""",
    )
    (repo / "x.toml").write_text("probe = true\n")
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    front = sandbox / ".local/share/gripsack/frontend"
    drivers = sorted(front.glob("ts-*/src/cli.ts"))
    assert drivers, f"embedded frontend not materialized under {front}"
    return drivers[-1].parents[1]


def evaluate(deno: str, frontend: Path, repo: Path, sandbox: Path) -> dict:
    """The plan/0013 D2 spawn contract, verbatim: no env, no network,
    no subprocesses — read-only within repo, inputs dir, and the
    provisioned frontend."""
    inputs_dir = sandbox / "inputs"
    inputs_dir.mkdir(exist_ok=True)
    inputs = inputs_dir / "inputs.json"
    inputs.write_text(json.dumps(INPUTS))
    env = {
        "HOME": str(sandbox),
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "DENO_DIR": str(sandbox / ".local/share/gripsack/deno-cache"),
    }
    out = subprocess.run(
        [
            deno, "run", "--no-remote", "--cached-only", "--no-lock",
            f"--allow-read={repo},{inputs_dir},{frontend}",
            str(frontend / "src" / "cli.ts"),
            str(repo),
            "--inputs",
            str(inputs),
        ],
        cwd=repo,
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
    )
    assert out.returncode == 0, out.stderr
    lines = [ln for ln in out.stdout.splitlines() if ln.strip()]
    return json.loads(lines[-1])


@pytest.mark.parametrize("env", sorted(p.name for p in FIXTURES.iterdir()))
def test_golden_ir_snapshot(sandbox, env, request):
    deno = find_deno()
    if not deno:
        pytest.skip("deno not installed (the e2e gate image ships it)")
    frontend = materialize_frontend(sandbox)

    repo = sandbox / env
    shutil.copytree(FIXTURES / env, repo)
    envelope = evaluate(deno, frontend, repo, sandbox)
    actual = canonical(envelope)

    golden = GOLDEN / f"{env}.ir.json"
    if os.environ.get("REGEN_GOLDEN") == "1":
        GOLDEN.mkdir(parents=True, exist_ok=True)
        golden.write_text(actual)
        return
    if not golden.exists():
        pytest.fail(
            f"no golden snapshot for {env} — generate it with "
            f"REGEN_GOLDEN=1 pytest e2e/test_golden.py -k {env}"
        )
    expected = golden.read_text()
    if actual != expected:
        golden.with_suffix(".ir.json.actual").write_text(actual)
    assert actual == expected, (
        f"{env}: emitted envelope drifted from the golden snapshot "
        f"(span-normalized, canonical JSON).\nExpected: {golden}\n"
        f"Actual written next to it as .actual — if the change is "
        f"intended, regenerate with REGEN_GOLDEN=1 pytest e2e/test_golden.py"
    )
