"""Generation-bound owned-policy migration through the ordinary transaction."""
import json
import signal
import subprocess

import pytest

from conftest import grip, make_env_repo
from test_workspace_files import declare, manifest, run


def predecessor(sandbox, *, origin=False, extra=False):
    entries = '"command": symlink("~/.migrated")'
    if extra:
        entries += ', "other": symlink("~/.unrelated-legacy")'
    repo = make_env_repo(
        sandbox / "legacy",
        'import { module, symlink } from "@gripsack/core";\n'
        'export default module("legacy", { config: { ' + entries + ' } });\n',
    )
    (repo / "command").write_text("#!/bin/sh\nprintf 'old\\n'\n")
    (repo / "command").chmod(0o755)
    if extra:
        (repo / "other").write_text("unrelated retained content\n")
    destination = sandbox / ".migrated"
    if origin:
        destination.write_text("original user content\n")
    run(repo, "apply", "--host", "testhost", *(["--take-over"] if origin else []))
    previous = manifest(sandbox)
    return destination, previous


def replacement(sandbox, *, hook=None, source=None):
    repo = sandbox / "workspace"
    extra = ""
    hooks = ""
    if hook is not None:
        extra = 'hook("verify", {trigger: "post_link", run: exec({argv: [' + ",".join(
            f"lit({json.dumps(arg)})" for arg in hook
        ) + ']})}),'
        hooks = ', hooks: ["verify"]'
    origin = source or 'source: repoFile("new-command")'
    declare(repo, extra + 'profile("personal", { files: [file({' + origin
            + ', content: identity(), destination: trackedCopyTo("~/.migrated")})]' + hooks + '})')
    if hook is not None:
        path = repo / "gripsack.ts"
        path.write_text('import { hook, exec } from "@gripsack/core";\n' + path.read_text())
    (repo / "new-command").write_text("#!/bin/sh\nprintf 'new\\n'\n")
    (repo / "new-command").chmod(0o755)
    return repo


def test_exact_predecessor_migration_preserves_origin_and_explicit_rollback(sandbox):
    destination, previous = predecessor(sandbox, origin=True)
    old_target = destination.readlink()
    repo = replacement(sandbox)
    before = manifest(sandbox)
    run(repo, "plan", "--migrate-from", str(previous["number"]))
    assert manifest(sandbox) == before
    assert destination.readlink() == old_target
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    current = manifest(sandbox)
    assert not destination.is_symlink()
    assert destination.stat().st_mode & 0o111
    assert current["modules"]["personal"]["entries"][0]["prior"] == previous["modules"]["legacy"]["entries"][0]["prior"]
    run(repo, "apply")
    assert manifest(sandbox) == current
    run(repo, "rollback", str(previous["number"]))
    assert destination.readlink() == old_target
    assert manifest(sandbox)["number"] == previous["number"]
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    declare(repo, 'profile("empty", {})')
    run(repo, "apply")
    assert destination.read_text() == "original user content\n"


@pytest.mark.parametrize("tamper", ["stale", "foreign", "drift"])
def test_migration_refuses_stale_generation_foreign_and_drift(sandbox, tamper):
    destination, previous = predecessor(sandbox)
    repo = replacement(sandbox)
    generation = previous["number"]
    if tamper == "stale":
        generation += 99
    elif tamper == "foreign":
        destination.unlink()
        destination.write_text("foreign bytes\n")
    else:
        destination.unlink()
        destination.symlink_to(sandbox / "different-target")
    before = destination.readlink() if destination.is_symlink() else destination.read_bytes()
    for command in ("plan", "apply"):
        refused = grip(command, "--migrate-from", str(generation), cwd=repo)
        assert refused.returncode != 0
        assert manifest(sandbox) == previous
        after = destination.readlink() if destination.is_symlink() else destination.read_bytes()
        assert after == before


