"""Generation history, rollback, allocation and concurrent commits."""

import subprocess
from conftest import GRIP, grip, make_env_repo, make_tarball, refresh_host


def test_current_link_must_resolve_under_home(sandbox):
    """0030 §H10: `current -> /tmp/42` is corruption, not a
    generation — apply fails closed."""
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

    current = sandbox / ".local/share/gripsack/current"
    current.unlink()
    current.symlink_to("/tmp/42")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, "an outside-home current must block"


def test_concurrent_applies_serialize_and_lose_nothing(sandbox):
    """Finding A: two applies over disjoint subsets must not lose a
    manifest update — the lifecycle holds apply.flock."""
    repo = sandbox / "myenv"
    for name in ("amod", "bmod"):
        confdir = repo / "configs" / name
        confdir.mkdir(parents=True)
        (confdir / f"{name}.conf").write_text(f"{name}\n")
    make_env_repo(
        repo,
        {
            name: f"""
import {{ module, trackedCopy }} from "@gripsack/core";

export default module("{name}", {{
  config: {{ "configs/{name}/{name}.conf": trackedCopy("~/.out/{name}.conf") }},
}});
"""
            for name in ("amod", "bmod")
        },
    )
    grip_bin = str(GRIP.resolve())
    p1 = subprocess.Popen(
        [grip_bin, "apply", "--host", "testhost", "amod"],
        cwd=repo,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    p2 = subprocess.Popen(
        [grip_bin, "apply", "--host", "testhost", "bmod"],
        cwd=repo,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    o1, e1 = p1.communicate(timeout=60)
    o2, e2 = p2.communicate(timeout=60)
    assert p1.returncode == 0, e1
    assert p2.returncode == 0, e2
    out = grip("why-owns", "~/.out/amod.conf", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "amod" in out.stdout
    out = grip("why-owns", "~/.out/bmod.conf", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "bmod" in out.stdout


def test_rollback_restores_previous_generation(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
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
    first = grip("apply", "--host", "testhost", cwd=repo)
    assert first.returncode == 0, first.stderr

    # a no-op apply creates no generation (0008 §3)
    second = grip("apply", "--host", "testhost", cwd=repo)
    assert second.returncode == 0, second.stderr
    assert "already satisfied" in second.stdout
    assert not (sandbox / ".local/share/gripsack/generations/2").exists()

    # a changed module produces generation 2; rollback restores 1
    (repo / "modules" / "extra.ts").write_text(
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("extra", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/extra") }},
}});
"""
    )
    refresh_host(repo)

    third = grip("apply", "--host", "testhost", cwd=repo)
    assert third.returncode == 0, third.stderr
    assert (sandbox / ".local/bin/extra").is_symlink()

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    current = sandbox / ".local/share/gripsack/current"
    assert current.resolve().name == "1"
    # the extra module's destination is gone after rollback
    assert not (sandbox / ".local/bin/extra").exists()


def test_rollback_preserves_tracked_copy_drift(sandbox):
    """0026 §1: a tracked copy edited since the current generation was
    deployed is DRIFT — rollback must preserve and report it, never
    overwrite it with the target generation's bytes."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("gen1\n")
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
    (confdir / "a.toml").write_text("gen2\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".config/demo/a.toml"
    assert dest.read_text() == "gen2\n"

    # the app writes to its own config — drift from gen 2's deployment
    dest.write_text("user edit\n")
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "user edit" in dest.read_text(), "rollback must not clobber drift"
    assert "your edit stands" in out.stdout
    current = sandbox / ".local/share/gripsack/current"
    assert current.resolve().name == "1", "the flip still happens — only the drifted file is kept"


def test_generation_numbers_are_never_reused(sandbox):
    """0026 §3: after rollback 3→1, the next apply allocates generation
    4 — generation 2 on disk stays byte-identical (immutable history)."""
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
    grip("apply", "--host", "testhost", cwd=repo)
    (confdir / "a.toml").write_text("two\n")
    grip("apply", "--host", "testhost", cwd=repo)
    (confdir / "a.toml").write_text("three\n")
    grip("apply", "--host", "testhost", cwd=repo)
    home = sandbox / ".local/share/gripsack"
    gen2_manifest = (home / "generations/2/manifest.json").read_text()

    out = grip("rollback", "1", cwd=repo)
    assert out.returncode == 0, out.stderr

    (confdir / "a.toml").write_text("four\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (home / "generations/4").is_dir(), "the new generation is 4, not a reused 2"
    assert (home / "generations/2/manifest.json").read_text() == gen2_manifest, (
        "generation 2 must not be rewritten"
    )


def test_rollback_never_rewrites_historical_generations(sandbox):
    """0027 §8: a generation is one immutable object. Rolling back to
    it must not change a single byte inside it — the profile backfills
    only when MISSING (pre-0.22 history), never re-renders over it."""
    import hashlib

    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("one\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
  env: { EDITOR: "demo" },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    (confdir / "a.toml").write_text("two\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    def tree_hash(d):
        h = hashlib.sha256()
        for f in sorted(d.rglob("*")):
            if f.is_file():
                h.update(f.name.encode())
                h.update(f.read_bytes())
        return h.hexdigest()

    home = sandbox / ".local/share/gripsack"
    before = tree_hash(home / "generations/1")
    assert (home / "generations/1/env/profile.sh").exists()

    out = grip("rollback", "1", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert tree_hash(home / "generations/1") == before, (
        "rollback must not touch a historical generation"
    )


def test_a_repaired_destination_cuts_a_generation(sandbox):
    """Satisfied means nothing changed on disk. An owned link that
    drifted (or a stale pre-store link replaced) is real filesystem
    work — the run must cut a generation so rollback can undo it,
    not summarize 'already satisfied' over a modified machine."""
    payload = make_tarball(
        sandbox / "tool.tar.gz", {"bin/tool": b"#!/bin/sh\necho tool\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("m", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/tool": symlink("~/.local/bin/tool") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".local/bin/tool"
    assert dest.is_symlink()

    # the link drifts to a foreign target
    dest.unlink()
    dest.symlink_to("/usr/bin/false")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "generation 2" in out.stdout, out.stdout
    assert "store" in str(dest.resolve())
    # and it is undoable: rollback returns to generation 1's link
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
