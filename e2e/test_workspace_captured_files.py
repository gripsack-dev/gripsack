"""Captured source ergonomics: no body duplication, generator, or manifest parity."""
import hashlib
import json
import os
import subprocess

import pytest

from conftest import grip
from test_capture_policy import exclude
from test_workspace_files import manifest, run


IMPORTS = '''import {
  defineWorkspace, workspace, profile, file, repoFile, repoTree,
  templateFile, identity, trackedCopyTo, symlinkTo, literalText,
} from "@gripsack/core";
'''


def declare(repo, files):
    repo.mkdir(exist_ok=True)
    (repo / "gripsack.ts").write_text(
        IMPORTS + "export default defineWorkspace(() => workspace({ outputs: [\n"
        + 'profile("files", { files: [\n' + files + "\n] })] }));\n"
    )


def tree_file(selection=None, policy="trackedCopyTo"):
    selection = json.dumps(selection or {})
    return (f'file({{ source: repoTree("configs", {selection}), content: identity(), '
            f'destination: {policy}("~/.captured-tree") }})')


def template_files(value, digest=None):
    variables = json.dumps({"value": value})
    claim = "" if digest is None else ", " + json.dumps(digest)
    return ",\n".join(
        f'file({{ source: repoFile("command.tmpl"), content: templateFile({variables}{claim}), '
        f'destination: {policy}("~/{name}") }})'
        for name, policy in [(".captured-copy", "trackedCopyTo"), (".captured-link", "symlinkTo")]
    )


def test_template_file_source_variables_modes_digest_and_retained_rollback(sandbox):
    repo = sandbox / "template-file"
    declare(repo, template_files("first"))
    source = repo / "command.tmpl"
    source.write_text("#!/bin/sh\nprintf '%s\\n' '{{ value }}'\n")
    source.chmod(0o755)
    run(repo, "apply")
    first = manifest(sandbox)
    first_store = first["modules"]["files"]["store_path"]
    for name in [".captured-copy", ".captured-link"]:
        target = sandbox / name
        assert target.stat().st_mode & 0o111
        assert subprocess.check_output([str(target)], text=True) == "first\n"
    assert not (sandbox / ".captured-copy").is_symlink()
    assert (sandbox / ".captured-link").is_symlink()

    declare(repo, template_files("second"))
    run(repo, "apply")
    second_store = manifest(sandbox)["modules"]["files"]["store_path"]
    assert second_store != first_store
    source.write_text("#!/bin/sh\nprintf 'source:%s\\n' '{{ value }}'\n")
    run(repo, "apply")
    source_store = manifest(sandbox)["modules"]["files"]["store_path"]
    assert source_store != second_store
    assert subprocess.check_output([str(sandbox / ".captured-copy")], text=True) == "source:second\n"
    source.chmod(0o644)
    run(repo, "apply")
    assert manifest(sandbox)["modules"]["files"]["store_path"] != source_store
    assert not (sandbox / ".captured-copy").stat().st_mode & 0o111
    rendered = (sandbox / ".captured-copy").read_bytes()
    declare(repo, template_files("second", hashlib.sha256(rendered).hexdigest()))
    run(repo, "apply")
    retained = manifest(sandbox)
    declare(repo, template_files("second", "0" * 64))
    refused = grip("apply", cwd=repo)
    assert refused.returncode != 0
    assert "result digest" in refused.stderr
    assert manifest(sandbox) == retained
    assert (sandbox / ".captured-copy").read_bytes() == rendered
    source.unlink()
    run(repo, "rollback", str(first["number"]))
    assert subprocess.check_output([str(sandbox / ".captured-copy")], text=True) == "first\n"
    run(repo, "store-verify")


