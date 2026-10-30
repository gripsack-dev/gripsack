"""Evaluation: the two levels every runnable example must pass, the
behavior-level assertions, and the negative selfchecks that keep the
harness honest.

  frontend level — the SDK driver runs each disposable scaffold with
  deny-by-default flags. This SDK-only harness does not claim the core's
  captured-source approval or runtime receipt guarantees. @gripsack/core
  resolves to the installed package, exercising its public export surface.

  core level — the real `grip check` (embedded frontend and sandboxed
  Deno) must accept the same root workspace; the installed package
  wins there too (pin resolution + the sandbox's resolved-pin read grant).
  Admission is not a claim that external packages or builds were executed."""

from __future__ import annotations

import json
import os
import platform
import re
import subprocess

from .extract import Block, CheckFailure
from .fixtures import scaffold, fresh_home

def host_facts() -> dict:
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise CheckFailure(
            f"unsupported checker host {platform.system()!r}/{platform.machine()!r}; "
            "expected Linux/WSL x86_64"
        )
    return {
        "os": "linux",
        "arch": "x86_64",
        "libc": "glibc-2.36",
        "hostname": "examples-checker",
    }


def minimal_env(home, deno_dir: bool = True, path: str | None = None) -> dict:
    env = {"PATH": path or "/usr/local/bin:/usr/bin:/bin", "HOME": str(home)}
    if deno_dir:
        env["DENO_DIR"] = str(home / "deno-cache")
    return env


def frontend_eval(sdk_src, deno, repo, inputs: dict, home) -> dict:
    """Run the SDK-only driver with the core's sandbox grant profile."""
    inputs_dir = home / "inputs"
    inputs_dir.mkdir(parents=True, exist_ok=True)
    inputs_path = inputs_dir / "inputs.json"
    inputs_path.write_text(json.dumps(inputs), encoding="utf-8")
    cmd = [
        str(deno), "run", "--no-remote", "--cached-only", "--no-lock",
        "--no-config", "--node-modules-dir=manual",
        f"--import-map={sdk_src / 'deno.json'}",
        f"--allow-read={repo},{inputs_path},{sdk_src}",
        str(sdk_src / "src/cli.ts"), str(repo), "--inputs", str(inputs_path),
    ]
    res = subprocess.run(
        cmd, cwd=repo, env=minimal_env(home), capture_output=True, text=True,
    )
    if res.returncode != 0:
        raise CheckFailure(
            f"frontend eval failed (exit {res.returncode}):\n"
            f"{res.stderr.strip()[-2000:]}"
        )
    try:
        return json.loads(res.stdout)
    except json.JSONDecodeError as e:
        raise CheckFailure(f"frontend eval printed no envelope: {e}\n{res.stdout[:500]}")


def core_check(grip, deno, repo, home, expect_outputs: dict) -> int:
    """Run real `grip check` and validate the complete output catalog."""
    env = minimal_env(home, deno_dir=False)
    env.update({
        "GRIPSACK_HOME": str(home / ".local/share/gripsack"),
        "GRIPSACK_DENO": str(deno),
    })
    inspected = subprocess.run(
        [str(grip), "trust", "inspect", str(repo), "--json"],
        cwd=repo, env=env, capture_output=True, text=True,
    )
    if inspected.returncode:
        raise CheckFailure(f"fixture source inspection failed:\n{inspected.stderr[-2000:]}")
    source = json.loads(inspected.stdout)
    approved = subprocess.run(
        [str(grip), "trust", "add", str(repo),
         "--bundle", source["bundle_digest"], "--policy", source["policy_digest"]],
        cwd=repo, env=env, capture_output=True, text=True,
    )
    if approved.returncode:
        raise CheckFailure(f"fixture source approval failed:\n{approved.stderr[-2000:]}")
    cmd = [str(grip), "check", "--json"]
    res = subprocess.run(
        cmd, cwd=repo, env=env,
        capture_output=True, text=True,
    )
    if res.returncode != 0:
        raise CheckFailure(
            f"grip check failed (exit {res.returncode}):\n"
            f"{res.stdout.strip()[-800:]}\n{res.stderr.strip()[-2000:]}"
        )
    try:
        report = json.loads(res.stdout)
    except json.JSONDecodeError as e:
        raise CheckFailure(f"grip check printed no JSON report: {e}")
    assert_outputs(report.get("outputs"), expect_outputs, "grip check")
    return len(report["outputs"])


# --------------------------------------------------------------------------
# behavior-level assertions (emitted IR, accepted counts — never doc
# source text)
# --------------------------------------------------------------------------

def dig(obj, path: str):
    """Navigate 'steps[1].action.kind'; [n] indexes, .name selects."""
    for part in path.split("."):
        if "[" in part:
            name, idx = part.split("[")
            if name:
                obj = obj[name]
            obj = obj[int(idx[:-1])]
        else:
            obj = obj[part]
    return obj




