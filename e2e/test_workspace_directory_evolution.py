"""Owned directory shapes evolve without acquiring authority over foreign data."""
import shutil
import signal

import pytest

from conftest import grip
from test_workspace_files import manifest, run
from test_workspace_migration import directory_predecessor, directory_replacement


def migrated(sandbox):
    destination, old_target, previous = directory_predecessor(sandbox)
    repo = directory_replacement(sandbox)
    run(repo, "apply", "--migrate-from", str(previous["number"]))
    return repo, destination, old_target


def grow(repo, destination):
    borrowed = destination / "borrowed"
    borrowed.mkdir()
    (borrowed / "user-note").write_text("foreign content\n")
    (repo / "tree/borrowed").mkdir()
    (repo / "tree/borrowed/config").write_text("managed configuration\n")
    (repo / "tree/new/deep").mkdir(parents=True)
    (repo / "tree/new/deep/value").write_text("new managed leaf\n")
    return borrowed.stat().st_ino


def recover_only(repo, monkeypatch):
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    run(repo, "apply")
    monkeypatch.delenv("GRIPSACK_FS_RECOVER_ONLY")


def test_growth_retirement_and_rollback_preserve_borrowed_ancestors(sandbox):
    repo, destination, old_target = migrated(sandbox)
    original = manifest(sandbox)
    borrowed_inode = grow(repo, destination)
    run(repo, "plan")
    assert manifest(sandbox) == original
    assert not (destination / "new").exists()
    assert not (destination / "borrowed/config").exists()
    run(repo, "apply")
    grown = manifest(sandbox)
    assert grown["number"] != original["number"]
    assert (destination / "new/deep/value").read_text() == "new managed leaf\n"
    assert (destination / "borrowed/config").read_text() == "managed configuration\n"
    assert (destination / "borrowed").stat().st_ino == borrowed_inode
    assert (destination / "borrowed/user-note").read_text() == "foreign content\n"
    assert (old_target / "data").read_text() == "old tree data\n"
    run(repo, "apply")
    assert manifest(sandbox) == grown

    shutil.rmtree(repo / "tree/new")
    shutil.rmtree(repo / "tree/borrowed")
    run(repo, "apply")
    retired = manifest(sandbox)
    assert not (destination / "new").exists()
    assert not (destination / "borrowed/config").exists()
    assert (destination / "borrowed").stat().st_ino == borrowed_inode
    assert (destination / "borrowed/user-note").read_text() == "foreign content\n"
    run(repo, "rollback", str(grown["number"]))
    assert (destination / "new/deep/value").read_text() == "new managed leaf\n"
    assert (destination / "borrowed/config").read_text() == "managed configuration\n"
    assert (destination / "borrowed").stat().st_ino == borrowed_inode
    assert (destination / "borrowed/user-note").read_text() == "foreign content\n"
    run(repo, "rollback", str(retired["number"]))
    assert not (destination / "new").exists()
    assert (destination / "borrowed/user-note").read_text() == "foreign content\n"


@pytest.mark.parametrize("cut", [
    "directory-evolve-before-publish", "directory-evolve-after-publish",
    "workspace-file:1", "after-prune",
])
def test_growth_crash_restores_previous_shape_and_keeps_foreign_data(sandbox, monkeypatch, cut):
    repo, destination, old_target = migrated(sandbox)
    previous = manifest(sandbox)
    borrowed_inode = grow(repo, destination)
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", cut)
    interrupted = grip("apply", cwd=repo)
    assert interrupted.returncode == -signal.SIGABRT, interrupted.stderr
    assert manifest(sandbox) == previous
    recover_only(repo, monkeypatch)
    assert manifest(sandbox) == previous
    assert not (destination / "new").exists()
    assert not (destination / "borrowed/config").exists()
    assert (destination / "borrowed").stat().st_ino == borrowed_inode
    assert (destination / "borrowed/user-note").read_text() == "foreign content\n"
    assert (destination / "data").read_text() == "new tree data\n"
    assert (old_target / "data").read_text() == "old tree data\n"
    run(repo, "apply")
    assert (destination / "new/deep/value").read_text() == "new managed leaf\n"
    assert (destination / "borrowed/config").read_text() == "managed configuration\n"


def test_retirement_crash_restores_removed_leaves_and_directory_shape(sandbox, monkeypatch):
    repo, destination, _ = migrated(sandbox)
    borrowed_inode = grow(repo, destination)
    run(repo, "apply")
    previous = manifest(sandbox)
    shutil.rmtree(repo / "tree/new")
    shutil.rmtree(repo / "tree/borrowed")
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "after-prune")
    interrupted = grip("apply", cwd=repo)
    assert interrupted.returncode == -signal.SIGABRT, interrupted.stderr
    assert manifest(sandbox) == previous
    recover_only(repo, monkeypatch)
    assert (destination / "new/deep/value").read_text() == "new managed leaf\n"
    assert (destination / "borrowed/config").read_text() == "managed configuration\n"
    assert (destination / "borrowed").stat().st_ino == borrowed_inode
    assert (destination / "borrowed/user-note").read_text() == "foreign content\n"
    run(repo, "apply")
    assert not (destination / "new").exists()
    assert not (destination / "borrowed/config").exists()
    assert (destination / "borrowed/user-note").read_text() == "foreign content\n"


def replace_nested_directory(repo):
    shutil.rmtree(repo / "tree/nested")
    (repo / "tree/nested").write_text("replacement leaf\n")


def test_directory_to_leaf_plan_is_readonly_and_rollback_restores_tree(sandbox):
    repo, destination, _ = migrated(sandbox)
    previous = manifest(sandbox)
    replace_nested_directory(repo)
    run(repo, "plan")
    assert manifest(sandbox) == previous
    assert (destination / "nested/command").read_text().endswith("printf 'new tree\\n'\n")
    run(repo, "apply")
    assert (destination / "nested").read_text() == "replacement leaf\n"
    run(repo, "rollback", str(previous["number"]))
    assert (destination / "nested/command").read_text().endswith("printf 'new tree\\n'\n")


def test_directory_to_leaf_refuses_foreign_descendants_without_mutation(sandbox):
    repo, destination, _ = migrated(sandbox)
    previous = manifest(sandbox)
    foreign = destination / "nested/user-note"
    foreign.write_text("foreign content\n")
    replace_nested_directory(repo)
    for command in ("plan", "apply"):
        refused = grip(command, cwd=repo)
        assert refused.returncode != 0
        assert foreign.read_text() == "foreign content\n"
        assert (destination / "nested/command").read_text().endswith("printf 'new tree\\n'\n")
        assert manifest(sandbox) == previous


def test_leaf_to_directory_plan_and_rollback_preserve_the_old_file(sandbox):
    repo, destination, _ = migrated(sandbox)
    previous = manifest(sandbox)
    source = repo / "tree/data"
    source.unlink()
    source.mkdir()
    (source / "child").write_text("expanded leaf\n")
    run(repo, "plan")
    assert manifest(sandbox) == previous
    assert (destination / "data").read_text() == "new tree data\n"
    run(repo, "apply")
    assert (destination / "data/child").read_text() == "expanded leaf\n"
    run(repo, "rollback", str(previous["number"]))
    assert (destination / "data").read_text() == "new tree data\n"
