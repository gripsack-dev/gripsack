"""Live-source grants and file transports cannot bypass capture selection."""
import json
import os
from pathlib import Path
import shutil
import subprocess

import pytest
from conftest import GRIP, grip
from test_capture_policy import exclude
from test_source_capture import (
    checked, new_receipt, pause_runtime, receipt_paths,
    value_file, value_repo, wait_ready,
)


def activated_path(sandbox, repo, spelling, monkeypatch):
    venv = repo / ".venv/bin"
    venv.mkdir(parents=True, exist_ok=True)
    if spelling == "alias":
        directory = sandbox / "venv-bin-alias"
        directory.symlink_to(venv, target_is_directory=True)
    elif spelling == "ancestor":
        directory = sandbox
    else:
        directory = venv
    # Select the real native evaluator, not a wrapper whose PATH is executable
    # runtime authority. Native Deno has no reason to grant this extra directory.
    monkeypatch.setenv("GRIPSACK_DENO", shutil.which("deno"))
    monkeypatch.setenv("PATH", str(directory) + os.pathsep + os.environ["PATH"])
    return venv


@pytest.mark.parametrize("spelling", ["direct", "alias", "ancestor"])
def test_activated_venv_path_cannot_restore_excluded_static_import(sandbox, monkeypatch, spelling):
    repo = sandbox / "repo"
    live = repo / ".venv/bin/live.js"
    value_repo(repo, live.as_uri())
    activated_path(sandbox, repo, spelling, monkeypatch)
    live.write_text('export default "EXCLUDED_FIRST";\n')
    exclude(repo, ".venv")
    for value in ["EXCLUDED_FIRST", "EXCLUDED_CHANGED"]:
        live.write_text(f'export default "{value}";\n')
        before = receipt_paths()
        denied = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
        assert denied.returncode != 0, denied.stdout + denied.stderr
        assert value not in denied.stdout, denied.stdout
        receipt = new_receipt(before)
        assert receipt["outcome"] == "failed"
        assert receipt["rounds"], "the real evaluator must exercise the denied static import"


@pytest.mark.parametrize("spelling", ["direct", "alias", "ancestor"])
def test_unused_activated_path_does_not_break_native_captured_evaluation(sandbox, monkeypatch, spelling):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "captured")
    activated_path(sandbox, repo, spelling, monkeypatch)
    exclude(repo, ".venv")
    checked(repo, "captured")


@pytest.mark.parametrize("spelling", ["direct", "alias", "ancestor"])
def test_script_runtime_cannot_grant_live_source_via_path(sandbox, monkeypatch, spelling):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "captured")
    activated_path(sandbox, repo, spelling, monkeypatch)
    exclude(repo, ".venv")
    runtime = sandbox / "runtime-wrapper"
    runtime.write_text('#!/bin/sh\nexec "' + shutil.which("deno") + '" "$@"\n')
    runtime.chmod(0o700)
    monkeypatch.setenv("GRIPSACK_DENO", str(runtime))
    checked(repo, "captured")
    live = repo / ".venv/bin/live.js"
    value_file(live, "EXCLUDED_PATH_MODULE")
    (repo / "outputs/selected.ts").write_text(
        'import {profile} from "@gripsack/core";\n'
        f'import value from {json.dumps(live.as_uri())};\n'
        'export default profile(value, {files:[]});\n'
    )
    before = receipt_paths()
    refused = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
    assert refused.returncode != 0, refused.stdout + refused.stderr
    assert new_receipt(before)["outcome"] == "failed"
    assert "EXCLUDED_PATH_MODULE" not in refused.stdout


@pytest.mark.parametrize("name", ["deno-cache", "eval-tmp"])
def test_evaluator_cache_or_scratch_alias_cannot_grant_excluded_source(sandbox, name):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "captured")
    (repo / ".venv/cache").mkdir(parents=True)
    exclude(repo, ".venv")
    home = Path(os.environ["GRIPSACK_HOME"])
    home.mkdir(parents=True, exist_ok=True)
    (home / name).symlink_to(repo / ".venv/cache", target_is_directory=True)
    refused = grip("check", "--repo", str(repo), "--json", approve=False)
    assert refused.returncode != 0
    assert "overlaps live source" in refused.stderr, refused.stdout + refused.stderr


