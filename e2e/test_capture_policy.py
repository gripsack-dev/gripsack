"""Explicit source exclusions use the real capture, approval and Deno paths."""
import json
import os
from pathlib import Path
import shutil
import subprocess

import pytest
from conftest import GRIP, grip, make_env_repo
from test_source_approval import (
    approve_source, checked, inspect_source, new_receipt, pause_runtime, pinned_value_repo,
    receipt_paths, rejected, value_file, value_repo, wait_ready,
)


def exclude(repo, *paths):
    # TOML strings and arrays have the same syntax as these JSON string values.
    (repo / "env.toml").write_text("[capture]\nexclude = " + json.dumps(paths) + "\n")


def test_ignored_outbound_venv_is_explicitly_excluded_and_names_failure(sandbox):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "legitimate")
    (repo / ".gitignore").write_text(".venv/\nvalue.ts\n")
    (repo / ".venv/bin").mkdir(parents=True)
    (repo / ".venv/bin/python3").symlink_to("/usr/bin/python3")
    before = receipt_paths()
    failed = grip("trust", "inspect", str(repo), "--json", approve=False)
    assert failed.returncode != 0
    assert "repo/.venv/bin/python3" in failed.stderr, failed.stderr
    assert "escapes its admitted roots" in failed.stderr, failed.stderr
    assert receipt_paths() == before

    exclude(repo, ".venv")
    reviewed = approve_source(repo)
    assert reviewed["version"] == 2
    assert reviewed["capture_exclusions"] == [".venv"]
    assert "repo/.venv" in reviewed["inventory"]["exclusions"]
    assert any(entry["path"] == "repo/value.ts" for entry in reviewed["inventory"]["entries"])
    checked(repo, "legitimate", reviewed)
    # Unavailable content is not part of the admitted read set; changing it
    # alone cannot force approval or alter the evaluator's result.
    value_file(repo / ".venv/private.ts", "not admitted")
    checked(repo, "legitimate", reviewed)


def test_editor_only_sdk_is_never_admitted_as_a_pin(sandbox):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "embedded")
    inspect_source(repo)
    editor = sandbox / "editor-sdk"
    frontend = Path(os.environ["GRIPSACK_HOME"]) / "frontend/current"
    shutil.copytree(frontend.resolve(), editor)
    editor.chmod(0o700)
    value_file(editor / "private.ts", "excluded SDK canary")
    (repo / "node_modules/@gripsack").mkdir(parents=True)
    (repo / "node_modules/@gripsack/core").symlink_to(editor, target_is_directory=True)
    exclude(repo, "node_modules/@gripsack/core")
    reviewed = approve_source(repo)
    assert reviewed["policy"]["frontend"]["implementation"] == "frontend"
    assert reviewed["inventory"]["roots"] == ["repository", "frontend"]
    assert not any(entry["path"].startswith("pin/") for entry in reviewed["inventory"]["entries"])
    checked(repo, "embedded", reviewed)
    # Both a changed and a dangling excluded editor pin remain outside capture.
    value_file(editor / "private.ts", "changed outside approval")
    checked(repo, "embedded", reviewed)
    link = repo / "node_modules/@gripsack/core"
    link.unlink()
    link.symlink_to(sandbox / "missing-editor-sdk")
    checked(repo, "embedded", reviewed)


@pytest.mark.parametrize("route", ["relative_read", "absolute_read", "relative_import", "absolute_import"])
def test_excluded_bytes_are_unavailable_through_live_and_captured_paths(sandbox, route):
    repo = make_env_repo(sandbox / "repo", {})
    value_file(repo / ".venv/private.ts", "EXCLUDED_SECRET")
    exclude(repo, ".venv")
    path = repo / ".venv/private.ts"
    if route == "relative_read":
        expression = 'Deno.readTextFileSync(".venv/private.ts")'
    elif route == "absolute_read":
        expression = f"Deno.readTextFileSync({json.dumps(str(path))})"
    elif route == "relative_import":
        expression = 'await import("../.venv/private.ts")'
    else:
        expression = f"await import({json.dumps(path.as_uri())})"
    (repo / "hosts/testhost.ts").write_text(
        'import {defineEnv, module} from "@gripsack/core";\n'
        'let denied = false;\n'
        f'try {{ {expression}; }} catch {{ denied = true; }}\n'
        'if (!denied) throw new Error("excluded bytes became readable");\n'
        'export default defineEnv(() => ({modules:[module("denied", {install:[]})]}));\n'
    )
    reviewed = approve_source(repo)
    checked(repo, "denied", reviewed)


