"""File subjects keep their explicit preparation/transaction/activation stages."""
import json

import pytest
from conftest import grip


IMPORTS = '''import { defineWorkspace, workspace, profile, file, repoFile,
  templateFile, literalText, identity, trackedCopyTo, symlinkTo, managedBlock, check,
  exec, lit, sourcePath, repoTree } from "@gripsack/core";
'''


def declare(repo, argv, *, subject="rendered", stage="pre_flip", value="new",
            source=True, checks=True, policy="copy", tree=False):
    repo.mkdir(exist_ok=True)
    destination = {"copy": 'trackedCopyTo("~/.file-checked")',
                   "link": 'symlinkTo("~/.file-checked")',
                   "block": 'managedBlock("~/.file-checked", "checked")'}[policy]
    if tree:
        body = 'source: repoTree("tree"), content: identity(), destination: trackedCopyTo("~/.checked-tree")'
    elif source:
        body = 'source: repoFile("template"), content: templateFile({value: ' + json.dumps(value) + '}), destination: ' + destination
    else:
        body = 'content: literalText(' + json.dumps(value) + '), destination: ' + destination
    if checks:
        body += ', checks: [{check:"validate",subject:' + json.dumps(subject) + ',stage:' + json.dumps(stage) + '}]'
    arguments = ','.join('sourcePath()' if arg is None else 'lit(' + json.dumps(arg) + ')' for arg in argv)
    (repo / "gripsack.ts").write_text(IMPORTS + '''
export default defineWorkspace(() => workspace({outputs: [
  check("validate", {subject:"personal", run:exec({argv:[''' + arguments + ''']})}),
  profile("personal", {files:[file({''' + body + '''})]})
]}));
''')


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def selected(sandbox):
    path = sandbox / ".local/share/gripsack/current"
    return path.readlink() if path.exists() else None


def hooks(repo):
    return json.loads(run(repo, "hooks", "list", "--json").stdout)["intents"]


def assert_retained_source_coordinates(sandbox, repo):
    snapshots = list((sandbox / ".local/share/gripsack/store").glob("*/file-check-*.json"))
    assert snapshots, "file-check identity must retain its complete snapshot"
    for snapshot in snapshots:
        inputs = json.loads(snapshot.read_text())["file_check"]
        for field in ["span", "file_span", "command_span"]:
            assert inputs[field]["file"] in {
                "gripsack.ts",
                str(repo / "gripsack.ts"), (repo / "gripsack.ts").as_uri(),
            }, inputs[field]
            assert inputs[field]["line"] > 0


@pytest.mark.parametrize("policy", ["copy", "link", "block"])
def test_invalid_rendered_candidate_never_mutates_destination(sandbox, policy):
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/true"], source=False, checks=False, value="old", policy=policy)
    run(repo, "apply")
    previous = selected(sandbox)
    destination = sandbox / ".file-checked"
    prior_bytes = destination.read_bytes()
    (repo / "template").write_text("value={{ value }}\n")
    declare(repo, ["/usr/bin/grep", "-Fx", "value=valid", None], value="invalid", policy=policy)
    result = grip("apply", cwd=repo)
    assert result.returncode != 0
    assert selected(sandbox) == previous
    assert destination.read_bytes() == prior_bytes


def test_source_and_rendered_checks_observe_different_captured_bytes(sandbox):
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/grep", "-Fx", "value={{ value }}", None], subject="source")
    (repo / "template").write_text("value={{ value }}\n")
    run(repo, "apply")
    assert (sandbox / ".file-checked").read_text() == "value=new\n"
    source_generation = selected(sandbox)
    assert_retained_source_coordinates(sandbox, repo)
    run(repo, "apply")
    assert selected(sandbox) == source_generation
    declare(repo, ["/usr/bin/grep", "-Fx", "value=new", None])
    run(repo, "apply")
    original = selected(sandbox)
    run(repo, "apply")
    assert selected(sandbox) == original
    assert_retained_source_coordinates(sandbox, repo)
    # Changing the command with identical file bytes changes consumer identity.
    declare(repo, ["/usr/bin/grep", "-F", "new", None])
    run(repo, "apply")
    assert selected(sandbox) != original


