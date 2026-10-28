"""Apply and rollback recovery across interrupted transactions."""

from conftest import native_target, grip, make_workspace_repo, make_tarball, refresh_workspace


def test_torn_run_marker_fails_closed(sandbox):
    """Missing previous state must not gain explicit-null recovery authority."""
    import json
    from pathlib import Path

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
    home = sandbox / ".local/share/gripsack"
    journal = home / "journal"
    journal.mkdir(parents=True)
    destination = sandbox / "interrupted-link"
    destination.symlink_to("installed")
    entry = {
        "v": 2, "dest": str(destination),
        "prior": {"kind": "absent"}, "before": {"kind": "absent"},
        "after": {"kind": "link", "target": "installed"},
    }
    (journal / "intent.json").write_text(json.dumps(entry))
    malformed = b'{"target_generation": 1, "op": "apply"}'
    marker = journal / "run.json"
    marker.write_bytes(malformed)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, out.stdout + out.stderr
    assert destination.readlink() == Path("installed")
    assert not (sandbox / ".config/demo/a.toml").exists()
    assert marker.read_bytes() == malformed
    assert json.loads((journal / "intent.json").read_text()) == entry

    marker.write_text(json.dumps({
        "previous_generation": None, "target_generation": 1, "op": "apply",
    }))
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr
    assert not destination.is_symlink() and not destination.exists()
    assert (sandbox / ".config/demo/a.toml").read_text() == "a\n"