def test_repo_tree_alias_retarget_add_remove_preserves_siblings_and_rollback(sandbox):
    repo = sandbox / "repository-tree"
    declare(repo, tree_file({"include": ["alias", "file-alias"]}))
    configs = repo / "configs"
    for name, value in [("one", "first"), ("two", "second")]:
        directory = configs / name
        directory.mkdir(parents=True)
        (directory / "data").write_text(value + "\n")
        script = directory / "command"
        script.write_text('#!/bin/sh\ncat "$(dirname "$0")/data"\n')
        script.chmod(0o755)
    (configs / "alias").symlink_to("one", target_is_directory=True)
    (configs / "file-alias").symlink_to("one/data")
    destination = sandbox / ".captured-tree"
    destination.mkdir()
    (destination / "unrelated").write_text("not owned\n")
    run(repo, "apply")
    first = manifest(sandbox)
    command = destination / "alias/command"
    assert not command.is_symlink()
    assert subprocess.check_output([str(command)], text=True) == "first\n"
    assert (destination / "file-alias").read_text() == "first\n"
    (configs / "alias").unlink()
    (configs / "alias").symlink_to("two", target_is_directory=True)
    (configs / "file-alias").unlink()
    (configs / "two/added").write_text("new\n")
    (destination / "alias/data").write_text("user modification\n")
    run(repo, "apply")
    assert (destination / "alias/data").read_text() == "user modification\n"
    run(repo, "apply", "--take-over")
    assert manifest(sandbox)["modules"]["files"]["store_path"] != first["modules"]["files"]["store_path"]
    assert subprocess.check_output([str(command)], text=True) == "second\n"
    assert not (destination / "file-alias").exists()
    assert (destination / "alias/added").read_text() == "new\n"
    repaired_entries = manifest(sandbox)["modules"]["files"]["entries"]
    for name in ["alias/command", "alias/added"]:
        entry = next(entry for entry in repaired_entries if entry["to"].endswith("/" + name))
        assert entry.get("prior") is None, "intact managed bytes are not a user origin"
    assert (destination / "unrelated").read_text() == "not owned\n"
    (configs / "two/added").unlink()
    run(repo, "apply")
    assert not (destination / "alias/added").exists()
    run(repo, "rollback", str(first["number"]))
    assert subprocess.check_output([str(command)], text=True) == "first\n"
    assert (destination / "file-alias").read_text() == "first\n"
    assert (destination / "unrelated").read_text() == "not owned\n"
    run(repo, "store-verify")


def test_repo_tree_takeover_retains_original_bytes_modes_and_absent_origins(sandbox):
    repo = sandbox / "tree-takeover-lineage"
    declare(repo, "")
    run(repo, "apply")
    empty = manifest(sandbox)
    configs = repo / "configs"
    configs.mkdir()
    destination = sandbox / ".captured-tree"
    destination.mkdir()
    for name in ["foreign", "matching", "managed", "drifted"]:
        (configs / name).write_text("desired\n")
    originals = {"foreign": "original user bytes\n", "matching": "desired\n"}
    for name, content in originals.items():
        target = destination / name
        target.write_text(content)
        target.chmod(0o600)
    declare(repo, tree_file())
    run(repo, "apply", "--take-over")
    acquired = manifest(sandbox)
    original_priors = {
        entry["to"]: entry["prior"]
        for entry in acquired["modules"]["files"]["entries"]
        if entry.get("prior")
    }
    assert len(original_priors) == 2
    (destination / "drifted").write_text("preserved user drift\n")
    (destination / "drifted").chmod(0o640)
    (configs / "managed").write_text("updated managed bytes\n")
    run(repo, "apply")
    assert (destination / "drifted").read_text() == "preserved user drift\n"
    assert (destination / "drifted").stat().st_mode & 0o7777 == 0o640
    (configs / "managed").write_text("managed update during takeover\n")
    run(repo, "apply", "--take-over")
    repaired = manifest(sandbox)
    for entry in repaired["modules"]["files"]["entries"]:
        if entry["to"] in original_priors:
            assert entry["prior"] == original_priors[entry["to"]]
        elif entry["to"].endswith("/managed"):
            assert entry.get("prior") is None
        elif entry["to"].endswith("/drifted"):
            assert entry["prior"]["mode"] == 0o640
    assert (destination / "managed").read_text() == "managed update during takeover\n"

    # Both rollback and undeclaration restore real user origins, never the
    # bytes of a destination that was created and remained managed.
    for rollback in [True, False]:
        if rollback:
            run(repo, "rollback", str(empty["number"]))
        else:
            run(repo, "apply", "--take-over")
            declare(repo, "")
            run(repo, "apply")
        for name, content in originals.items():
            assert (destination / name).read_text() == content
            assert (destination / name).stat().st_mode & 0o7777 == 0o600
        assert not (destination / "managed").exists()
        assert (destination / "drifted").read_text() == "preserved user drift\n"
        assert (destination / "drifted").stat().st_mode & 0o7777 == 0o640
    run(repo, "store-verify")


