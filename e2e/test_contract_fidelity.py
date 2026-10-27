"""Workspace preview fidelity, source admission and retained permissions.

Authoring-style parity, universal step references/resources/retries and old
receipt upgrades no longer exist in IR9. Workspace graph/check admission is
covered in test_workspace_contract and test_workspace_file_checks.
"""
import json
from pathlib import Path

import pytest
from conftest import grip, make_workspace_repo


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def current(sandbox):
    wire = json.loads((sandbox / ".local/share/gripsack/current/manifest.json").read_text())
    return wire.get("generation", wire)


def declaration(*, rendered=False, revision=1):
    content = 'templateFile({})' if rendered else 'identity()'
    return f'''import {{profile,file,repoFile,identity,templateFile,literalText,trackedCopyTo}} from "@gripsack/core";
export default profile("demo", {{files:[
 file({{source:repoFile("payload"),content:{content},destination:trackedCopyTo("~/.private")}}),
 file({{content:literalText("{revision}"),destination:trackedCopyTo("~/.revision")}})
]}});'''


@pytest.mark.parametrize("command", ["check", "plan", "apply"])
def test_missing_repository_source_fails_before_deployment(sandbox, command):
    repo = make_workspace_repo(sandbox / "env", declaration())
    result = grip(command, cwd=repo)
    assert result.returncode != 0
    assert not (sandbox / ".private").exists()
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_check_and_plan_validate_without_deploying(sandbox):
    repo = make_workspace_repo(sandbox / "env", declaration())
    (repo / "payload").write_text("one")
    for command in ("check", "plan"):
        run(repo, command)
        assert not (sandbox / ".private").exists()
        assert not (sandbox / ".local/share/gripsack/current").exists()
        assert not (repo / "gripsack.lock").exists()
    run(repo, "apply")
    first = current(sandbox)
    preview = run(repo, "plan")
    assert "~/.private (satisfied)" in preview.stdout
    assert "~/.revision (satisfied)" in preview.stdout
    run(repo, "apply")
    assert current(sandbox) == first


def test_private_copy_updates_and_exec_changes_keep_acquired_permissions(sandbox):
    repo = make_workspace_repo(sandbox / "env", declaration())
    destination = sandbox / ".private"
    destination.write_text("original")
    destination.chmod(0o600)
    source = repo / "payload"
    source.write_text("one")
    source.chmod(0o644)
    run(repo, "apply", "--take-over")
    first = current(sandbox)["number"]
    run(repo, "apply")
    assert current(sandbox)["number"] == first
    source.write_text("two")
    run(repo, "apply")
    assert destination.read_text() == "two"
    assert destination.stat().st_mode & 0o777 == 0o600
    source.chmod(0o755)
    run(repo, "apply")
    assert destination.stat().st_mode & 0o777 == 0o700
    source.chmod(0o644)
    run(repo, "apply")
    assert destination.stat().st_mode & 0o777 == 0o600
    run(repo, "store-verify")


@pytest.mark.parametrize("next_content", ["one", "two"])
def test_template_rollback_restores_full_permissions(sandbox, next_content):
    repo = make_workspace_repo(sandbox / "env", declaration(rendered=True))
    destination = sandbox / ".private"
    destination.write_text("original")
    destination.chmod(0o600)
    (repo / "payload").write_text("one")
    run(repo, "apply", "--take-over")
    first = current(sandbox)["number"]
    destination.chmod(0o640)
    (repo / "payload").write_text(next_content)
    (repo / "outputs/hello.ts").write_text(declaration(rendered=True, revision=2))
    run(repo, "apply", "--take-over")
    assert current(sandbox)["number"] != first
    assert destination.stat().st_mode & 0o777 == 0o640
    run(repo, "rollback", str(first))
    assert destination.read_text() == "one"
    assert destination.stat().st_mode & 0o777 == 0o600


def test_store_verify_detects_copy_source_exec_tampering(sandbox):
    repo = make_workspace_repo(sandbox / "env", declaration())
    (repo / "payload").write_text("payload")
    run(repo, "apply")
    state = current(sandbox)["modules"]["demo"]
    entry = next(entry for entry in state["entries"] if entry["to"].endswith("/.private"))
    payload = Path(state["store_path"]) / entry["from"]
    payload.chmod(0o555)
    result = grip("store-verify", cwd=repo)
    assert result.returncode != 0, result.stdout + result.stderr
    assert payload.read_text() == "payload"