@pytest.mark.parametrize("target", [".venv/private.ts", ".venv", ".venv/../value.ts"])
def test_alias_into_an_exclusion_fails_before_evaluation(sandbox, target):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "legitimate")
    value_file(repo / ".venv/private.ts", "not admitted")
    exclude(repo, ".venv")
    (repo / "alternate").symlink_to(target)
    before = receipt_paths()
    failed = grip("trust", "inspect", str(repo), "--json", approve=False)
    assert failed.returncode != 0
    assert "repo/alternate" in failed.stderr, failed.stderr
    assert "excluded subtree repo/.venv" in failed.stderr, failed.stderr
    assert receipt_paths() == before


def test_excluded_sdk_bytes_cannot_return_via_explicit_import(sandbox):
    repo = value_repo(sandbox / "repo", "../node_modules/@gripsack/core/private.ts")
    external = sandbox / "editor-sdk"
    external.mkdir()
    value_file(external / "private.ts", "EXCLUDED_SDK")
    (repo / "node_modules/@gripsack").mkdir(parents=True)
    (repo / "node_modules/@gripsack/core").symlink_to(external)
    exclude(repo, "node_modules/@gripsack/core")
    reviewed = approve_source(repo)
    assert reviewed["policy"]["frontend"]["implementation"] == "frontend"
    failed = grip("check", "--host", "testhost", "--json", cwd=repo, approve=False)
    assert failed.returncode != 0, failed.stdout + failed.stderr
    assert "EXCLUDED_SDK" not in failed.stdout


def test_policy_changes_and_selected_source_invalidate_exact_approval(sandbox):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "before")
    exclude(repo, ".venv")
    reviewed = approve_source(repo)
    checked(repo, "before", reviewed)
    # Even an absent subtree is a policy change, authenticated by env.toml.
    exclude(repo, ".venv", "editor-cache")
    current = inspect_source(repo)
    assert current["capture_exclusions"] == [".venv", "editor-cache"]
    assert "repo/editor-cache" not in current["inventory"]["exclusions"]
    text = grip("trust", "inspect", str(repo), approve=False)
    assert text.returncode == 0, text.stderr
    assert "capture exclude: editor-cache" in text.stdout
    assert "frontend implementation: embedded SDK" in text.stdout
    assert current["bundle_digest"] != reviewed["bundle_digest"]
    assert current["policy_digest"] != reviewed["policy_digest"]
    stale = grip("trust", "add", str(repo), "--bundle", reviewed["bundle_digest"],
                 "--policy", reviewed["policy_digest"], approve=False)
    assert stale.returncode != 0
    rejected(repo)
    renewed = approve_source(repo)
    checked(repo, "before", renewed)
    value_file(repo / "value.ts", "after")
    rejected(repo)
    checked(repo, "after", approve_source(repo))


@pytest.mark.parametrize("path", ["", ".", "..", "/", "/tmp", "../outside", "a/../b", "a//b", "a/", "*.ts", "env.toml", "gripsack.ts", "hosts", "hosts/testhost.ts", "gripsack.lock", "locks", "locks/testhost.lock", "Gripsack.Lock", "LOCKS/testhost.lock"])
def test_malformed_outside_root_and_entrypoint_exclusions_fail(sandbox, path):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "unused")
    exclude(repo, path)
    before = receipt_paths()
    result = grip("trust", "inspect", str(repo), "--json", approve=False)
    assert result.returncode != 0
    assert "capture exclusion" in result.stderr, result.stderr
    assert receipt_paths() == before


@pytest.mark.parametrize("kind", ["symlink", "oversized"])
def test_configuration_bootstrap_is_bounded_and_never_follows_aliases(sandbox, kind):
    repo = value_repo(sandbox / "repo")
    config = repo / "env.toml"
    config.unlink()
    if kind == "symlink":
        external = sandbox / "external.toml"
        external.write_text('[capture]\nexclude = [".venv"]\n')
        config.symlink_to(external)
    else:
        with config.open("wb") as output:
            output.truncate(1024 * 1024 + 1)
    result = grip("trust", "inspect", str(repo), "--json", approve=False)
    assert result.returncode != 0
    assert "env.toml must be a regular non-symlink file" in result.stderr, result.stderr