def file_workspace(sandbox, *, excluded):
    repo = sandbox / "real-repository"
    repo.mkdir()
    alias = sandbox / "declared-repository"
    alias.symlink_to(repo, target_is_directory=True)
    payload = repo / "payload"
    payload.mkdir()
    (payload / "value").write_text("captured before mutation\n")
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, pkg, provider, fileFetch, profile, file,
      artifactFile, identity, trackedCopyTo,
    } from "@gripsack/core";
    export default defineWorkspace(() => workspace({outputs:[
      pkg("data", {producer:provider(fileFetch(''' + json.dumps(str(alias / "payload")) + ''')),
        commands:{}, target:{os:"linux",arch:"x86_64"}, layout:{kind:"relocatable"}}),
      profile("files", {files:[file({source:artifactFile("data", "value"), content:identity(),
        destination:trackedCopyTo("~/.captured-file-source")})]}),
    ]}));
    ''')
    if excluded:
        exclude(repo, "payload")
    return repo, alias


@pytest.mark.parametrize("arguments", [("update",), ("plan",), ("build", "data", "--json")])
@pytest.mark.parametrize("parent_components", [False, True])
def test_declared_repository_alias_file_fetch_cannot_read_excluded_source(sandbox, arguments, parent_components):
    repo, alias = file_workspace(sandbox, excluded=True)
    if parent_components:
        anchor = sandbox / "anchor"
        anchor.mkdir()
        alias = anchor / ".." / alias.name
    refused = grip(*arguments, "--repo", str(alias), cwd=sandbox, approve=False)
    assert refused.returncode != 0, refused.stdout + refused.stderr
    assert "excluded by capture policy" in refused.stdout + refused.stderr
    assert not (repo / "gripsack.lock").exists()
    assert not (sandbox / ".captured-file-source").exists()
    assert not (Path(os.environ["GRIPSACK_HOME"]) / "current").exists()


def test_declared_repository_alias_file_fetch_uses_captured_not_later_live_bytes(sandbox, monkeypatch):
    repo, alias = file_workspace(sandbox, excluded=False)
    control = pause_runtime(sandbox, monkeypatch)
    before = receipt_paths()
    process = subprocess.Popen(
        [str(GRIP), "build", "data", "--repo", str(alias), "--json"], cwd=sandbox,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    try:
        wait_ready(control, 1, process)
        (repo / "payload/value").write_text("later live bytes must not be acquired\n")
        (control / "release-1").touch()
        stdout, stderr = process.communicate(timeout=30)
        assert process.returncode == 0, stdout + stderr
        outputs = json.loads(stdout)["outputs"]
        assert len(outputs) == 1
        assert (Path(outputs[0]["path"]) / "value").read_text() == "captured before mutation\n"
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate()
    assert new_receipt(before)["outcome"] == "completed"


@pytest.mark.parametrize("runtime_kind", ["native", "script", "source_spelled", "source_path_name"])
@pytest.mark.parametrize("alias_kind", ["leaf", "descendant"])
def test_excluded_outbound_alias_cannot_import_mutable_runtime_neighbor(sandbox, monkeypatch, runtime_kind, alias_kind):
    repo = sandbox / "repo"
    external = sandbox / "external-runtime"
    binary_directory = external / "bin"
    binary_directory.mkdir(parents=True)
    real_deno = shutil.which("deno")
    selected_deno = binary_directory / "deno"
    shutil.copy2(real_deno, selected_deno)
    selected_deno.chmod(0o700)
    canary = binary_directory / "live.js"
    canary.write_text('export default "OUTBOUND_RUNTIME_FIRST";\n')
    live_root = repo / ".venv" if alias_kind == "leaf" else repo / ".venv/runtime"
    live_import = live_root / "bin/live.js"
    value_repo(repo, live_import.as_uri())
    live_root.parent.mkdir(parents=True, exist_ok=True)
    live_root.symlink_to(external, target_is_directory=True)
    exclude(repo, ".venv")
    if runtime_kind == "native":
        monkeypatch.setenv("GRIPSACK_DENO", str(selected_deno))
    elif runtime_kind == "source_spelled":
        monkeypatch.setenv("GRIPSACK_DENO", str(live_root / "bin/deno"))
    elif runtime_kind == "source_path_name":
        monkeypatch.setenv("GRIPSACK_DENO", "deno")
        monkeypatch.setenv("PATH", str(live_root / "bin") + os.pathsep + os.environ["PATH"])
    else:
        wrapper = sandbox / "outbound-runtime-wrapper"
        wrapper.write_text(f'#!/bin/sh\nexec "{selected_deno}" "$@"\n')
        wrapper.chmod(0o700)
        monkeypatch.setenv("GRIPSACK_DENO", str(wrapper))
        monkeypatch.setenv("PATH", str(binary_directory) + os.pathsep + os.environ["PATH"])
    # Calibrate the payload with the real native Deno: it must load through the
    # live excluded spelling without confinement, not fail for syntax/fixture setup.
    control = sandbox / "control.ts"
    control.write_text(f'import value from {json.dumps(live_import.as_uri())};\nconsole.log(value);\n')
    unconfined = subprocess.run(
        [str(selected_deno), "run", "--no-config", "--no-remote", str(control)],
        cwd=sandbox, capture_output=True, text=True, timeout=30,
    )
    assert unconfined.returncode == 0, unconfined.stdout + unconfined.stderr
    assert unconfined.stdout.strip() == "OUTBOUND_RUNTIME_FIRST"
    if runtime_kind in {"source_spelled", "source_path_name"}:
        before = receipt_paths()
        refused = grip("check", "--repo", str(repo), "--json", approve=False)
        assert refused.returncode != 0
        assert "overlaps live source" in refused.stderr, refused.stdout + refused.stderr
        assert receipt_paths() == before
        return
    for value in ["OUTBOUND_RUNTIME_FIRST", "OUTBOUND_RUNTIME_CHANGED"]:
        canary.write_text(f'export default "{value}";\n')
        before = receipt_paths()
        refused = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
        assert refused.returncode != 0, refused.stdout + refused.stderr
        receipt = new_receipt(before)
        assert receipt["outcome"] == "failed"
        assert receipt["rounds"], "runtime startup must reach the actual denied import"
        assert value not in refused.stdout


def test_runtime_dependency_discovery_never_executes_a_path_shadow_ldd(sandbox, monkeypatch):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "captured")
    selected = shutil.which("deno")
    helpers = sandbox / "helper-shadow"
    helpers.mkdir()
    marker = sandbox / "ambient-ldd-ran"
    helper = helpers / "ldd"
    helper.write_text(f'#!/bin/sh\nprintf invoked > "{marker}"\nexit 99\n')
    helper.chmod(0o700)
    monkeypatch.setenv("GRIPSACK_DENO", selected)
    monkeypatch.setenv("PATH", str(helpers) + os.pathsep + os.environ["PATH"])
    checked(repo, "captured")
    assert not marker.exists(), "operator PATH selected a repository-unbound ldd helper"


@pytest.mark.skipif(os.uname().sysname != "Linux", reason="ELF trailing-byte identity mutation")
def test_path_selected_script_interpreter_byte_changes_move_the_policy_identity(sandbox, monkeypatch):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "captured")
    engine = shutil.which("deno")
    directory = sandbox / "operator-interpreter"
    directory.mkdir()
    interpreter = directory / "sh"
    shutil.copy2("/bin/sh", interpreter)
    interpreter.chmod(0o700)
    wrapper = sandbox / "selected-wrapper"
    wrapper.write_text(f'#!/usr/bin/env sh\nexec "{engine}" "$@"\n')
    wrapper.chmod(0o700)
    monkeypatch.setenv("PATH", str(directory) + os.pathsep + os.environ["PATH"])
    monkeypatch.setenv("GRIPSACK_DENO", str(wrapper))
    reviewed = checked(repo, "captured")
    with interpreter.open("ab") as output:
        output.write(b"\nchanged executable identity, outside ELF load segments\n")
    current = checked(repo, "captured")
    assert current["source"] == reviewed["source"]
    assert current["policy"]["runtime"] == reviewed["policy"]["runtime"], "env + wrapper remain unchanged"
    # No renewal is needed, but the receipt still records that the runtime
    # closure the evaluator ran under is different.
    assert current["policy_digest"] != reviewed["policy_digest"]


@pytest.mark.parametrize("reject_after_discovery", [False, True])
def test_python_runtime_metadata_never_imports_repository_sysconfig(sandbox, monkeypatch, reject_after_discovery):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "captured")
    marker = sandbox / "unapproved-python-import"
    (repo / "sysconfig.py").write_text(
        f'open({json.dumps(str(marker))}, "w").write("executed")\n'
        'raise RuntimeError("unapproved repository sysconfig")\n'
    )
    if reject_after_discovery:
        forbidden = repo / "forbidden-cache"
        forbidden.mkdir()
        (Path(os.environ["GRIPSACK_HOME"]) / "deno-cache").symlink_to(forbidden, target_is_directory=True)
    run = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
    if reject_after_discovery:
        assert run.returncode != 0
        assert "overlaps live source" in run.stderr, run.stdout + run.stderr
    else:
        assert run.returncode == 0, run.stdout + run.stderr
    assert not marker.exists()
    assert not (Path(os.environ["GRIPSACK_HOME"]) / "current").exists()


def test_isolated_runtime_discovery_preserves_a_real_external_python_venv(sandbox, monkeypatch):
    python = shutil.which("python3")
    engine = shutil.which("deno")
    venv = sandbox / "external-python-venv"
    created = subprocess.run(
        [python, "-I", "-m", "venv", "--without-pip", str(venv)],
        cwd=sandbox, capture_output=True, text=True, timeout=60,
    )
    assert created.returncode == 0, created.stdout + created.stderr
    interpreter = venv / "bin/python3"
    control = subprocess.run(
        [str(interpreter), "-I", "-c",
         "import json,sys,sysconfig; print(json.dumps({'prefix':sys.prefix, **sysconfig.get_paths()}))"],
        cwd=sandbox, capture_output=True, text=True, timeout=30,
    )
    assert control.returncode == 0, control.stdout + control.stderr
    installation = json.loads(control.stdout)
    assert Path(installation["prefix"]).resolve() == venv.resolve()
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "real-venv")
    wrapper = sandbox / "python-venv-wrapper"
    wrapper.write_text(
        '#!/usr/bin/env python3\nimport os,sys\n'
        f'os.execv({engine!r}, [{engine!r}, *sys.argv[1:]])\n'
    )
    wrapper.chmod(0o700)
    monkeypatch.setenv("PATH", str(venv / "bin") + os.pathsep + os.environ["PATH"])
    monkeypatch.setenv("GRIPSACK_DENO", str(wrapper))
    # Discovery reaches the venv's own runtime roots; evaluating under
    # confinement is the observable contract (grants are not introspectable).
    checked(repo, "real-venv")