@pytest.mark.parametrize("alias_kind", ["home", "tmp"])
@pytest.mark.parametrize("subject", ["source", "rendered"])
def test_file_checks_accept_stable_store_and_temporary_aliases(sandbox, monkeypatch, alias_kind, subject):
    real = sandbox / "physical-root"
    real.mkdir()
    alias = sandbox / "declared-root"
    alias.symlink_to(real, target_is_directory=True)
    if alias_kind == "home":
        monkeypatch.setenv("GRIPSACK_HOME", str(alias / "gripsack"))
    else:
        monkeypatch.setenv("TMPDIR", str(alias))
    repo = sandbox / "checks"
    expected = "value={{ value }}" if subject == "source" else "value=new"
    declare(repo, ["/usr/bin/grep", "-Fx", expected, None], subject=subject)
    (repo / "template").write_text("value={{ value }}\n")
    run(repo, "apply")
    assert (sandbox / ".file-checked").read_text() == "value=new\n"
    run(repo, "apply")
    if subject == "source":
        # Replay retained source evidence through the same declared store alias.
        declare(repo, ["/usr/bin/grep", "-Fx", expected, None],
                subject="source", stage="post_activate")
        run(repo, "apply")
        store_home = alias / "gripsack" if alias_kind == "home" else sandbox / ".local/share/gripsack"
        original = (store_home / "current").readlink()
        declare(repo, ["/usr/bin/true"], source=False, checks=False, value="replacement")
        run(repo, "apply")
        (repo / "template").unlink()
        (repo / "gripsack.ts").unlink()
        run(repo, "rollback", original.name)
        assert (sandbox / ".file-checked").read_text() == "value=new\n"


def test_file_check_store_alias_retarget_refuses_candidate(sandbox, monkeypatch):
    import sys

    real = sandbox / "physical-store"
    real.mkdir()
    alias = sandbox / ".local/share/gripsack"
    alias.parent.mkdir(parents=True)
    alias.symlink_to(real, target_is_directory=True)
    replacement = sandbox / "replacement-store"
    monkeypatch.setenv("GRIPSACK_HOME", str(alias))
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/true"], source=False, checks=False, value="old")
    run(repo, "apply")
    previous = selected(sandbox)
    # The real validator retargets only after observation and command binding.
    # Copy identical leaves: refusal must be about identity, not changed bytes.
    declare(repo, [sys.executable, "-c",
                  "import pathlib,shutil,sys; "
                  "assert pathlib.Path(sys.argv[1]).read_text() == 'new'; "
                  "alias=pathlib.Path(sys.argv[2]); replacement=pathlib.Path(sys.argv[3]); "
                  "shutil.copytree(alias.resolve(),replacement,symlinks=True); "
                  "alias.unlink(); alias.symlink_to(replacement,target_is_directory=True)",
                  None, str(alias), str(replacement)], source=False, value="new")
    result = grip("apply", cwd=repo)
    assert alias.resolve() == replacement
    assert result.returncode != 0
    assert "subject changed" in result.stderr
    assert selected(sandbox) == previous
    assert (sandbox / ".file-checked").read_text() == "old"


def test_missing_check_runtime_is_preparation_failure(sandbox):
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/true"], source=False, checks=False, value="old")
    run(repo, "apply")
    previous = selected(sandbox)
    declare(repo, ["/not/a/check/runtime"], source=False)
    result = grip("apply", cwd=repo)
    assert result.returncode != 0
    assert selected(sandbox) == previous
    assert (sandbox / ".file-checked").read_text() == "old"


@pytest.mark.parametrize("stage", ["pre_flip", "post_link", "post_activate"])
def test_readonly_never_executes_file_check(sandbox, stage):
    repo = sandbox / "checks"
    effect = sandbox / "effect"
    declare(repo, ["/usr/bin/touch", str(effect)], source=False, subject="deployed", stage=stage)
    run(repo, "check")
    run(repo, "plan")
    assert not effect.exists()
    assert not (sandbox / ".file-checked").exists()
    assert selected(sandbox) is None


def test_deployed_preflip_checks_new_destination_and_failure_compensates(sandbox):
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/true"], source=False, checks=False, value="old")
    run(repo, "apply")
    previous = selected(sandbox)
    declare(repo, ["/usr/bin/grep", "-Fx", "old", None], source=False, subject="deployed", value="new")
    result = grip("apply", cwd=repo)
    assert result.returncode != 0  # Checking the OLD path would incorrectly pass.
    assert (sandbox / ".file-checked").read_text() == "old"
    assert selected(sandbox) == previous
    declare(repo, ["/usr/bin/grep", "-Fx", "new", None], source=False, subject="deployed", value="new")
    run(repo, "apply")
    assert (sandbox / ".file-checked").read_text() == "new"
    run(repo, "rollback", previous.name)
    assert (sandbox / ".file-checked").read_text() == "old"


@pytest.mark.parametrize("stage", ["post_link", "post_activate"])
def test_late_failure_retains_new_generation_until_explicit_rollback(sandbox, stage):
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/true"], source=False, checks=False, value="old")
    run(repo, "apply")
    previous = selected(sandbox)
    declare(repo, ["/usr/bin/false"], source=False, subject="deployed", stage=stage)
    run(repo, "apply")  # Existing activation ledger reports degraded effects.
    current = selected(sandbox)
    assert current != previous
    assert (sandbox / ".file-checked").read_text() == "new"
    assert hooks(repo)[-1]["state"]["kind"] == "failed"
    run(repo, "apply")
    assert selected(sandbox) == current
    assert hooks(repo)[-1]["state"] == {"kind": "failed", "attempt": 1}
    assert_retained_source_coordinates(sandbox, repo)
    run(repo, "rollback", previous.name)
    assert (sandbox / ".file-checked").read_text() == "old"


def test_retained_source_check_replays_after_source_and_declaration_removal(sandbox):
    repo = sandbox / "checks"
    effect = sandbox / "observed-source"
    declare(repo, ["/usr/bin/cp", None, str(effect)], subject="source", stage="post_activate")
    (repo / "template").write_text("value={{ value }}\n")
    run(repo, "apply")
    original = selected(sandbox)
    assert effect.read_text() == "value={{ value }}\n"
    (repo / "template").unlink()
    declare(repo, ["/usr/bin/true"], source=False, checks=False, value="replacement")
    run(repo, "apply")
    effect.unlink()
    (repo / "gripsack.ts").unlink()
    run(repo, "rollback", original.name)
    assert effect.read_text() == "value={{ value }}\n"
    assert (sandbox / ".file-checked").read_text() == "value=new\n"


def test_source_mode_and_per_leaf_tree_checks(sandbox):
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/test", "-x", None], subject="source", tree=True)
    tree = repo / "tree"
    tree.mkdir()
    for name in ["first", "second"]:
        path = tree / name
        path.write_text("executable\n")
        path.chmod(0o755)
    run(repo, "apply")
    previous = selected(sandbox)
    (tree / "second").chmod(0o644)
    result = grip("apply", cwd=repo)
    assert result.returncode != 0
    assert selected(sandbox) == previous
    assert (sandbox / ".checked-tree/second").stat().st_mode & 0o111


@pytest.mark.parametrize("subject,stage", [("rendered", "post_link"), ("rendered", "post_activate"), ("source", "pre_flip")])
def test_nonsensical_file_subject_stage_is_rejected(sandbox, subject, stage):
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/true"], source=False, subject=subject, stage=stage)
    result = grip("plan", cwd=repo)
    assert result.returncode != 0
    assert selected(sandbox) is None
    assert not (sandbox / ".file-checked").exists()


def test_cancelled_preflight_never_deploys_candidate(sandbox):
    import os
    import signal
    import subprocess
    import sys
    import time
    from conftest import GRIP, start_grip

    repo = sandbox / "checks"
    started = sandbox / "started"
    declare(repo, [sys.executable, "-c",
                  "import os,pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(60)",
                  str(started)], source=False)
    process = start_grip([str(GRIP.resolve()), "apply"], cwd=repo,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        deadline = time.monotonic() + 30
        while not started.exists() and process.poll() is None and time.monotonic() < deadline:
            time.sleep(0.02)
        if not started.exists():
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
            stdout, stderr = process.communicate(timeout=20)
            pytest.fail("the actual check must launch before cancellation\n" + stdout + stderr)
        process.send_signal(signal.SIGINT)
        stdout, stderr = process.communicate(timeout=20)
        assert process.returncode != 0, stdout + stderr
        child = int(started.read_text())
        with pytest.raises(ProcessLookupError):
            os.kill(child, 0)
        assert not list((sandbox / ".local/share/gripsack/workspace-staging").iterdir())
        assert selected(sandbox) is None
        assert not (sandbox / ".file-checked").exists()
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate(timeout=10)
        if started.exists() and started.read_text():
            try:
                os.kill(int(started.read_text()), signal.SIGKILL)
            except ProcessLookupError:
                pass


def test_retained_file_check_restores_package_runtime_after_source_removal(sandbox):
    from conftest import make_toolchain_tarball

    effect = sandbox / "package-observation"
    archive = make_toolchain_tarball(
        sandbox / "validator.tar.gz",
        {"bin/validate": b'#!/bin/sh\nIFS= read -r line < "$1"\nprintf "%s\\n" "$line" > "$2"\n'},
    )
    repo = sandbox / "checks"
    repo.mkdir()
    (repo / "template").write_text("original source\n")
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, profile, file, repoFile, identity, trackedCopyTo,
      check, exec, packageCommand, sourcePath, lit, pkg, provider, tarball,
    } from "@gripsack/core";
    export default defineWorkspace(ctx => workspace({outputs: [
      pkg("tools", {producer: provider(tarball(''' + json.dumps(archive.as_uri()) + ''')),
        commands: {validate:"bin/validate"}, target:{os:ctx.facts.os,arch:ctx.facts.arch},
        layout:{kind:"relocatable"}}),
      check("validate", {subject:"personal", run:exec({argv:[
        packageCommand("tools","validate"), sourcePath(), lit(''' + json.dumps(str(effect)) + ''')
      ]})}),
      profile("personal", {files:[file({source:repoFile("template"), content:identity(),
        destination:trackedCopyTo("~/.file-checked"),
        checks:[{check:"validate",subject:"source",stage:"post_activate"}]})]})
    ]}));\n''')
    run(repo, "update", "tools")
    run(repo, "apply")
    original = selected(sandbox)
    assert effect.read_text() == "original source\n"
    archive.unlink()
    (repo / "template").unlink()
    declare(repo, ["/usr/bin/true"], source=False, checks=False, value="replacement")
    run(repo, "apply")
    effect.unlink()
    run(repo, "rollback", original.name)
    assert effect.read_text() == "original source\n"
    assert all(row["state"]["kind"] == "succeeded" for row in hooks(repo))


def test_missing_explicit_interpreter_blocks_before_old_file_release(sandbox):
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/true"], source=False, checks=False, value="old")
    run(repo, "apply")
    previous = selected(sandbox)
    declare(repo, [None], subject="source")
    source = repo / "template"
    source.write_text("#!/no/such/declared-interpreter\nexit 0\n")
    source.chmod(0o755)
    result = grip("apply", cwd=repo)
    assert result.returncode != 0
    assert selected(sandbox) == previous
    assert (sandbox / ".file-checked").read_text() == "old"


def test_check_cannot_replace_the_bytes_it_was_required_to_validate(sandbox):
    repo = sandbox / "checks"
    declare(repo, ["/usr/bin/truncate", "-s", "0", None], source=False, value="approved")
    result = grip("apply", cwd=repo)
    assert result.returncode != 0
    assert "subject changed" in result.stderr
    assert selected(sandbox) is None
    assert not (sandbox / ".file-checked").exists()