def test_excluded_repository_file_cannot_be_materialized_from_live_tree(sandbox):
    repo = sandbox / "workspace"
    repo.mkdir()
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, profile, file, repoFile, symlinkTo,
    } from "@gripsack/core";
    export default defineWorkspace(() => workspace({outputs:[profile("files", {files:[
      file({source:repoFile(".venv/private.txt"), destination:symlinkTo("~/.excluded-copy")}),
    ]})]}));
    ''')
    (repo / ".venv").mkdir()
    (repo / ".venv/private.txt").write_text("EXCLUDED_PAYLOAD")
    exclude(repo, ".venv")
    approve_source(repo)
    result = grip("apply", cwd=repo, approve=False)
    assert result.returncode != 0, result.stdout + result.stderr
    assert not (sandbox / ".excluded-copy").exists()
    assert not (Path(os.environ["GRIPSACK_HOME"]) / "current").exists()


@pytest.mark.parametrize(("kind", "alias"), [("file", False), ("directory", False), ("file", True)])
def test_excluded_declared_input_has_no_live_tree_fallback(sandbox, kind, alias):
    repo = sandbox / "input-workspace"
    repo.mkdir()
    source = "alternate" if alias else ".venv"
    declaration = (f'inputFile("secret", "{source}/private.txt")' if kind == "file" else
                   f'inputDirectory("secret", "{source}", {{include:["private.txt"]}})')
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, task, exec, lit, inputFile, inputDirectory, input,
    } from "@gripsack/core";
    export default defineWorkspace(() => workspace({
      inputs:[''' + declaration + '''],
      outputs:[task("consume", {steps:[exec({argv:[lit("/usr/bin/cat"), input("secret")]})]})],
    }));
    ''')
    (repo / ".venv").mkdir()
    (repo / ".venv/private.txt").write_text("EXCLUDED_INPUT")
    if alias:
        (repo / "alternate").symlink_to(".venv", target_is_directory=True)
        exclude(repo, ".venv/private.txt")
    else:
        exclude(repo, ".venv")
    approve_source(repo)
    result = grip("task", "consume", cwd=repo, approve=False)
    assert result.returncode != 0, result.stdout + result.stderr
    assert "excluded by capture policy" in result.stderr, result.stdout + result.stderr
    assert "EXCLUDED_INPUT" not in result.stdout


def test_capture_policy_cannot_add_ambient_roots(sandbox):
    repo = value_repo(sandbox / "repo")
    (repo / "env.toml").write_text('[capture]\nroots = ["/usr"]\n')
    result = grip("trust", "inspect", str(repo), "--json", approve=False)
    assert result.returncode != 0
    assert "roots" in result.stderr, result.stderr


def test_ordinary_external_sdk_pin_still_wins_with_unrelated_exclusion(sandbox):
    repo, _ = pinned_value_repo(sandbox)
    exclude(repo, ".venv")
    reviewed = approve_source(repo)
    assert reviewed["policy"]["frontend"]["implementation"] == "pinned_frontend"
    checked(repo, "before", reviewed)


def test_policy_edit_after_capture_cannot_change_approved_evaluation(sandbox, monkeypatch):
    repo = value_repo(sandbox / "repo")
    value_file(repo / "value.ts", "captured")
    (repo / ".venv/bin").mkdir(parents=True)
    (repo / ".venv/bin/python3").symlink_to("/usr/bin/python3")
    exclude(repo, ".venv")
    control = pause_runtime(sandbox, monkeypatch)
    reviewed = approve_source(repo)
    before = receipt_paths()
    process = subprocess.Popen(
        [str(GRIP), "check", "--host", "testhost", "--json"], cwd=repo,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    try:
        wait_ready(control, 1, process)
        exclude(repo)
        (control / "release-1").touch()
        stdout, stderr = process.communicate(timeout=30)
        assert process.returncode == 0, stdout + stderr
        assert json.loads(stdout)["modules"] == ["captured"]
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate()
    receipt = new_receipt(before)
    assert receipt["source"] == reviewed["bundle_digest"]
    assert receipt["policy_digest"] == reviewed["policy_digest"]
    failed = grip("trust", "inspect", str(repo), "--json", approve=False)
    assert failed.returncode != 0
    assert "repo/.venv/bin/python3" in failed.stderr, failed.stderr
