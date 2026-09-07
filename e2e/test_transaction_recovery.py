"""Apply and rollback recovery across interrupted transactions."""

from conftest import grip, make_env_repo, make_tarball, refresh_host


def test_torn_run_marker_fails_closed(sandbox):
    """A run marker missing `previous_generation` is torn or corrupt
    (the field is required on the wire) — recovery fails closed and
    retains the journal, never guessing a commit state."""
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
    journal = home / "journal"
    journal.mkdir(exist_ok=True)
    (journal / "run.json").write_text('{"target_generation": 2, "op": "apply"}')

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, "a torn marker must block"
    assert (journal / "run.json").exists(), "the journal is retained"


def test_failed_apply_rolls_back_this_runs_deployments(sandbox):
    """0001 §9 / review finding E1: a mid-graph failure must leave no
    half-applied deployment — the flip never happens, and every
    destination the failed run touched returns to the previous
    generation's state."""
    confdir = sandbox / "myenv" / "configs" / "aaa"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("v1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("aaa", {
  config: { "configs/aaa/a.conf": trackedCopy("~/.out/a.conf") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".out/a.conf").read_text() == "v1\n"

    # v2 of aaa + a module that fails at deploy (payload lacks the
    # entry — a deploy-time failure E110 can't catch)
    (confdir / "a.conf").write_text("v2\n")
    payload = make_tarball(sandbox / "b.tar.gz", {"bin/b": b"#!/bin/sh\n"})
    (repo / "modules" / "bbb.ts").write_text(
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("bbb", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/MISSING": symlink("~/.out/b") }},
}});
"""
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    # the flip never happened…
    generations = sandbox / ".local/share/gripsack/generations"
    # generations/ also holds the durable high-water mark (0027 §9) —
    # filter to actual generation directories
    assert [p.name for p in generations.iterdir() if p.is_dir()] == ["1"]
    # …and this run's deployments are rolled back exactly
    assert (sandbox / ".out/a.conf").read_text() == "v1\n"
    assert not (sandbox / ".out/b").exists()


def test_kill_between_prune_and_flip_recovers(sandbox, monkeypatch):
    """A kill -9 between prune and the flip leaves pruned destinations
    removed under the OLD current generation — before 0025 §B that
    mutation was unjournaled and unrecoverable. The next apply's
    reconcile restores the prior, then re-prunes cleanly."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("a\n")
    (confdir / "b.toml").write_text("b\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: {
    "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml"),
    "configs/demo/b.toml": trackedCopy("~/.config/demo/b.toml"),
  },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest_b = sandbox / ".config/demo/b.toml"
    assert dest_b.read_text() == "b\n"

    # undeclare b, then die right after the prune, before the flip
    (repo / "modules" / "hello.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
"""
    )
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "after-prune")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, "the crash hook must kill the run"
    # the window: b pruned, generation 1 still current
    assert not dest_b.exists()
    current = sandbox / ".local/share/gripsack/current"
    assert current.resolve().name == "1"

    # the next apply reconciles: restores the pruned file (journal
    # prior), then completes the undeclare cleanly
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "recovered" in out.stdout or "recovered" in out.stderr
    assert current.resolve().name == "2"
    assert not dest_b.exists()


def test_kill_mid_rollback_recovers(sandbox, monkeypatch):
    """A kill -9 mid-rollback (restores done, flip pending) used to
    leave destinations at the TARGET generation's content under the
    ORIGINAL current with no record. Rollback is journaled now
    (0025 §A): the next apply's reconcile restores the pre-rollback
    state."""
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
    current = sandbox / ".local/share/gripsack/current"
    assert current.resolve().name == "2"

    # die after the rollback's restore, before its flip
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "after-rollback-restore")
    out = grip("rollback", cwd=repo)
    assert out.returncode != 0, "the crash hook must kill the run"
    # the window: dest restored to gen1's content, current still 2
    assert dest.read_text() == "gen1\n"
    assert current.resolve().name == "2"

    # the next apply reconciles the crashed rollback: the pre-rollback
    # (generation 2) content comes back
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == "gen2\n"
    assert current.resolve().name == "2"


def test_module_rename_rollback_journals_once(sandbox, monkeypatch):
    """0026 §2: the same destination under a renamed module gets ONE
    transition in a rollback — kill mid-rollback and reconcile must
    restore the TRUE pre-rollback state. The pre-0.23 two-pass
    rollback journaled the dest twice (prune pass, restore pass); the
    second entry's prior was the post-removal state, so recovery
    deleted a file the rollback started from."""
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
    # gen 2: new content AND a new module name for the same dest
    (confdir / "a.toml").write_text("two\n")
    (repo / "modules" / "hello.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("renamed", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
"""
    )
    # the rename makes the dest foreign to the new module — take it
    # over so gen 2 really deploys "two" (with "one" as its prior)
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".config/demo/a.toml"
    assert dest.read_text() == "two\n"

    # kill after the rollback's mutations, before its flip
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "after-rollback-restore")
    out = grip("rollback", "1", cwd=repo)
    assert out.returncode != 0
    current = sandbox / ".local/share/gripsack/current"
    assert current.resolve().name == "2"
    assert dest.read_text() == "one\n", "the restore landed before the kill"

    # reconcile must restore the TRUE pre-rollback content — the old
    # double-journaling recorded Absent as the prior and deleted the
    # file instead. The recovery is observable without touching the
    # repo: a clean reconcile restores "two" and the apply is
    # satisfied at generation 2; the old bug deleted the dest, so the
    # apply would redeploy and cut a spurious generation
    # (a new generation IS cut — the manifest's module name changed in
    # the rename; the point is the dest was never deleted/redeployed)
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == "two\n", dest.read_text()


