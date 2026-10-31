"""Apply lifecycle: production, publication, source changes and pruning."""

import json
import os
import shutil
import subprocess
from conftest import native_target, grip, make_workspace_repo, make_tarball


def test_apply_creates_generation_and_symlinks(sandbox):
    payload = make_tarball(
        sandbox / "myenv" / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ artifactFile, file, fileFetch, identity, pkg, profile, provider, symlinkTo }} from "@gripsack/core";

export default [pkg("hello-payload", {{ producer: provider(fileFetch("{payload.name}")), commands: {{}}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
profile("hello", {{ files: [
    file({{ source: artifactFile("hello-payload", "bin/hello"), content: identity(), destination: symlinkTo("~/.local/bin/hello") }})
  ] }})];
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    home = sandbox / ".local/share/gripsack"
    assert (home / "generations/1").is_dir()
    assert (home / "current").is_symlink()
    assert (sandbox / ".local/bin/hello").is_symlink()




def test_tree_entries_and_prune_on_undeclare(sandbox):
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "settings.json").write_text('{"theme": "mocha"}\n')
    (confdir / "keymap.json").write_text("[]\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoTree, trackedCopyTo } from "@gripsack/core";

export default profile("zed", { files: [
    file({ source: repoTree("configs/zed"), content: identity(), destination: trackedCopyTo("~/.config/zed") })
  ] });
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".config" / "zed"
    assert (deployed / "settings.json").exists()
    assert (deployed / "keymap.json").exists()

    # drop a file from the tree -> pruned on next apply
    (confdir / "keymap.json").unlink()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (deployed / "settings.json").exists()
    assert not (deployed / "keymap.json").exists()


def test_owned_prune_on_undeclare(sandbox):
    """Regression: prune-on-undeclare must work for owned symlinks too —
    the recorded hash is the source content, so the tracked_copy hash
    check can never match a symlink (gripsack-exec apply.rs)."""
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    (confdir / "b.txt").write_text("b\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoTree, symlinkTo } from "@gripsack/core";

export default profile("zed", { files: [
    file({ source: repoTree("configs/zed"), content: identity(), destination: symlinkTo("~/.config/zed") })
  ] });
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".config" / "zed"
    assert (deployed / "a.txt").is_symlink()
    assert (deployed / "b.txt").is_symlink()

    (confdir / "b.txt").unlink()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (deployed / "a.txt").is_symlink()
    assert not (deployed / "b.txt").exists()

    # a user file replacing our symlink is never pruned
    (deployed / "a.txt").unlink()
    (deployed / "a.txt").write_text("user edit\n")
    (confdir / "a.txt").unlink()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (deployed / "a.txt").read_text() == "user edit\n"


def test_apply_repo_from_elsewhere(sandbox):
    """The bootstrap story: apply a repo that isn't the cwd."""
    payload = make_tarball(
        sandbox / "myenv" / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ artifactFile, file, fileFetch, identity, pkg, profile, provider, symlinkTo }} from "@gripsack/core";

export default [pkg("hello-payload", {{ producer: provider(fileFetch("{payload.name}")), commands: {{}}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
profile("hello", {{ files: [
    file({{ source: artifactFile("hello-payload", "bin/hello"), content: identity(), destination: symlinkTo("~/.local/bin/hello") }})
  ] }})];
""",
    )
    elsewhere = sandbox / "elsewhere"
    elsewhere.mkdir()
    out = grip("apply", "--host", "testhost", "--repo", str(repo), cwd=elsewhere)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/hello").is_symlink()

    # git URL form (a local path is a valid clone source)
    git_env = {
        "GIT_AUTHOR_NAME": "t",
        "GIT_AUTHOR_EMAIL": "t@t",
        "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@t",
        "PATH": os.environ["PATH"],
    }
    subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True, env=git_env)
    subprocess.run(["git", "add", "-A"], cwd=repo, check=True, env=git_env)
    subprocess.run(["git", "commit", "--quiet", "-m", "init"], cwd=repo, check=True, env=git_env)
    bare = sandbox / "myenv-remote"
    subprocess.run(["git", "clone", "--quiet", str(repo), str(bare)], check=True, env=git_env)
    out = grip("apply", "--host", "testhost", "--repo", str(bare), cwd=elsewhere)
    assert out.returncode == 0, out.stderr


def test_update_rewrites_lockfile_then_apply_deploys(sandbox):
    """Explicit update pins an external source; frozen apply consumes that pin."""
    payload = sandbox / "hello.tar.gz"
    make_tarball(payload, {"bin/hello": b"#!/bin/sh\necho v1\n"})
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ artifactFile, file, fileFetch, identity, pkg, profile, provider, symlinkTo }} from "@gripsack/core";

export default [pkg("hello-payload", {{ producer: provider(fileFetch("{payload}")), commands: {{}}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
profile("hello", {{ files: [
    file({{ source: artifactFile("hello-payload", "bin/hello"), content: identity(), destination: symlinkTo("~/.local/bin/hello") }})
  ] }})];
""",
    )
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    lock = repo / "gripsack.lock"
    first_pin = lock.read_text()

    # no movement -> unchanged
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/hello").read_bytes() == b"#!/bin/sh\necho v1\n"
    assert lock.read_text() == first_pin

    # payload changes -> update bumps the pin, apply deploys it
    make_tarball(payload, {"bin/hello": b"#!/bin/sh\necho v2\n"})
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert lock.read_text() != first_pin

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/hello").read_bytes() == b"#!/bin/sh\necho v2\n"


def test_config_tree_gain_deploys_from_store(sandbox):
    """A captured tree gain deploys from immutable store content, with either
    a warm store or a cold store under an unchanged provider pin."""
    payload = make_tarball(sandbox / "myenv" / "tool.tar.gz", {"bin/tool": b"#!/bin/sh\n"})
    confdir = sandbox / "myenv" / "configs" / "tool"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ artifactFile, file, fileFetch, identity, pkg, profile, provider, repoTree, symlinkTo }} from "@gripsack/core";

export default [pkg("tool-payload", {{ producer: provider(fileFetch("{payload.name}")), commands: {{}}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
profile("tool", {{ files: [
    file({{ source: artifactFile("tool-payload", "bin/tool"), content: identity(), destination: symlinkTo("~/.local/bin/tool") }}),
    file({{ source: repoTree("configs/tool"), content: identity(), destination: symlinkTo("~/.config/tool") }})
  ] }})];
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    store = sandbox / ".local/share/gripsack/store"
    deployed_a = sandbox / ".config" / "tool" / "a.conf"
    assert deployed_a.is_symlink()
    assert str(deployed_a.readlink()).startswith(str(store))

    # the tree gains a file while the package transport stays unchanged
    (confdir / "b.conf").write_text("b\n")
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed_b = sandbox / ".config" / "tool" / "b.conf"
    assert deployed_b.is_symlink()
    target = str(deployed_b.readlink())
    assert target.startswith(str(store)), f"new file deployed from the repo checkout: {target}"

    # Cold store: retained history must not pretend missing evidence is known.
    # The live apply may instead realize the unchanged, already-approved source.
    lock = (repo / "gripsack.lock").read_bytes()
    old_targets = {path: path.readlink() for path in [deployed_a, deployed_b]}
    shutil.rmtree(store)
    missing_snapshot = sandbox / "missing.snapshot.json"
    out = grip(
        "snapshot", "current", "--out", str(missing_snapshot), cwd=repo, approve=False
    )
    assert out.returncode != 0, out.stdout + out.stderr
    assert not missing_snapshot.exists()
    assert not store.exists()
    out = grip("apply", "--host", "testhost", cwd=repo, approve=False)
    assert out.returncode == 0, out.stderr
    assert (repo / "gripsack.lock").read_bytes() == lock
    for path, content in [(deployed_a, "a\n"), (deployed_b, "b\n")]:
        assert path.is_symlink()
        assert path.readlink() == old_targets[path]
        assert path.read_text() == content
    assert (sandbox / ".local/bin/tool").read_bytes() == b"#!/bin/sh\n"
    # Freshly retained evidence is readable without evaluation, and keeps its
    # privacy boundary rather than repopulating historical declaration gaps.
    snapshot = sandbox / "recreated.snapshot.json"
    out = grip("snapshot", "current", "--out", str(snapshot), cwd=repo, approve=False)
    assert out.returncode == 0, out.stderr
    document = json.loads(snapshot.read_text())
    assert "historical_declaration" not in document["completeness"]["unknown_reasons"]
    assert snapshot.stat().st_mode & 0o777 == 0o600

    # An existing artifact with a missing record is not a cold-store cache miss.
    # Live apply must not overwrite corruption or advance the selected generation.
    retained_roots = list(store.glob("*-semantic-review"))
    assert len(retained_roots) == 1
    record = retained_roots[0] / "snapshot.json"
    record.unlink()
    current = store.parent / "current"
    selected = current.readlink()
    out = grip("apply", "--host", "testhost", cwd=repo, approve=False)
    assert out.returncode != 0, out.stdout + out.stderr
    assert current.readlink() == selected
    assert not record.exists()
    assert deployed_a.read_text() == "a\n"
    assert deployed_b.read_text() == "b\n"