def test_repo_tree_filters_aliases_and_traverses_include_ancestors(sandbox):
    repo = sandbox / "tree-selection"
    declare(repo, tree_file({"include": ["deep/selected", "alias"], "exclude": ["alias"]}))
    configs = repo / "configs"
    (configs / "deep/selected").mkdir(parents=True)
    (configs / "deep/selected/value").write_text("selected\n")
    (configs / "deep/other").write_text("not selected\n")
    (configs / "alias").symlink_to("deep", target_is_directory=True)
    run(repo, "apply")
    assert (sandbox / ".captured-tree/deep/selected/value").read_text() == "selected\n"
    assert not (sandbox / ".captured-tree/deep/other").exists()
    assert not (sandbox / ".captured-tree/alias").exists()
    prior = manifest(sandbox)
    # Selection does not weaken whole-source capture. An unsafe namespace
    # requires a capture exclusion, even when no file declaration visits it.
    (configs / "alias").unlink()
    (configs / "alias").symlink_to(".", target_is_directory=True)
    refused = grip("apply", cwd=repo)
    assert refused.returncode != 0
    assert "cycle" in refused.stderr
    assert manifest(sandbox) == prior


@pytest.mark.parametrize("kind", ["escape", "dangling", "special"])
def test_tree_selection_does_not_replace_source_capture_exclusions(sandbox, kind):
    repo = sandbox / "capture-policy"
    declare(repo, tree_file({"exclude": ["hostile"]}))
    configs = repo / "configs"
    configs.mkdir()
    (configs / "good").write_text("approved\n")
    hostile = configs / "hostile"
    if kind == "special":
        os.mkfifo(hostile)
    else:
        hostile.symlink_to("/etc/passwd" if kind == "escape" else "missing")
    # Capturing the repository precedes evaluating the source selector. A tree
    # filter cannot authorize unsafe live objects or hide them from approval.
    refused = grip("plan", cwd=repo)
    assert refused.returncode != 0
    assert not (sandbox / ".captured-tree").exists()
    exclude(repo, "configs/hostile")
    run(repo, "apply")
    assert (sandbox / ".captured-tree/good").read_text() == "approved\n"
    assert not (sandbox / ".captured-tree/hostile").exists()


def test_repo_tree_expanded_destination_collisions_refuse_before_mutation(sandbox):
    repo = sandbox / "tree-collision"
    declare(repo, tree_file() + ',\nfile({ content: literalText("collision"), destination: trackedCopyTo("~/.captured-tree/value") })')
    (repo / "configs").mkdir()
    (repo / "configs/value").write_text("captured\n")
    refused = grip("apply", cwd=repo)
    assert refused.returncode != 0
    assert not (sandbox / ".captured-tree/value").exists()


def test_repo_tree_symlink_policy_is_per_file_not_directory_ownership(sandbox):
    repo = sandbox / "tree-links"
    declare(repo, tree_file(policy="symlinkTo"))
    (repo / "configs/nested").mkdir(parents=True)
    (repo / "configs/nested/value").write_text("captured\n")
    destination = sandbox / ".captured-tree"
    destination.mkdir()
    (destination / "unrelated").write_text("foreign\n")
    run(repo, "apply")
    assert (destination / "nested/value").is_symlink()
    assert not (destination / "nested").is_symlink()
    assert (destination / "nested/value").read_text() == "captured\n"
    assert (destination / "unrelated").read_text() == "foreign\n"