def test_failed_apply_rolls_back_this_runs_deployments(sandbox):
    """A deployed pre-flip check observes both writes, then fails: every
    destination is compensated and the previous generation stays selected."""
    confdir = sandbox / "myenv" / "configs" / "aaa"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("v1\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("aaa", { files: [
    file({ source: repoFile("configs/aaa/a.conf"), content: identity(), destination: trackedCopyTo("~/.out/a.conf") })
  ] });
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".out/a.conf").read_text() == "v1\n"
    current = sandbox / ".local/share/gripsack/current"
    previous_selection = current.readlink()

    # Fail after both destinations have actually changed, not during preparation.
    (confdir / "a.conf").write_text("v2\n")
    marker = sandbox / "check-observed-both-writes"
    (repo / "outputs" / "bbb.ts").write_text(
        f"""
import {{ check, file, literalText, profile, exec, lit, trackedCopyTo }} from "@gripsack/core";

export default [
  check("reject-candidate", {{ subject: "bbb", run: exec({{
    argv: [lit("/bin/sh"), lit("-c"), lit('test "$(cat "{sandbox}/.out/a.conf")" = v2 && test "$(cat "{sandbox}/.out/b")" = candidate && touch "{marker}"; exit 1')]
  }}) }}),
  profile("bbb", {{ files: [file({{
    content: literalText("candidate"),
    destination: trackedCopyTo("~/.out/b"),
    checks: [{{ check: "reject-candidate", subject: "deployed", stage: "pre_flip" }}],
  }})] }}),
];
"""
    )
    refresh_workspace(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert marker.exists(), "the failing check observed both new destination contents"
    assert current.readlink() == previous_selection, "failed apply must not select another transaction"
    # This run's deployments are rolled back exactly.
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
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("demo", { files: [
    file({ source: repoFile("configs/demo/a.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.toml") }),
    file({ source: repoFile("configs/demo/b.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/b.toml") })
  ] });
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest_b = sandbox / ".config/demo/b.toml"
    assert dest_b.read_text() == "b\n"

    # undeclare b, then die right after the prune, before the flip
    (repo / "outputs" / "hello.ts").write_text(
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("demo", { files: [
    file({ source: repoFile("configs/demo/a.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.toml") })
  ] });
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
    # gen 2: new content AND a new module name for the same dest
    (confdir / "a.toml").write_text("two\n")
    (repo / "outputs" / "hello.ts").write_text(
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("renamed", { files: [
    file({ source: repoFile("configs/demo/a.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.toml") })
  ] });
"""
    )
    # the rename makes the dest foreign to the new module — take it
    # over so gen 2 really deploys "two" (with "one" as its prior)
    refresh_workspace(repo)
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
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("demo", { files: [
    file({ source: repoFile("configs/demo/a.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.toml") })
  ] });
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


def test_failed_apply_restores_previous_owned_link(sandbox):
    """A failed deployed check restores the exact prior immutable store link."""
    payload = make_tarball(sandbox / "myenv" / "a.tar.gz", {"a.txt": b"original\n"})
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ artifactFile, file, fileFetch, identity, pkg, profile, provider, symlinkTo }} from "@gripsack/core";

export default [pkg("aa-payload", {{ producer: provider(fileFetch("{payload.name}")), commands: {{}}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
profile("aa", {{ files: [
    file({{ source: artifactFile("aa-payload", "a.txt"), content: identity(), destination: symlinkTo("~/.local/bin/aa") }})
  ] }})];
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    link = sandbox / ".local/bin/aa"
    previous_target = link.readlink()
    current = sandbox / ".local/share/gripsack/current"
    previous_selection = current.readlink()
    updated = make_tarball(sandbox / "myenv" / "a-v2.tar.gz", {"a.txt": b"candidate\n"})
    declaration = repo / "outputs/hello.ts"
    declaration.write_text(declaration.read_text().replace(payload.name, updated.name))
    marker = sandbox / "check-observed-new-link"
    (repo / "outputs" / "zz.ts").write_text(
        f"""
import {{ check, file, literalText, profile, exec, lit, trackedCopyTo }} from "@gripsack/core";

export default [
  check("reject-link", {{ subject: "zz", run: exec({{
    argv: [lit("/bin/sh"), lit("-c"), lit('test "$(cat "{link}")" = candidate && touch "{marker}"; exit 1')]
  }}) }}),
  profile("zz", {{ files: [file({{
    content: literalText("candidate"),
    destination: trackedCopyTo("~/.local/bin/zz"),
    checks: [{{ check: "reject-link", subject: "deployed", stage: "pre_flip" }}],
  }})] }}),
];
"""
    )
    refresh_workspace(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, out.stdout
    assert marker.exists(), "the candidate link was deployed before the failure"
    assert link.readlink() == previous_target
    assert link.read_text() == "original\n"
    assert current.readlink() == previous_selection
    assert not (sandbox / ".local/bin/zz").exists()


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
        sandbox / "myenv" / "a.tar.gz", {"conf.txt": b"v=new\n"}
    )
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ artifactFile, file, fileFetch, identity, pkg, profile, provider, trackedCopyTo }} from "@gripsack/core";

export default [pkg("a-payload", {{ producer: provider(fileFetch("{payload.name}")), commands: {{}}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
profile("a", {{ files: [
    file({{ source: artifactFile("a-payload", "conf.txt"), content: identity(), destination: trackedCopyTo("~/.config/a/conf.txt") }})
  ] }})];
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
    # the tagged wire shape (0045 F1): versioned, variant-explicit
    entry = {
        "v": 1,
        "dest": dest,
        "prior": {"kind": "file", "hash": prior_sha, "mode": 420},
        "after": {"kind": "file", "identity": canonical_sha(half)},
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

    payload = make_tarball(sandbox / "myenv" / "a.tar.gz", {"conf.txt": b"v=new\n"})
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ artifactFile, file, fileFetch, identity, pkg, profile, provider, trackedCopyTo }} from "@gripsack/core";

export default [pkg("a-payload", {{ producer: provider(fileFetch("{payload.name}")), commands: {{}}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
profile("a", {{ files: [
    file({{ source: artifactFile("a-payload", "conf.txt"), content: identity(), destination: trackedCopyTo("~/.config/a/conf.txt") }})
  ] }})];
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
        "v": 1,
        "dest": dest,
        "prior": {"kind": "file", "hash": prior_sha, "mode": 420},
        "after": {"kind": "file", "identity": canonical_sha(b"half\n")},
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


def test_legacy_untagged_entry_fails_closed(sandbox):
    """0045 F1: a pre-0.40 journal entry (bare-string `after`, no wire
    version) is never reinterpreted — its variant cannot be proven
    from the string. Recovery blocks, the entry is quarantined with
    its evidence, and the destination is untouched."""
    import hashlib
    import json

    payload = make_tarball(sandbox / "myenv" / "a.tar.gz", {"conf.txt": b"v=new\n"})
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ artifactFile, file, fileFetch, identity, pkg, profile, provider, trackedCopyTo }} from "@gripsack/core";

export default [pkg("a-payload", {{ producer: provider(fileFetch("{payload.name}")), commands: {{}}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
profile("a", {{ files: [
    file({{ source: artifactFile("a-payload", "conf.txt"), content: identity(), destination: trackedCopyTo("~/.config/a/conf.txt") }})
  ] }})];
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
    # exactly what 0.39 wrote: no `v`, `after` a bare string
    legacy = {
        "dest": dest,
        "prior": {"kind": "file", "hash": prior_sha, "mode": 420},
        "after": canonical_sha(b"v=newer\n"),
    }
    rejected = journal / (hashlib.sha256(dest.encode()).hexdigest() + ".json")
    rejected.write_text(json.dumps(legacy))
    rejected.chmod(0o666)
    journal.chmod(0o755)

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, "legacy entries block, never guessed"
    assert "pre-0.40" in out.stderr, out.stderr
    assert len(list((journal / "quarantine").glob("*.json"))) == 1
    assert conf.read_text() == "v=new\n", "the destination is untouched"
    quarantine = journal / "quarantine"
    assert journal.stat().st_mode & 0o7777 == 0o700
    assert quarantine.stat().st_mode & 0o7777 == 0o700
    saved = quarantine / rejected.name
    assert saved.stat().st_mode & 0o7777 == 0o600
    assert json.loads(saved.read_text()) == legacy