@pytest.mark.parametrize("unavailable", ["content", "runtime", "package"])
def test_migration_preflight_keeps_old_command_when_replacement_unavailable(sandbox, unavailable):
    destination, previous = predecessor(sandbox)
    old_target = destination.readlink()
    repo = replacement(sandbox, hook=["/no/such/gripsack-migration-runtime"] if unavailable == "runtime" else None)
    if unavailable == "content":
        (repo / "new-command").unlink()
    elif unavailable == "package":
        declare(repo, '''pkg("missing", {producer: provider(fileFetch("absent-payload")),
          target: {os: "linux", arch: "x86_64"}, layout: {kind: "relocatable"}}),
          profile("personal", {files: [file({source: artifactFile("missing", "command"),
          content: identity(), destination: trackedCopyTo("~/.migrated")})]})''')
    refused = grip("apply", "--migrate-from", str(previous["number"]), cwd=repo)
    assert refused.returncode != 0
    assert manifest(sandbox) == previous
    assert destination.readlink() == old_target
    assert destination.read_bytes() == old_target.read_bytes()
    command = subprocess.run([str(destination)], capture_output=True, text=True, timeout=10)
    assert command.returncode == 0, command.stderr
    assert command.stdout == "old\n"


@pytest.mark.parametrize("cut", ["workspace-file:1", "after-prune"])
def test_interrupted_migration_restores_predecessor_before_retry(sandbox, monkeypatch, cut):
    destination, previous = predecessor(sandbox)
    old_target = destination.readlink()
    repo = replacement(sandbox)
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", cut)
    interrupted = grip("apply", "--migrate-from", str(previous["number"]), cwd=repo)
    assert interrupted.returncode == -signal.SIGABRT, interrupted.stderr
    assert manifest(sandbox) == previous
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    run(repo, "apply")
    monkeypatch.delenv("GRIPSACK_FS_RECOVER_ONLY")
    assert destination.readlink() == old_target
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    assert not destination.is_symlink()


def test_failed_postactivation_verification_retains_migration_until_explicit_rollback(sandbox):
    destination, previous = predecessor(sandbox)
    old_target = destination.readlink()
    repo = replacement(sandbox, hook=["/bin/sh", "-c", "exit 9"])
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    migrated = manifest(sandbox)
    assert migrated["number"] != previous["number"]
    assert not destination.is_symlink()
    rows = json.loads(run(repo, "hooks", "list", "--json").stdout)["intents"]
    assert any(row["state"]["kind"] == "failed" for row in rows)
    run(repo, "apply")
    assert manifest(sandbox) == migrated
    assert json.loads(run(repo, "hooks", "list", "--json").stdout)["intents"] == rows
    configuration = sandbox / ".config/gripsack"
    configuration.mkdir(parents=True, exist_ok=True)
    (configuration / "config.toml").write_text("[settings]\nkeep_generations = 1\n")
    run(repo, "gc")
    assert (sandbox / ".local/share/gripsack/generations" / str(previous["number"])).is_dir()
    assert old_target.is_file()
    run(repo, "rollback", str(previous["number"]))
    assert destination.readlink() == old_target


def test_partial_migration_transfers_only_selected_ownership(sandbox):
    destination, previous = predecessor(sandbox, extra=True)
    unrelated = sandbox / ".unrelated-legacy"
    other_target = unrelated.readlink()
    repo = replacement(sandbox)
    run(repo, "apply", "personal", "--migrate-from", str(previous["number"]))
    active = manifest(sandbox)
    assert set(active["modules"]) == {"legacy", "personal"}
    assert [entry["to"] for entry in active["modules"]["legacy"]["entries"]] == ["~/.unrelated-legacy"]
    assert unrelated.readlink() == other_target
    assert not destination.is_symlink()
    run(repo, "rollback", str(previous["number"]))
    assert destination.is_symlink()
    assert unrelated.readlink() == other_target


def directory_predecessor(sandbox, *, origin=False):
    repo = make_env_repo(
        sandbox / "legacy-directory",
        'import { module, symlink } from "@gripsack/core";\n'
        'export default module("legacy", {config: {"tree": symlink("~/.migrated-tree")}});\n',
    )
    tree = repo / "tree"
    (tree / "nested").mkdir(parents=True)
    (tree / "nested/command").write_text("#!/bin/sh\nprintf 'old tree\\n'\n")
    (tree / "nested/command").chmod(0o755)
    (tree / "data").write_text("old tree data\n")
    destination = sandbox / ".migrated-tree"
    if origin:
        destination.write_text("original directory-path user file\n")
        destination.chmod(0o600)
    run(repo, "apply", "--host", "testhost", *(["--take-over"] if origin else []))
    return destination, destination.readlink(), manifest(sandbox)