def test_roll_forward_kill_recovers_as_uncommitted(sandbox, monkeypatch):
    """0026 §4: rolling FORWARD (rollback 1→2 after rolling back) with
    a kill before the flip — the 0.22 direction rule read current(1) <=
    target(2) as committed and discarded the journal. Exact-equality
    commit detection recovers it as uncommitted."""
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
    dest = sandbox / ".config/demo/a.toml"
    out = grip("rollback", "1", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == "one\n"

    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "after-rollback-restore")
    out = grip("rollback", "2", cwd=repo)
    assert out.returncode != 0
    current = sandbox / ".local/share/gripsack/current"
    assert current.resolve().name == "1"
    assert dest.read_text() == "two\n", "the restore landed before the kill"

    # revert the repo to gen 1's content so the recovery is
    # OBSERVABLE: reconcile restores "one", then the apply is
    # satisfied — a misclassified-committed journal (0.22) would have
    # left "two" standing, and apply keeps drifted destinations
    (confdir / "a.toml").write_text("one\n")
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == "one\n", (
        "an uncommitted roll-forward must restore the prior — 0.22 kept it"
    )


def test_failed_apply_rollback_leaves_no_placeholder_links(sandbox):
    """A mid-graph failure rolls this run's deploys back to the
    previous generation — restored links must be the EXPANDED paths
    the generation actually deployed, never placeholder-literal."""
    import sys
    os_dir = "darwin" if sys.platform == "darwin" else "linux"
    payload = make_tarball(sandbox / "a.tar.gz", {f"{os_dir}/a.txt": b"a\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("aa", {{
  fetch: fileFetch("{payload}"),
  install: {{ "{{os}}/a.txt": symlink("~/.local/bin/aa") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    link = sandbox / ".local/bin/aa"
    assert "{" not in str(link.readlink())

    # add a module whose fetch fails -> the apply aborts mid-graph and
    # rolls back aa's redeploy to the previous generation
    (repo / "modules" / "zz.ts").write_text(
        """
import { fileFetch, module, symlink } from "@gripsack/core";

export default module("zz", {
  fetch: fileFetch("%s/does-not-exist.tar.gz"),
  install: { "x": symlink("~/.local/bin/zz") },
});
"""
        % sandbox
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, out.stdout
    target = str(link.readlink())
    assert "{" not in target, f"rollback wrote a placeholder-literal link: {target}"
    assert link.exists(), f"rollback left a dangling link: {target}"


def canonical_sha(data: bytes, mode: int = 0o644) -> str:
    """The journal's mode-aware file identity (0031): type tag +
    mode marker + LE mode + contents — what deploy records as the
    intended post-mutation identity."""
    import hashlib

    return hashlib.sha256(
        b"file\0" + b"\x01" + mode.to_bytes(4, "little") + data
    ).hexdigest()


def test_apply_recovers_from_an_interrupted_run(sandbox):
    """Crash recovery (0019): a run killed between a deploy mutation
    and the flip leaves an uncommitted journal entry — the next apply
    restores the prior before redeploying, reports it, drains the
    journal at the flip, and a user edit made after the crash wins
    (the drift guard). The crashed state is crafted exactly as a kill
    between record/mutate and commit_run would leave it."""
    import hashlib
    import json

    payload = make_tarball(
        sandbox / "a.tar.gz", {"conf.txt": b"v=new\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, trackedCopy }} from "@gripsack/core";

export default module("a", {{
  fetch: fileFetch("{payload}"),
  config: {{ "conf.txt": trackedCopy("~/.config/a/conf.txt") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    conf = sandbox / ".config/a/conf.txt"
    assert conf.read_text() == "v=new\n"

    home = sandbox / ".local/share/gripsack"
    journal = home / "journal"
    journal.mkdir(parents=True, exist_ok=True)
    prior_bytes = b"v=new\n"
    prior_sha = hashlib.sha256(prior_bytes).hexdigest()
    (home / "prior").mkdir(exist_ok=True)
    (home / "prior" / prior_sha).write_bytes(prior_bytes)
    half = b"v=newer (half-deployed)\n"
    conf.write_bytes(half)
    dest = str(conf)
    entry = {
        "dest": dest,
        "prior": {"kind": "file", "hash": prior_sha, "mode": 420},
        "after": canonical_sha(half),
    }
    (journal / (hashlib.sha256(dest.encode()).hexdigest() + ".json")).write_text(
        json.dumps(entry)
    )

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "recovered 1 destination(s)" in out.stdout, out.stdout
    # the prior was restored, then the (unchanged) module redeployed it
    assert conf.read_text() == "v=new\n"
    assert not list(journal.glob("*.json")), "journal must drain at the flip"

    # an apply is satisfied afterwards — no half-state lingers
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert "satisfied" in out.stdout, out.stdout


def test_recovery_leaves_user_edits_alone(sandbox):
    """The same interrupted-run entry, but the user edited the file
    after the crash: the drift guard keeps their bytes."""
    import hashlib
    import json

    payload = make_tarball(sandbox / "a.tar.gz", {"conf.txt": b"v=new\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, trackedCopy }} from "@gripsack/core";

export default module("a", {{
  fetch: fileFetch("{payload}"),
  config: {{ "conf.txt": trackedCopy("~/.config/a/conf.txt") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    conf = sandbox / ".config/a/conf.txt"

    home = sandbox / ".local/share/gripsack"
    journal = home / "journal"
    journal.mkdir(parents=True, exist_ok=True)
    prior_sha = hashlib.sha256(b"v=new\n").hexdigest()
    (home / "prior").mkdir(exist_ok=True)
    (home / "prior" / prior_sha).write_bytes(b"v=new\n")
    dest = str(conf)
    entry = {
        "dest": dest,
        "prior": {"kind": "file", "hash": prior_sha, "mode": 420},
        "after": canonical_sha(b"half\n"),
    }
    (journal / (hashlib.sha256(dest.encode()).hexdigest() + ".json")).write_text(
        json.dumps(entry)
    )
    # the user's edit AFTER the crash — not the half-deployed content
    conf.write_text("my own edit\n")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "kept" in out.stdout, out.stdout
    assert conf.read_text() == "my own edit\n"
