"""Evaluation: the two levels every runnable example must pass, the
behavior-level assertions, and the negative selfchecks that keep the
harness honest.

  frontend level — the SDK driver runs each disposable scaffold with
  deny-by-default flags. This SDK-only harness does not claim the core's
  captured-source approval or runtime receipt guarantees. @gripsack/core
  resolves to the installed package, exercising its public export surface.

  core level — the real `grip check` (embedded frontend, sandboxed
  deno, two-stage probe binding) must accept the same repo; the
  installed package wins there too (pin resolution + the sandbox's
  resolved-pin read grant)."""

from __future__ import annotations

import json
import os
import platform
import re
import shutil
import subprocess

from .extract import Block, CheckFailure
from .fixtures import scaffold, fresh_home, write_executable, write_fixture

def host_facts() -> dict:
    os_name = {"Linux": "linux", "Darwin": "darwin"}.get(platform.system())
    if os_name is None:
        raise CheckFailure(
            f"unsupported checker host OS {platform.system()!r} — "
            "expectations are derived for linux/darwin"
        )
    machine = platform.machine()
    arch = "aarch64" if machine in ("arm64", "aarch64") else "x86_64"
    return {
        "os": os_name,
        "arch": arch,
        "libc": "glibc-2.36" if os_name == "linux" else None,
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


def core_check(grip, deno, repo, host: str | None, home,
               path: str | None = None, expect_outputs: dict | None = None) -> int:
    """Run real `grip check`; validate workspace names/kinds or return module count."""
    env = minimal_env(home, deno_dir=False, path=path)
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
    cmd = [str(grip), "check"]
    if host is not None:
        cmd += ["--host", host]
    if expect_outputs is not None:
        cmd.append("--json")
    res = subprocess.run(
        cmd, cwd=repo, env=env,
        capture_output=True, text=True,
    )
    if res.returncode != 0:
        raise CheckFailure(
            f"grip check failed (exit {res.returncode}):\n"
            f"{res.stdout.strip()[-800:]}\n{res.stderr.strip()[-2000:]}"
        )
    if expect_outputs is not None:
        try:
            report = json.loads(res.stdout)
        except json.JSONDecodeError as e:
            raise CheckFailure(f"grip check printed no JSON report: {e}")
        assert_outputs(report.get("outputs"), expect_outputs, "grip check")
        return len(report["outputs"])
    m = re.search(r"check: ok (\d+) modules", res.stdout)
    if not m:
        raise CheckFailure(f"no module count in check output: {res.stdout[-500:]}")
    return int(m.group(1))


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


def resolve_expect(names, facts: dict) -> list[str]:
    out = []
    for n in names:
        if isinstance(n, tuple):
            name, os_gate = n
            if facts["os"] != os_gate:
                continue
            out.append(name)
        else:
            out.append(n)
    return out


def check_ir_point(ir: dict, point: tuple, where: str) -> None:
    kind, mod, path, expected = point
    if kind == "output":
        nodes = {node["name"]: node for node in ir["workspace"]["outputs"]}
    elif kind == "module":
        nodes = ir["modules"]
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


def assert_envelope(env: dict, expect: list[str], where: str,
                    expect_outputs: dict | None = None) -> None:
    if env.get("diagnostics"):
        raise CheckFailure(f"{where}: frontend diagnostics: {env['diagnostics']}")
    if expect_outputs is not None:
        ir = env.get("ir")
        if not isinstance(ir, dict) or "modules" in ir:
            raise CheckFailure(f"{where}: expected workspace IR, got {ir!r}")
        workspace = ir.get("workspace")
        if not isinstance(workspace, dict):
            raise CheckFailure(f"{where}: no workspace in IR")
        assert_outputs(workspace.get("outputs"), expect_outputs, where)
        return
    got = sorted(env["ir"]["modules"])
    want = sorted(expect)
    if got != want:
        dropped = sorted(set(want) - set(got))
        hint = (
            " — modules vanished after eval: falsy entries drop silently "
            "by design, which is exactly what a factory that forgot "
            "`return module(...)` produces"
            if dropped
            else ""
        )
        raise CheckFailure(f"{where}: IR modules {got}, want {want}{hint}")


# --------------------------------------------------------------------------
# the positive pipeline
# --------------------------------------------------------------------------

def run_example(entry: dict, block: Block, ctx) -> list[str]:
    """Full positive pipeline for one runnable example."""
    failures: list[str] = []
    facts = ctx["facts"]
    expect = resolve_expect(entry.get("expect_modules", []), facts)
    expect_outputs = entry.get("expect_outputs") if entry["kind"] == "workspace" else None
    if entry["kind"] == "workspace" and not expect_outputs:
        raise CheckFailure(f"{entry['id']}: workspace needs explicit named-output expectations")
    stages: list[tuple[str, dict]] = [("default", {})]

    # extra tag stages: per-file conditional selection must actually
    # flip the chosen source in the emitted IR
    for extra in entry.get("tag_stages", []):
        stages.append((f"tags={extra['tags']}", {"tags": extra["tags"]}))

    for stage_name, overrides in stages:
        repo = scaffold(entry, block, ctx["base"], ctx["sdk_pkg"])
        home = fresh_home(ctx["base"], f"{entry['id']}-{stage_name}".replace("/", "_"))
        inputs = {"version": 1, "host": "", "facts": facts, "tags": [],
                  "probes": {}, "settings": {}}
        inputs.update(overrides)
        # The inputs protocol requires a nonempty host even though root
        # workspaces do not select hosts/<host>.ts.
        inputs["host"] = entry["host"] if entry["kind"] == "host" else "example"
        where = f"{entry['id']} [{stage_name}] frontend"
        try:
            envelope = frontend_eval(ctx["sdk_src"], ctx["deno"], repo, inputs, home)
            assert_envelope(envelope, expect, where, expect_outputs)
            if "probe_requests" in entry and stage_name == "default":
                got = [[p["kind"], p["name"]] for p in envelope.get("probe_requests", [])]
                if got != entry["probe_requests"]:
                    raise CheckFailure(f"{where}: probe requests {got}, want {entry['probe_requests']}")
            for point in entry.get("ir", []):
                check_ir_point(envelope["ir"], point, where)
            for extra in entry.get("tag_stages", []):
                if f"tags={extra['tags']}" == stage_name:
                    check_ir_point(envelope["ir"], extra["point"], where)
        except CheckFailure as e:
            failures.append(str(e))

    # bound-probe stage (frontend fixpoint: requests answered, none re-asked)
    if "expect_modules_bound" in entry:
        repo = scaffold(entry, block, ctx["base"], ctx["sdk_pkg"])
        home = fresh_home(ctx["base"], f"{entry['id']}-bound")
        inputs = {"version": 1, "host": entry["host"], "facts": facts,
                  "tags": [], "settings": {},
                  "probes": {"executable:nvidia-smi": True}}
        where = f"{entry['id']} [bound-probe] frontend"
        try:
            envelope = frontend_eval(ctx["sdk_src"], ctx["deno"], repo, inputs, home)
            assert_envelope(envelope, resolve_expect(entry["expect_modules_bound"], facts), where)
            if envelope.get("probe_requests"):
                raise CheckFailure(
                    f"{where}: bound probes re-requested {envelope['probe_requests']}"
                )
        except CheckFailure as e:
            failures.append(str(e))

    # core level: the real grip check on the same scaffolded repo
    host = (entry["host"] if entry["kind"] == "host"
            else None if entry["kind"] == "workspace" else "example")
    for label, path_env in (("core", None), ("core-bound", ctx["fake_probe_path"])):
        if label == "core-bound" and "expect_modules_bound" not in entry:
            continue
        repo = scaffold(entry, block, ctx["base"], ctx["sdk_pkg"])
        home = fresh_home(ctx["base"], f"{entry['id']}-{label}")
        if label == "core-bound":
            want = resolve_expect(entry["expect_modules_bound"], facts)
        else:
            # the probe binds against the REAL PATH the run gets — if
            # the checker host genuinely has nvidia-smi, cuda belongs
            # in the expectation (CI runners do not)
            want = list(expect)
            if "expect_modules_bound" in entry and ctx["clean_has_probe"]:
                want += [n for n in resolve_expect(entry["expect_modules_bound"], facts)
                         if n not in want]
        try:
            count = core_check(ctx["grip"], ctx["deno"], repo, host, home,
                               path=path_env or ctx["clean_path"],
                               expect_outputs=expect_outputs)
            want_count = len(expect_outputs) if expect_outputs is not None else len(want)
            if count != want_count:
                raise CheckFailure(
                    f"{entry['id']} [{label}]: grip check accepted {count} "
                    f"entries, want {want_count}"
                )
        except CheckFailure as e:
            failures.append(str(e))
    return failures