def directory_replacement(sandbox):
    repo = sandbox / "workspace-directory"
    declare(repo, '''profile("personal", {files: [
      file({source: repoTree("tree"), content: identity(),
        destination: trackedCopyTo("~/.migrated-tree")})
    ]})''')
    entrypoint = repo / "gripsack.ts"
    entrypoint.write_text('import {repoTree} from "@gripsack/core";\n' + entrypoint.read_text())
    (repo / "tree/nested").mkdir(parents=True)
    (repo / "tree/nested/command").write_text("#!/bin/sh\nprintf 'new tree\\n'\n")
    (repo / "tree/nested/command").chmod(0o755)
    (repo / "tree/data").write_text("new tree data\n")
    return repo


def test_directory_migration_plan_apply_rollback_reactivation_and_gc(sandbox):
    destination, old_target, previous = directory_predecessor(sandbox)
    repo = directory_replacement(sandbox)
    before = {str(path.relative_to(old_target)): path.read_bytes()
              for path in old_target.rglob("*") if path.is_file()}
    run(repo, "plan", "--migrate-from", str(previous["number"]))
    assert destination.is_symlink()
    assert destination.readlink() == old_target
    assert not list(sandbox.glob(".gripsack-directory-*")), "plan created preparation artifacts"
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    migrated = manifest(sandbox)
    assert not destination.is_symlink()
    assert destination.is_dir()
    assert migrated["directory_transitions"][0]["destination"] == str(destination)
    keys = {entry["key"] for entry in migrated["modules"]["personal"]["entries"]}
    assert keys == {str(destination / "data"), str(destination / "nested/command")}
    invocation = subprocess.run([str(destination / "nested/command")],
                                capture_output=True, text=True, timeout=10)
    assert invocation.returncode == 0, invocation.stderr
    assert invocation.stdout == "new tree\n"
    assert {str(path.relative_to(old_target)): path.read_bytes()
            for path in old_target.rglob("*") if path.is_file()} == before
    configuration = sandbox / ".config/gripsack"
    configuration.mkdir(parents=True, exist_ok=True)
    (configuration / "config.toml").write_text("[settings]\nkeep_generations = 1\n")
    run(repo, "gc")
    assert old_target.is_dir()
    run(repo, "apply")
    assert manifest(sandbox) == migrated
    run(repo, "rollback", str(previous["number"]))
    assert destination.readlink() == old_target
    run(repo, "rollback", str(migrated["number"]))
    assert not destination.is_symlink()
    assert (destination / "nested/command").read_text().endswith("printf 'new tree\\n'\n")


def test_directory_failed_preparation_never_replaces_old_layout(sandbox):
    destination, old_target, previous = directory_predecessor(sandbox)
    repo = directory_replacement(sandbox)
    (repo / "tree/nested/command").unlink()
    # Missing captured input is diagnosed before any parent cutover.
    declare(repo, '''profile("personal", {files: [
      file({source: repoFile("tree/nested/command"), content: identity(),
        destination: trackedCopyTo("~/.migrated-tree/nested/command")})
    ]})''')
    refused = grip("apply", "--migrate-from", str(previous["number"]), cwd=repo)
    assert refused.returncode != 0
    assert destination.readlink() == old_target
    assert manifest(sandbox) == previous
    assert not list(sandbox.glob(".gripsack-directory-*"))


@pytest.mark.parametrize("cut", [
    "directory-before-publish", "directory-after-publish", "workspace-file:1", "after-prune",
])
def test_directory_crash_recovery_is_child_before_parent(sandbox, monkeypatch, cut):
    destination, old_target, previous = directory_predecessor(sandbox)
    repo = directory_replacement(sandbox)
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", cut)
    interrupted = grip("apply", "--migrate-from", str(previous["number"]), cwd=repo)
    assert interrupted.returncode == -signal.SIGABRT, interrupted.stderr
    assert manifest(sandbox) == previous
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    run(repo, "apply")
    monkeypatch.delenv("GRIPSACK_FS_RECOVER_ONLY")
    assert destination.readlink() == old_target
    assert (old_target / "data").read_text() == "old tree data\n"
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    assert not destination.is_symlink()


def test_directory_rollback_preserves_foreign_child_and_current_selection(sandbox):
    destination, old_target, previous = directory_predecessor(sandbox)
    repo = directory_replacement(sandbox)
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    migrated = manifest(sandbox)
    foreign = destination / "nested/foreign"
    foreign.write_text("do not delete\n")
    refused = grip("rollback", str(previous["number"]), cwd=repo)
    assert refused.returncode != 0
    assert foreign.read_text() == "do not delete\n"
    assert manifest(sandbox) == migrated
    assert (destination / "data").read_text() == "new tree data\n"
    assert (old_target / "data").read_text() == "old tree data\n"


