"""Store hygiene e2e: gc, store-verify (incl. repair and deploy-output
hashes), why-owns, read-only payloads — split from test_flow.py;
fixture repos come from conftest."""



import os
import stat

from conftest import (
    grip,
    make_env_repo,
    make_tarball,
    only_store_path,
    remove_module,
)



def test_gc_collects_unreferenced_store_paths_and_why_owns(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho v1\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("why-owns", "~/.local/bin/hello", cwd=repo)
    assert out.returncode == 0
    assert "hello" in out.stdout
    store = sandbox / ".local/share/gripsack/store"
    before = {p.name for p in store.iterdir()}
    assert len(before) == 1

    # drop the module entirely; deployment pruned on the next apply
    remove_module(repo, "hello")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not (sandbox / ".local/bin/hello").exists()

    # gen 1 still references the store path — gc alone must keep it
    # (rollback!). keep_generations = 1 is what releases it.
    out = grip("gc", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert set({p.name for p in store.iterdir()}) == before

    user_conf = sandbox / ".config/gripsack"
    user_conf.mkdir(parents=True)
    (user_conf / "config.toml").write_text("[settings]\nkeep_generations = 1\n")
    out = grip("gc", cwd=repo)
    assert out.returncode == 0, out.stderr
    after = set(store.iterdir()) if store.exists() else set()
    assert not after, f"gc left {after}"
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
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
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
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
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
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho v1\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    remove_module(repo, "hello")
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
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("demo", {
  config: { ...tree("configs/demo", "~/.config/demo", "owned") },
});
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


def test_store_verify_covers_fetched_modules(sandbox):
    """0014 §1a: verify used to compare the store TREE hash against the
    lock's TRANSPORT hash (never matches) under a hostname-keyed lock
    lookup (usually skipped). The manifest's tree256 is the expectation
    now — a tampered fetched payload fails verify, hostname-free."""
    payload = make_tarball(sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr

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
    # repair removed it; the next apply republishes the SAME path —
    # the lock's tree256 names the content, and the bytes haven't moved
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert only_store_path(sandbox) == target.name


def test_store_payloads_are_read_only(sandbox):
    """0016 §D3: payload files land a-w at publish — an app writing
    through an owned symlink gets EACCES, the store stays verifiable,
    and repair/gc (which unlink via writable parent dirs) still work."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("demo", {
  config: { ...tree("configs/demo", "~/.config/demo", "owned") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    store = sandbox / ".local/share/gripsack/store"
    payload = next(store.iterdir()) / "configs/demo/a.txt"
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
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { merge, module, template } from "@gripsack/core";

export default module("demo", {
  config: {
    "configs/demo/block.sh": merge("~/.bashrc", "#"),
    "configs/demo/id.toml": template("~/.config/demo/id.toml", { name: "t" }),
  },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr
