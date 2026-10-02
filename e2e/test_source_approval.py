"""0048 R1: real Deno consumes approved copies, never later worktree bytes.

These gate cases deliberately bypass conftest's disposable-fixture approval.
Git operations are local; every source, runtime wrapper and receipt is sandboxed.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

import pytest
from conftest import GRIP, grip, make_env_repo


def git(repo, *arguments):
    result = subprocess.run(
        ["git", "-c", "protocol.file.allow=always", "-c", "user.name=Fixture",
         "-c", "user.email=fixture@example.invalid", *arguments],
        cwd=repo, env=dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null"),
        capture_output=True, text=True, timeout=30,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    return result.stdout.strip()


def inspect_source(repo):
    result = grip("trust", "inspect", str(repo), "--json", approve=False)
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def approve_source(repo):
    source = inspect_source(repo)
    result = grip("trust", "add", str(repo), "--bundle", source["bundle_digest"],
                  "--policy", source["policy_digest"], approve=False)
    assert result.returncode == 0, result.stderr
    return source


def receipt_paths():
    return set((Path(os.environ["GRIPSACK_HOME"]) / "evaluations").glob("*.json"))


def new_receipt(before):
    added = receipt_paths() - before
    assert len(added) == 1, added
    path, = added
    assert path.stat().st_mode & 0o777 == 0o600
    receipt = json.loads(path.read_text())
    inspected = grip("trust", "inspect", "--receipt", receipt["id"], "--json", approve=False)
    assert inspected.returncode == 0, inspected.stderr
    assert json.loads(inspected.stdout) == receipt
    return receipt


def checked(repo, expected, source):
    before = receipt_paths()
    result = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
    assert result.returncode == 0, result.stdout + result.stderr
    assert json.loads(result.stdout)["modules"] == [expected]
    receipt = new_receipt(before)
    assert receipt["outcome"] == "completed"
    assert receipt["source"] == source["bundle_digest"]
    assert receipt["policy_digest"] == source["policy_digest"]
    assert receipt["policy"] == source["policy"]
    assert [(r["number"], r["process"]["exit_code"]) for r in receipt["rounds"]] == [(1, 0)]
    return receipt


def rejected(repo):
    before = receipt_paths()
    result = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
    assert result.returncode == 1, result.stdout + result.stderr
    receipt = new_receipt(before)
    assert receipt["outcome"] == "rejected"
    assert receipt["rounds"] == []
    return receipt


def value_file(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"export default {json.dumps(value)};\n")


def value_repo(root, imported="../value.ts"):
    return make_env_repo(root, {
        "selected": 'import { module } from "@gripsack/core";\n'
                    f'import value from {json.dumps(imported)};\n'
                    'export default module(value, { install: [] });\n',
    })


@pytest.mark.parametrize("change", ["staged", "unstaged", "untracked", "ignored", "symlink", "submodule"])
def test_every_admitted_git_source_change_requires_renewal(sandbox, change):
    imported = "../dependency/value.ts" if change == "submodule" else "../value.ts"
    repo = value_repo(sandbox / "repo", imported)
    git(repo, "init", "-q")
    value = repo / "value.ts"
    if change == "submodule":
        dependency = sandbox / "dependency"
        dependency.mkdir()
        value_file(dependency / "value.ts", "before")
        git(dependency, "init", "-q")
        git(dependency, "add", ".")
        git(dependency, "commit", "-qm", "fixture")
        git(repo, "submodule", "add", "-q", str(dependency), "dependency")
        value = repo / "dependency/value.ts"
    elif change == "symlink":
        value_file(repo / "one.ts", "before")
        value_file(repo / "two.ts", "after")
        value.symlink_to("one.ts")
    else:
        value_file(value, "before")
    if change == "ignored":
        (repo / ".gitignore").write_text("value.ts\n")
    git(repo, "add", ".")
    if change == "untracked":
        git(repo, "rm", "--cached", "value.ts")
    git(repo, "commit", "-qm", "reviewed")
    parent = git(repo, "rev-parse", "HEAD")
    source = approve_source(repo)
    checked(repo, "before", source)

    if change == "symlink":
        value.unlink()
        value.symlink_to("two.ts")
    else:
        value_file(value, "after")
    if change == "staged":
        git(repo, "add", "value.ts")
    assert git(repo, "rev-parse", "HEAD") == parent
    if change == "submodule":
        assert git(repo, "rev-parse", "HEAD:dependency") == git(repo / "dependency", "rev-parse", "HEAD")
    denied = rejected(repo)
    assert denied["source"] != source["bundle_digest"]
    renewed = approve_source(repo)
    assert renewed["bundle_digest"] == denied["source"]
    checked(repo, "after", renewed)


def test_branch_names_do_not_authorize_bytes_or_force_identical_renewal(sandbox):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "before")
    git(repo, "init", "-q")
    git(repo, "add", ".")
    git(repo, "commit", "-qm", "reviewed")
    source = approve_source(repo)
    git(repo, "switch", "-qc", "same-bytes")
    checked(repo, "before", source)
    value_file(repo / "value.ts", "after")
    git(repo, "commit", "-qam", "changed bytes")
    rejected(repo)
    checked(repo, "after", approve_source(repo))


def test_alternate_worktree_has_distinct_repository_authority(sandbox):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "before")
    git(repo, "init", "-q")
    git(repo, "add", ".")
    git(repo, "commit", "-qm", "reviewed")
    original = approve_source(repo)
    alternate = sandbox / "alternate"
    git(repo, "worktree", "add", "-qb", "alternate", str(alternate))
    denied = rejected(alternate)
    assert denied["repository"] != original["repository"]
    assert denied["source"] == original["bundle_digest"]
    checked(alternate, "before", approve_source(alternate))


def test_reclone_at_same_path_tracks_bytes_not_git_metadata(sandbox):
    origin = value_repo(sandbox / "origin")
    value_file(origin / "value.ts", "before")
    git(origin, "init", "-q")
    git(origin, "add", ".")
    git(origin, "commit", "-qm", "reviewed")
    repo = sandbox / "clone"
    git(sandbox, "clone", "-q", str(origin), str(repo))
    approved = approve_source(repo)
    shutil.rmtree(repo)
    git(sandbox, "clone", "-q", str(origin), str(repo))
    checked(repo, "before", approved)
    value_file(origin / "value.ts", "after")
    git(origin, "commit", "-qam", "new content")
    shutil.rmtree(repo)
    git(sandbox, "clone", "-q", str(origin), str(repo))
    rejected(repo)
    checked(repo, "after", approve_source(repo))


def pinned_value_repo(sandbox):
    repo = value_repo(sandbox / "repo", "../node_modules/@gripsack/core/canary.ts")
    inspect_source(repo)  # provisions the core-owned SDK, but executes no repo code
    frontend = Path(os.environ["GRIPSACK_HOME"]) / "frontend/current"
    pin = sandbox / "external-sdk"
    shutil.copytree(frontend.resolve(), pin)
    pin.chmod(0o700)
    value_file(pin / "canary.ts", "before")
    (repo / "node_modules/@gripsack").mkdir(parents=True)
    (repo / "node_modules/@gripsack/core").symlink_to(pin, target_is_directory=True)
    return repo, pin / "canary.ts"


def test_pinned_frontend_content_and_added_native_policy_require_renewal(sandbox):
    repo, value = pinned_value_repo(sandbox)
    source = approve_source(repo)
    checked(repo, "before", source)
    value_file(value, "after")
    rejected(repo)
    checked(repo, "after", approve_source(repo))
    # Operator policy is outside source capture; even unchanged source cannot
    # retain approval when the effective native action configuration expands.
    user_config = sandbox / ".config/gripsack/config.toml"
    user_config.parent.mkdir(parents=True)
    before = inspect_source(repo)
    user_config.write_text('[settings]\ndownload_limit_bytes = 1099511627776\n')
    after = inspect_source(repo)
    assert after["bundle_digest"] == before["bundle_digest"]
    assert after["policy_digest"] != before["policy_digest"]
    rejected(repo)
    checked(repo, "after", approve_source(repo))


@pytest.mark.parametrize("generated", [False, True])
def test_outside_imports_are_unavailable_but_generated_in_root_imports_are_reviewable(sandbox, generated):
    outside = sandbox / "outside.ts"
    value_file(outside, "outside")
    imported = "../generated.ts" if generated else outside.as_uri()
    repo = value_repo(sandbox / "repo", imported)
    source = approve_source(repo)
    if generated:
        value_file(repo / "generated.ts", "generated")
        rejected(repo)
        checked(repo, "generated", approve_source(repo))
    else:
        before = receipt_paths()
        result = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
        assert result.returncode == 1
        receipt = new_receipt(before)
        assert receipt["source"] == source["bundle_digest"]
        assert receipt["outcome"] == "failed"
        assert receipt["rounds"][0]["process"]["exit_code"] != 0


def pause_runtime(sandbox, monkeypatch):
    real_deno = shutil.which("deno")
    # The confined evaluator can only reach its private scratch (TMPDIR the
    # core redirects), so the rendezvous lives there rather than the sandbox.
    control = Path(os.environ["GRIPSACK_HOME"]) / "eval-tmp" / "runtime-control"
    control.mkdir(parents=True, exist_ok=True)
    wrapper = sandbox / "paused-deno"
    wrapper.write_text(
        '#!/usr/bin/env python3\n'
        'import hashlib, json, os, pathlib, sys, time\n'
        f'real = {real_deno!r}\ncontrol = pathlib.Path({str(control)!r})\n'
        'if sys.argv[1:] == ["--version"]: os.execv(real, [real, "--version"])\n'
        'counter = control / "counter"\n'
        'round = int(counter.read_text()) + 1 if counter.exists() else 1\n'
        'counter.write_text(str(round))\n'
        'inputs = pathlib.Path(sys.argv[sys.argv.index("--inputs") + 1])\n'
        'evidence = {"input_sha256": hashlib.sha256(inputs.read_bytes()).hexdigest(), "arguments": sys.argv[1:]}\n'
        '(control / f"ready-{round}.json").write_text(json.dumps(evidence))\n'
        'while not (control / f"release-{round}").exists(): time.sleep(0.01)\n'
        'os.execv(real, [real, *sys.argv[1:]])\n'
    )
    wrapper.chmod(0o700)
    monkeypatch.setenv("GRIPSACK_DENO", str(wrapper))
    return control


def wait_ready(control, round, process):
    path = control / f"ready-{round}.json"
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if path.exists():
            try:
                return json.loads(path.read_text())
            except json.JSONDecodeError:
                pass
        if process.poll() is not None:
            stdout, stderr = process.communicate()
            pytest.fail(f"evaluation exited before round {round}: {stdout}\n{stderr}")
        time.sleep(0.01)
    pytest.fail(f"evaluation did not reach round {round}")


@pytest.mark.parametrize("kind", ["repository", "pin", "symlink"])
@pytest.mark.parametrize("rounds", [1, 2])
def test_source_mutation_after_approval_and_between_rounds_cannot_change_evaluation(sandbox, monkeypatch, kind, rounds):
    if kind == "pin":
        repo, value = pinned_value_repo(sandbox)
    else:
        repo = value_repo(sandbox / "repo")
        value = repo / "value.ts"
        if kind == "symlink":
            value_file(repo / "before.ts", "before")
            value_file(repo / "after.ts", "after")
            value.symlink_to("before.ts")
        else:
            value_file(value, "before")
    if rounds == 2:
        host = repo / "hosts/testhost.ts"
        host.write_text(host.read_text().replace("modules: [selected]", 'modules: [ctx.probe.executable("sh") && selected]'))
    control = pause_runtime(sandbox, monkeypatch)
    source = approve_source(repo)
    before = receipt_paths()
    process = subprocess.Popen([str(GRIP), "check", "--host", "testhost", "--json"],
                               cwd=repo, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    observed = []
    try:
        for round in range(1, rounds + 1):
            observed.append(wait_ready(control, round, process))
            if kind == "symlink":
                value.unlink()
                value.symlink_to("after.ts")
            else:
                value_file(value, f"after-{round}")
            (control / f"release-{round}").touch()
        stdout, stderr = process.communicate(timeout=30)
        assert process.returncode == 0, stdout + stderr
        assert json.loads(stdout)["modules"] == ["before"]
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate()
    receipt = new_receipt(before)
    assert receipt["source"] == source["bundle_digest"]
    assert receipt["policy_digest"] == source["policy_digest"]
    assert receipt["outcome"] == "completed"
    assert [r["input_sha256"] for r in receipt["rounds"]] == [r["input_sha256"] for r in observed]
    assert [r["number"] for r in receipt["rounds"]] == list(range(1, rounds + 1))
    if rounds == 2:
        assert observed[0]["input_sha256"] != observed[1]["input_sha256"]
    for round in observed:
        grant, = [arg for arg in round["arguments"] if arg.startswith("--allow-read=")]
        assert str(repo) not in grant
        assert str(Path(os.environ["GRIPSACK_HOME"])) not in grant
    rejected(repo)


def test_legacy_trust_and_ambient_bypass_never_authorize_new_bytes(sandbox, monkeypatch):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "before")
    home = Path(os.environ["GRIPSACK_HOME"])
    home.mkdir(parents=True)
    (home / "trust.toml").write_text(f'[[repos]]\npath = {json.dumps(str(repo))}\ntrusted_at = "2026-09-29T00:00:00Z"\n')
    rejected(repo)
    monkeypatch.setenv("GRIPSACK_TRUST_ALL", "1")
    before = receipt_paths()
    result = grip("check", "--host", "testhost", cwd=repo, approve=False)
    assert result.returncode == 1
    assert receipt_paths() == before
    monkeypatch.delenv("GRIPSACK_TRUST_ALL")
    checked(repo, "before", approve_source(repo))


@pytest.mark.parametrize("fault", ["eio", "permission"])
def test_capture_read_errors_never_become_missing_or_empty_source(sandbox, fault):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "before")
    source = approve_source(repo)
    trace = sandbox / "source-read.tsv"
    environment = dict(os.environ, GRIPSACK_FS_TRACE=str(trace), GRIPSACK_FS_INCLUDE_READS="1")
    command = [str(GRIP), "check", "--host", "testhost", "--json"]
    baseline = subprocess.run(command, cwd=repo, env=environment, capture_output=True, text=True, timeout=60)
    assert baseline.returncode == 0, baseline.stdout + baseline.stderr
    rows = [line.split("\t", 3) for line in trace.read_text().splitlines()]
    point = next(row for row in rows if row[1:] == ["Before", "Read", '"repo/value.ts"'])
    trace.unlink()
    before = receipt_paths()
    environment.update(GRIPSACK_FS_CUT=point[0], GRIPSACK_FS_FAULT=fault)
    failed = subprocess.run(command, cwd=repo, env=environment, capture_output=True, text=True, timeout=60)
    assert failed.returncode == 1, failed.stdout + failed.stderr
    assert receipt_paths() == before, "capture failure must precede executable evaluation"
    emitted = [line.split("\t", 3) for line in trace.read_text().splitlines()]
    assert emitted[-1] == point
    assert (repo / "value.ts").read_text() == 'export default "before";\n'
    checked(repo, "before", source)


def test_stale_noninteractive_approval_cannot_authorize_changed_source_or_runtime(sandbox, monkeypatch):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "before")
    reviewed = inspect_source(repo)
    value_file(repo / "value.ts", "after")
    stale = grip("trust", "add", str(repo), "--bundle", reviewed["bundle_digest"],
                 "--policy", reviewed["policy_digest"], approve=False)
    assert stale.returncode == 1
    rejected(repo)
    current = approve_source(repo)
    real_deno = shutil.which("deno")
    wrapper = sandbox / "selected-runtime"
    wrapper.write_text(f'#!/bin/sh\nexec "{real_deno}" "$@"\n')
    wrapper.chmod(0o700)
    monkeypatch.setenv("GRIPSACK_DENO", str(wrapper))
    changed = inspect_source(repo)
    assert changed["bundle_digest"] == current["bundle_digest"]
    assert changed["policy_digest"] != current["policy_digest"]
    stale = grip("trust", "add", str(repo), "--bundle", current["bundle_digest"],
                 "--policy", current["policy_digest"], approve=False)
    assert stale.returncode == 1
    rejected(repo)
    checked(repo, "after", approve_source(repo))


@pytest.mark.parametrize("alias_kind", ["relative", "absolute", "directory"])
def test_native_overlay_materializes_captured_alias_targets(sandbox, alias_kind):
    source = 'tree("alias", "~/.aliased", "owned")' if alias_kind == "directory" else '{alias: symlink("~/.aliased/file")}'
    repo = make_env_repo(sandbox / "repo", {
        "aliased": 'import {module,symlink,tree} from "@gripsack/core";\n'
                   f'export default module("aliased", {{config: {source}}});\n',
    })
    original = repo / "original/file"
    original.parent.mkdir()
    original.write_text("captured\n")
    original.chmod(0o750)
    alias = repo / "alias"
    if alias_kind == "directory":
        alias.symlink_to("original", target_is_directory=True)
    else:
        alias.symlink_to(original if alias_kind == "absolute" else "original/file")
    approved = approve_source(repo)
    first = grip("apply", "--host", "testhost", cwd=repo, approve=False)
    assert first.returncode == 0, first.stdout + first.stderr
    installed = sandbox / ".aliased/file"
    assert installed.is_symlink()
    assert installed.read_text() == "captured\n"
    generation = (Path(os.environ["GRIPSACK_HOME"]) / "current").readlink()
    # Published lock bytes can renew the read set; this is explicit fixture
    # approval, not a blanket exemption for core-written repository content.
    approve_source(repo)
    preview = grip("plan", "--host", "testhost", cwd=repo, approve=False)
    assert preview.returncode == 0, preview.stderr
    assert "(satisfied)" in preview.stdout, preview.stdout
    second = grip("apply", "--host", "testhost", cwd=repo, approve=False)
    assert second.returncode == 0, second.stderr
    assert (Path(os.environ["GRIPSACK_HOME"]) / "current").readlink() == generation
    original.write_text("later live bytes\n")
    assert installed.read_text() == "captured\n"
    denied = rejected(repo)
    assert denied["source"] != approved["bundle_digest"]


def test_takeover_cannot_replace_a_directory_entry_inside_approved_source(sandbox):
    repo = make_env_repo(sandbox / "repo", {})
    (repo / "original").write_text("source bytes\n")
    (repo / "destination").symlink_to("original")
    (repo / "payload").write_text("replacement\n")
    (repo / "hosts/testhost.ts").write_text(
        'import {defineEnv,module,symlink} from "@gripsack/core";\n'
        f'export default defineEnv(() => ({{modules:[module("guard",{{config:{{payload:symlink({json.dumps(str(repo / "destination"))})}}}})]}}));\n'
    )
    approve_source(repo)
    result = grip("apply", "--host", "testhost", "--take-over", cwd=repo, approve=False)
    assert result.returncode == 1
    assert (repo / "destination").readlink() == Path("original")
    assert (repo / "original").read_text() == "source bytes\n"
    assert not (Path(os.environ["GRIPSACK_HOME"]) / "current").exists()


def test_hoisted_node_modules_never_become_ambient_evaluation_sources(sandbox, monkeypatch):
    monkeypatch.setenv("TMPDIR", str(sandbox))
    package = sandbox / "node_modules/ambient-fixture"
    package.mkdir(parents=True)
    (package / "package.json").write_text('{"name":"ambient-fixture","version":"1.0.0","type":"module","main":"index.js"}')
    (package / "index.js").write_text('export default "admitted-package";\n')
    repo = value_repo(sandbox / "repo", "ambient-fixture")
    (repo / "package.json").write_text('{"type":"module","dependencies":{"ambient-fixture":"1.0.0"}}')
    approve_source(repo)
    before = receipt_paths()
    denied = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
    assert denied.returncode == 1
    assert new_receipt(before)["outcome"] == "failed"
    shutil.copytree(package, repo / "node_modules/ambient-fixture")
    checked(repo, "admitted-package", approve_source(repo))


def test_round_file_grant_does_not_grant_its_parent_directory(sandbox):
    prelude = (
        'import {module} from "@gripsack/core";\n'
        'const input = Deno.args[Deno.args.indexOf("--inputs") + 1];\n'
        'const facts = JSON.parse(Deno.readTextFileSync(input));\n'
        'if (facts.version !== 1) throw new Error("invalid fixture input");\n'
    )
    repo = make_env_repo(sandbox / "repo", {
        "selected": prelude + 'export default module("input-file", {install: []});\n',
    })
    checked(repo, "input-file", approve_source(repo))
    (repo / "modules/selected.ts").write_text(
        prelude + 'Array.from(Deno.readDirSync(input.slice(0, input.lastIndexOf("/"))));\n'
        'export default module("parent-directory-leak", {install: []});\n'
    )
    approve_source(repo)
    before = receipt_paths()
    denied = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
    assert denied.returncode == 1
    receipt = new_receipt(before)
    assert receipt["outcome"] == "failed"
    assert receipt["rounds"][0]["process"]["exit_code"] != 0
