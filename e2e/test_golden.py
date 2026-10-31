"""Golden IR snapshot corpus (plan/0013 D1) — successor of the
dual-frontend parity corpus: with one frontend there is no cross-language
shadow, but the regression value stays. Every fixture env under
fixtures/envs/ is evaluated by the SDK driver with a FIXED inputs file.
This SDK-only harness uses the core's grant profile, but does not claim the
core's source approval, retained runtime or evaluation receipt guarantees.

Determinism: facts come from the inputs file, never this host. Root
workspace declaration order is preserved. Diagnostic spans and Bash line
maps remain part of the snapshots so current source provenance is covered.

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

from conftest import grip, make_workspace_repo

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


def strip_provenance(node):
    """Exclude diagnostic-only source locations from semantic snapshots."""
    if isinstance(node, dict):
        return {
            key: strip_provenance(value)
            for key, value in node.items()
            if key not in ("span", "line_map")
        }
    if isinstance(node, list):
        return [strip_provenance(value) for value in node]
    return node


def relative_provenance(node, repo):
    """Keep source locations, removing only the disposable repository prefix."""
    if isinstance(node, dict):
        result = {key: relative_provenance(value, repo) for key, value in node.items()}
        if "file" in result and "line" in result:
            path = Path(result["file"])
            if path.is_absolute() and path.is_relative_to(repo):
                result["file"] = path.relative_to(repo).as_posix()
        return result
    if isinstance(node, list):
        return [relative_provenance(value, repo) for value in node]
    return node


def canonical(envelope: dict) -> str:
    return json.dumps(
        {
            "ir": envelope["ir"],
            "diagnostics": envelope.get("diagnostics", []),
            "probe_requests": envelope.get("probe_requests", []),
        },
        indent=2,
        sort_keys=True,
    ) + "\n"


def find_deno() -> str | None:
    return os.environ.get("GRIPSACK_DENO") or shutil.which("deno")


def materialize_frontend(sandbox: Path) -> Path:
    """Force the core to materialize the embedded frontend by running
    any eval command once; return its ts-<version>/ directory."""
    repo = make_workspace_repo(
        sandbox / "probe",
        'import { profile } from "@gripsack/core";\n'
        'export default profile("probe", {});\n',
    )
    out = grip("check", cwd=repo)
    assert out.returncode == 0, out.stderr
    front = sandbox / ".local/share/gripsack/frontend"
    drivers = sorted(front.glob("ts-*/src/cli.ts"))
    assert drivers, f"embedded frontend not materialized under {front}"
    return drivers[-1].parents[1]


def evaluate(
    deno: str, frontend: Path, repo: Path, sandbox: Path, inputs_override: dict | None = None,
) -> dict:
    """Direct SDK evaluation with deny-by-default grants and fixed facts."""
    inputs_dir = sandbox / "inputs"
    inputs_dir.mkdir(exist_ok=True)
    inputs_file = inputs_dir / "inputs.json"
    inputs_file.write_text(json.dumps(INPUTS if inputs_override is None else inputs_override))
    env = {
        "HOME": str(sandbox),
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "DENO_DIR": str(sandbox / ".local/share/gripsack/deno-cache"),
    }
    out = subprocess.run(
        [
            deno, "run", "--no-remote", "--cached-only", "--no-lock",
            "--no-config", "--node-modules-dir=manual",
            f"--import-map={frontend / 'deno.json'}",
            f"--allow-read={repo},{inputs_file},{frontend}",
            str(frontend / "src" / "cli.ts"),
            str(repo),
            "--inputs",
            str(inputs_file),
        ],
        cwd=repo,
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
    )
    assert out.returncode == 0, out.stderr
    lines = [ln for ln in out.stdout.splitlines() if ln.strip()]
    return relative_provenance(json.loads(lines[-1]), repo.resolve())


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
        f"(canonical JSON including source provenance).\nExpected: {golden}\n"
        f"Actual written next to it as .actual — if the change is "
        f"intended, regenerate with REGEN_GOLDEN=1 pytest e2e/test_golden.py"
    )


def test_workspace_golden_rejects_a_changed_typed_command_input(sandbox):
    """An admitted semantic edit must drift the golden for its own
    reason, not because a Bash source line or span moved."""
    deno = find_deno()
    if not deno:
        pytest.skip("deno not installed (the e2e gate image ships it)")
    frontend = materialize_frontend(sandbox)
    repo = sandbox / "changed-command"
    shutil.copytree(FIXTURES / "native-workspace", repo)
    source = repo / "gripsack.ts"
    original = source.read_text()
    input_value = "from a typed environment value"
    changed_value = "different declared build input"
    assert original.count(input_value) == 1, "semantic mutant anchor lost"
    source.write_text(original.replace(input_value, changed_value))

    actual = json.loads(canonical(evaluate(deno, frontend, repo, sandbox)))
    expected = json.loads((GOLDEN / "native-workspace.ir.json").read_text())

    def command_env(document):
        build = next(
            output for output in document["ir"]["workspace"]["outputs"]
            if output["name"] == "invoke"
        )
        return build["steps"][0]["command"]["env"]["INPUT"]

    assert command_env(expected)["value"] == input_value
    assert command_env(actual)["value"] == changed_value
    command_env(actual)["value"] = input_value
    assert actual == expected, "golden drift must be attributable to the changed command input"