def check_ir_point(ir: dict, point: tuple, where: str) -> None:
    kind, mod, path, expected = point
    if kind == "output":
        nodes = {node["name"]: node for node in ir["workspace"]["outputs"]}
    else:
        raise CheckFailure(f"{where}: unknown IR point kind {kind!r}")
    if mod not in nodes:
        raise CheckFailure(f"{where}: no {kind} {mod!r} in IR (have {sorted(nodes)})")
    substring = path.endswith("~")
    try:
        actual = dig(nodes[mod], path.removesuffix("~"))
    except (KeyError, IndexError, TypeError) as e:
        raise CheckFailure(f"{where}: missing {kind} field {mod}.{path}: {e}") from e
    if substring:
        # substring expectation (for script bodies)
        if expected not in actual:
            raise CheckFailure(f"{where}: {mod}.{path} does not contain {expected!r}")
        return
    if isinstance(expected, dict):
        missing = {k: v for k, v in expected.items() if actual.get(k) != v}
        if missing:
            raise CheckFailure(
                f"{where}: {mod}.{path} mismatch: {missing} (got {actual})"
            )
        return
    if actual != expected:
        raise CheckFailure(f"{where}: {mod}.{path} = {actual!r}, want {expected!r}")


def assert_outputs(outputs, expect: dict, where: str) -> None:
    """Compare the whole catalog, including duplicate names and output kinds."""
    if not isinstance(outputs, list) or any(
        not isinstance(node, dict)
        or not isinstance(node.get("name"), str)
        or not isinstance(node.get("kind"), str)
        for node in outputs
    ):
        raise CheckFailure(f"{where}: no valid named-output catalog: {outputs!r}")
    got = sorted((node["name"], node["kind"]) for node in outputs)
    want = sorted(expect.items())
    if got != want:
        raise CheckFailure(f"{where}: named outputs {got}, want {want}")


def assert_envelope(env: dict, expect_outputs: dict, where: str) -> None:
    if env.get("diagnostics"):
        raise CheckFailure(f"{where}: frontend diagnostics: {env['diagnostics']}")
    ir = env.get("ir")
    if not isinstance(ir, dict) or set(ir) != {"ir_version", "host", "workspace"}:
        raise CheckFailure(f"{where}: expected workspace-only IR, got {ir!r}")
    if ir["ir_version"] != 10:
        raise CheckFailure(f"{where}: expected IR10, got {ir['ir_version']!r}")
    workspace = ir["workspace"]
    if not isinstance(workspace, dict):
        raise CheckFailure(f"{where}: no workspace in IR")
    assert_outputs(workspace.get("outputs"), expect_outputs, where)


# --------------------------------------------------------------------------
# the positive pipeline
# --------------------------------------------------------------------------

def run_example(entry: dict, block: Block, ctx) -> list[str]:
    """Evaluate the actual published root entrypoint, then admit it in core."""
    failures: list[str] = []
    expect = entry["expect_outputs"]
    repo = scaffold(entry, block, ctx["base"], ctx["sdk_pkg"])
    home = fresh_home(ctx["base"], f"{entry['id']}-frontend")
    inputs = {"version": 1, "host": "example", "facts": ctx["facts"],
              "tags": [], "probes": {}, "settings": {}}
    where = f"{entry['id']} frontend"
    try:
        envelope = frontend_eval(ctx["sdk_src"], ctx["deno"], repo, inputs, home)
        assert_envelope(envelope, expect, where)
        for point in entry.get("ir", []):
            check_ir_point(envelope["ir"], point, where)
        for node in envelope["ir"]["workspace"]["outputs"]:
            span = node.get("span", {})
            line = span.get("line")
            if (not str(span.get("file", "")).endswith("/gripsack.ts")
                    or not isinstance(line, int)
                    or not 1 <= line <= len(block.code.splitlines())
                    or not isinstance(span.get("col"), int) or span["col"] < 1):
                raise CheckFailure(f"{where}: invalid source span for {node['name']}: {span}")
            declaration = re.compile(
                rf'\b(?:pkg|environment|profile|task|check|hook|service|schedule)\("{re.escape(node["name"])}"'
            )
            expected_lines = [i for i, text in enumerate(block.code.splitlines(), 1)
                              if declaration.search(text)]
            if expected_lines != [line]:
                raise CheckFailure(
                    f"{where}: {node['name']} span line {line} does not identify "
                    f"its published declaration (lines {expected_lines})"
                )
    except CheckFailure as e:
        failures.append(str(e))
    try:
        core_check(ctx["grip"], ctx["deno"], repo,
                   fresh_home(ctx["base"], f"{entry['id']}-core"), expect)
    except CheckFailure as e:
        failures.append(str(e))
    return failures