def test_directory_ancestor_replacement_blocks_apply_without_touching_target(sandbox):
    destination, _, previous = directory_predecessor(sandbox)
    repo = directory_replacement(sandbox)
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    migrated = manifest(sandbox)
    (destination / "nested").rename(sandbox / "moved-owned-directory")
    foreign = sandbox / "foreign-directory"
    foreign.mkdir()
    (foreign / "command").write_text("foreign command\n")
    (destination / "nested").symlink_to(foreign, target_is_directory=True)
    refused = grip("apply", cwd=repo)
    assert refused.returncode != 0
    assert (foreign / "command").read_text() == "foreign command\n"
    assert manifest(sandbox) == migrated


def test_directory_undeclare_restores_original_ownership_lineage(sandbox):
    destination, _, previous = directory_predecessor(sandbox, origin=True)
    repo = directory_replacement(sandbox)
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    migrated = manifest(sandbox)
    declare(repo, 'profile("empty", {})')
    run(repo, "apply")
    assert destination.is_file() and not destination.is_symlink()
    assert destination.read_text() == "original directory-path user file\n"
    assert destination.stat().st_mode & 0o777 == 0o600
    empty = manifest(sandbox)
    destination.write_text("foreign replacement after ownership ended\n")
    refused = grip("rollback", str(migrated["number"]), cwd=repo)
    assert refused.returncode != 0
    assert destination.read_text() == "foreign replacement after ownership ended\n"
    assert manifest(sandbox) == empty
    destination.write_text("original directory-path user file\n")
    # Reactivation captures this immediate file, not the older managed link.
    run(repo, "rollback", str(migrated["number"]))
    assert destination.is_dir() and not destination.is_symlink()
    assert (destination / "data").read_text() == "new tree data\n"


def test_directory_multiple_incarnations_and_interrupted_rollback(sandbox, monkeypatch):
    destination, old_target, previous = directory_predecessor(sandbox)
    repo = directory_replacement(sandbox)
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    first = manifest(sandbox)
    first_inode = destination.stat().st_ino
    run(repo, "rollback", str(previous["number"]))
    assert destination.readlink() == old_target
    (repo / "tree/data").write_text("second migration epoch\n")
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    second = manifest(sandbox)
    second_inode = destination.stat().st_ino
    assert second_inode != first_inode
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "after-rollback-restore")
    interrupted = grip("rollback", str(first["number"]), cwd=repo)
    assert interrupted.returncode == -signal.SIGABRT, interrupted.stderr
    assert manifest(sandbox) == second
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    run(repo, "apply")
    monkeypatch.delenv("GRIPSACK_FS_RECOVER_ONLY")
    assert destination.stat().st_ino == second_inode
    assert (destination / "data").read_text() == "second migration epoch\n"
    run(repo, "rollback", str(first["number"]))
    assert destination.stat().st_ino == first_inode
    assert (destination / "data").read_text() == "new tree data\n"
    run(repo, "rollback", str(second["number"]))
    assert destination.stat().st_ino == second_inode
    assert (destination / "data").read_text() == "second migration epoch\n"


def test_directory_transition_preserves_physical_destination_alias_identity(sandbox):
    destination, old_target, previous = directory_predecessor(sandbox)
    repo = directory_replacement(sandbox)
    (sandbox / ".destination-alias").symlink_to(sandbox, target_is_directory=True)
    entrypoint = repo / "gripsack.ts"
    entrypoint.write_text(entrypoint.read_text().replace(
        'trackedCopyTo("~/.migrated-tree")',
        'trackedCopyTo("~/.destination-alias/.migrated-tree")',
    ))
    run(repo, "plan", "--migrate-from", str(previous["number"]))
    assert destination.readlink() == old_target
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    migrated = manifest(sandbox)
    assert migrated["directory_transitions"][0]["destination"] == str(destination)
    assert {entry["key"] for entry in migrated["modules"]["personal"]["entries"]} == {
        str(destination / "data"), str(destination / "nested/command"),
    }
    run(repo, "rollback", str(previous["number"]))
    assert destination.readlink() == old_target
