"""Store hygiene e2e: gc, store-verify (incl. repair and deploy-output
hashes), why-owns, read-only payloads — split from test_flow.py;
fixture repos come from conftest."""



import json
import os
import stat
from pathlib import Path

from conftest import (
    native_target,
    grip,
    make_workspace_repo,
    make_tarball,
    remove_output,
)



def test_gc_collects_unreferenced_store_paths_and_why_owns(sandbox):
    payload = make_tarball(
        sandbox / "myenv" / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho v1\n"}
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
    out = grip("why-owns", "~/.local/bin/hello", cwd=repo)
    assert out.returncode == 0
    assert "hello" in out.stdout
    store = sandbox / ".local/share/gripsack/store"
    before = {p.name for p in store.iterdir()}
    manifest = json.loads((sandbox / ".local/share/gripsack/current/manifest.json").read_text())
    retained = manifest["modules"]["hello"]
    roots = {Path(retained["store_path"]).name}
    roots.update(Path(path).name for path in retained["build_closure"])
    assert before == roots
    assert len(roots) >= 2, "the profile and its native package are both retained"

    # drop the module entirely; deployment pruned on the next apply
    make_workspace_repo(repo, {"keep": 'import { profile } from "@gripsack/core"; export default profile("keep", {});'})
    remove_output(repo, "hello")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not (sandbox / ".local/bin/hello").exists()

    # gen 1 still references the store path — gc alone must keep it
    # (rollback!). keep_generations = 1 is what releases it.
    out = grip("gc", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert before <= {p.name for p in store.iterdir()}

    user_conf = sandbox / ".config/gripsack"
    user_conf.mkdir(parents=True)
    (user_conf / "config.toml").write_text("[settings]\nkeep_generations = 1\n")
    out = grip("gc", cwd=repo)
    assert out.returncode == 0, out.stderr
    after = {p.name for p in store.iterdir()} if store.exists() else set()
    assert not (after & roots), f"gc retained retired payloads: {after & roots}"
    current_manifest = json.loads((sandbox / ".local/share/gripsack/current/manifest.json").read_text())
    current_roots = {Path(module["store_path"]).name for module in current_manifest["modules"].values()}
    assert current_roots <= after, "gc must retain the explicit empty profile"
    assert not (sandbox / ".local/share/gripsack/generations/1").exists()
    assert (sandbox / ".local/share/gripsack/generations/2").exists()
    out = grip("why-owns", "~/.local/bin/hello", cwd=repo)
    assert out.returncode != 0


def test_gc_survives_tip_removal_without_id_reuse(sandbox):
    """0027 §9: generation IDs are never reused — rollback 3→1, gc
    with keep=1 removes 2 and 3 (the on-disk maximum moves BACKWARD),
    and the next apply still allocates 4, from the durable high-water
    mark."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("one\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("demo", { files: [
    file({ source: repoFile("configs/demo/a.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.toml") })
  ] });
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    for content in ("two\n", "three\n"):
        (confdir / "a.toml").write_text(content)
        out = grip("apply", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
    home = sandbox / ".local/share/gripsack"
    assert (home / "generations/3").is_dir()

    out = grip("rollback", "1", cwd=repo)
    assert out.returncode == 0, out.stderr
    user_conf = sandbox / ".config/gripsack"
    user_conf.mkdir(parents=True)
    (user_conf / "config.toml").write_text("[settings]\nkeep_generations = 0\n")
    out = grip("gc", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not (home / "generations/2").exists(), "gc should remove 2"
    assert not (home / "generations/3").exists(), "gc should remove 3 (not current)"

    (confdir / "a.toml").write_text("four\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (home / "generations/4").is_dir(), (
        f"the high-water mark must hold across gc: {sorted((home / 'generations').iterdir())}"
    )


def test_gc_aborts_when_generations_are_unreadable(sandbox):
    """0027 §2: an enumeration failure must never read as 'no
    generations' — gc would otherwise collect the active generation's
    store objects. Here generations/ is a FILE: read_dir fails, gc
    aborts, nothing is deleted."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("a\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("demo", { files: [
    file({ source: repoFile("configs/demo/a.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.toml") })
  ] });
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    home = sandbox / ".local/share/gripsack"
    store_before = {p.name for p in (home / "store").iterdir()}
    assert store_before

    gens = home / "generations"
    for child in gens.iterdir():
        if child.is_dir():
            import shutil

            shutil.rmtree(child)
        else:
            child.unlink()
    gens.rmdir()
    gens.write_text("corrupted into a file\n")

    out = grip("gc", cwd=repo)
    assert out.returncode != 0, "gc must fail closed"
    assert store_before == {p.name for p in (home / "store").iterdir()}, (
        "gc must delete NOTHING when the inventory is unreadable"
    )

def test_gc_dry_run_previews_without_deleting(sandbox):
    payload = make_tarball(
        sandbox / "myenv" / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho v1\n"}
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
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    make_workspace_repo(repo, {"keep": 'import { profile } from "@gripsack/core"; export default profile("keep", {});'})
    remove_output(repo, "hello")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    user_conf = sandbox / ".config/gripsack"
    user_conf.mkdir(parents=True)
    (user_conf / "config.toml").write_text("[settings]\nkeep_generations = 1\n")
    store = sandbox / ".local/share/gripsack/store"
    before = set(store.iterdir())
    out = grip("gc", "--dry-run", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "dry run" in out.stdout
    assert "collected" in out.stdout or "pruned" in out.stdout
    assert set(store.iterdir()) == before  # nothing deleted
    assert (sandbox / ".local/share/gripsack/generations/1").exists()


def test_store_verify_detects_and_repairs_corruption(sandbox):
    """0008 §3 shipped: the re-hash walk catches a tampered payload and
    --repair removes it; the next apply re-fetches (publish_dir's
    refusal becomes a republish)."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoTree, symlinkTo } from "@gripsack/core";

export default profile("demo", { files: [
    file({ source: repoTree("configs/demo"), content: identity(), destination: symlinkTo("~/.config/demo") })
  ] });
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stderr

    # tamper with a store path — verify must catch it, repair must
    # remove it, and the next apply re-fetches
    store = sandbox / ".local/share/gripsack/store"
    target = next(store.iterdir())
    for f in target.rglob("*"):
        if f.is_file() and not f.is_symlink():
            f.chmod(0o644)  # store payloads are read-only (0016 §D3)
            f.write_text("tampered\n")
    out = grip("store-verify", cwd=repo)
    assert out.returncode != 0
    assert "corrupt" in out.stdout
    out = grip("store-verify", "--repair", cwd=repo)
    assert "removed corrupt" in out.stdout
    assert not target.exists()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert target.exists()


def test_store_verify_covers_native_package_payloads(sandbox):
    """0014 §1a: verify used to compare the store TREE hash against the
    lock's TRANSPORT hash (never matches) under a hostname-keyed lock
    lookup (usually skipped). The manifest's tree256 is the expectation
    now — a tampered fetched payload fails verify, hostname-free."""
    payload = make_tarball(sandbox / "myenv" / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
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
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr

    store = sandbox / ".local/share/gripsack/store"
    target = next(path for path in store.iterdir() if (path / "bin/hello").is_file())
    for f in target.rglob("*"):
        if f.is_file() and not f.is_symlink():
            f.chmod(0o644)  # store payloads are read-only (0016 §D3)
            f.write_text("tampered\n")
    out = grip("store-verify", cwd=repo)
    assert out.returncode != 0
    assert "corrupt" in out.stdout
    out = grip("store-verify", "--repair", cwd=repo)
    assert "removed corrupt" in out.stdout
    # repair removed it; the next apply republishes the SAME path —
    # the lock's tree256 names the content, and the bytes haven't moved
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert target.is_dir()
    assert (target / "bin/hello").read_bytes() == b"#!/bin/sh\necho hello\n"


def test_store_payloads_are_read_only(sandbox):
    """0016 §D3: payload files land a-w at publish — an app writing
    through an owned symlink gets EACCES, the store stays verifiable,
    and repair/gc (which unlink via writable parent dirs) still work."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoTree, symlinkTo } from "@gripsack/core";

export default profile("demo", { files: [
    file({ source: repoTree("configs/demo"), content: identity(), destination: symlinkTo("~/.config/demo") })
  ] });
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    store = sandbox / ".local/share/gripsack/store"
    payload = (sandbox / ".config/demo/a.txt").resolve()
    assert payload.exists()
    # mode bits are the assertion that works everywhere (e2e's docker
    # stage runs as root, and root's CAP_DAC_OVERRIDE bypasses them)
    assert stat.S_IMODE(payload.stat().st_mode) & 0o222 == 0, "payload file must be read-only"

    # the app-write-through-the-symlink path: EACCES, not corruption
    if os.geteuid() != 0:
        deployed = sandbox / ".config/demo/a.txt"
        try:
            deployed.write_text("corrupt\n")
            raise AssertionError("write through an owned symlink must fail")
        except PermissionError:
            pass

    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr

    # repair still collects (unlink needs a writable parent, not file)
    for f in next(store.iterdir()).rglob("*"):
        if f.is_file() and not f.is_symlink():
            os.chmod(f, 0o644)
            f.chmod(0o644)  # store payloads are read-only (0016 §D3)
            f.write_text("tampered\n")
    out = grip("store-verify", "--repair", cwd=repo)
    assert "removed corrupt" in out.stdout


def test_store_verify_merge_and_template_hashes(sandbox):
    """Merge and template entries record DEPLOY-OUTPUT hashes (trimmed
    block, rendered bytes) — verify must recompute the same value, not
    compare the raw store file (a raw hash can never match)."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "block.sh").write_text("export FROB=1\n")
    (confdir / "id.toml").write_text('name = "{{ name }}"\n')
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, managedBlock, profile, repoFile, templateFile, trackedCopyTo } from "@gripsack/core";

export default profile("demo", { files: [
    file({ source: repoFile("configs/demo/block.sh"), content: identity(), destination: managedBlock("~/.bashrc", "demo") }),
    file({ source: repoFile("configs/demo/id.toml"), content: templateFile({ name: "t" }), destination: trackedCopyTo("~/.config/demo/id.toml") })
  ] });
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr
