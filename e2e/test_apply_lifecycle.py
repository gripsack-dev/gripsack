"""Apply lifecycle e2e: generations, satisfied re-applies, subsets and
prune, concurrency, and rollback of failed runs — split from
test_flow.py (plan/0003 §5); fixture repos come from conftest."""



import os
import shutil
import subprocess

from conftest import (
    GRIP,
    grip,
    make_env_repo,
    make_tarball,
    refresh_host,
    remove_module,
)



def test_apply_creates_generation_and_symlinks(sandbox):
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
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    home = sandbox / ".local/share/gripsack"
    assert (home / "generations/1").is_dir()
    assert (home / "current").is_symlink()
    assert (sandbox / ".local/bin/hello").is_symlink()


def test_fetchless_build_outputs_reach_the_store(sandbox):
    """A module whose content is a build step (no fetch) must keep
    its outputs: publish used to hand-roll a fresh staging dir and
    wipe what the step had just produced — apply "succeeded" with an
    empty store path."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module } from "@gripsack/core";

export default module("built", {
  steps: [{
    id: "build",
    action: { kind: "custom_shell", script: "echo hello-artifact > artifact.txt" },
  }],
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    home = sandbox / ".local/share/gripsack"
    artifacts = list((home / "store").glob("*-built/artifact.txt"))
    assert artifacts, "build step output missing from the store path"
    assert artifacts[0].read_text().strip() == "hello-artifact"


def test_tree_entries_and_prune_on_undeclare(sandbox):
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "settings.json").write_text('{"theme": "mocha"}\n')
    (confdir / "keymap.json").write_text("[]\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("zed", {
  config: { ...tree("configs/zed", "~/.config/zed") },
});
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
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("zed", {
  config: { ...tree("configs/zed", "~/.config/zed", "owned") },
});
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


def test_explicit_steps_module_is_satisfied_on_reapply(sandbox):
    """Class/explicit-steps modules keep fetch specs in steps, not
    module.fetch — their store path must still be stable (canary-caught)."""
    payload = sandbox / "hello.tar.gz"
    make_tarball(payload, {"bin/x": b"#!/bin/sh\necho x\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fetchStep, module, shellStep, tarball }} from "@gripsack/core";

export default module("stepped", {{
  steps: [
    fetchStep(tarball("file://{payload}")),
    shellStep("true", "noop", {{ needs: ["fetch"] }}),
  ],
}});
""",
    )
    first = grip("apply", "--host", "testhost", cwd=repo)
    assert first.returncode == 0, first.stderr
    second = grip("apply", "--host", "testhost", cwd=repo)
    assert second.returncode == 0, second.stderr
    assert "already satisfied" in second.stdout


def test_update_rewrites_lockfile_then_apply_deploys(sandbox):
    """The flake cycle: update moves the lockfile, apply executes it."""
    payload = sandbox / "hello.tar.gz"
    make_tarball(payload, {"bin/hello": b"#!/bin/sh\necho v1\n"})
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
    lock = repo / "locks" / "testhost.lock"
    assert lock.exists()
    first_pin = lock.read_text()

    # no movement -> unchanged
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "unchanged" in out.stdout
    assert lock.read_text() == first_pin

    # payload changes -> update bumps the pin, apply deploys it
    make_tarball(payload, {"bin/hello": b"#!/bin/sh\necho v2\n"})
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "bumped" in out.stdout
    assert lock.read_text() != first_pin

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "applied" in out.stdout


def test_exported_env_profile_tracks_the_generation(sandbox):
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("zed", {
  config: { "configs/zed/a": trackedCopy("~/.config/zed/a") },
  env: { EDITOR: "zed", "PATH+": "{store}/bin" },
});
""",
    )
    (repo / "configs" / "zed").mkdir(parents=True)
    (repo / "configs" / "zed" / "a").write_text("a\n")
    # the profile is generation-local since 0.22, sourced through the
    # current symlink (plan/0025 §C): it activates with the flip
    profile = sandbox / ".local/share/gripsack/current/env/profile.sh"
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    text = profile.read_text()
    assert 'export EDITOR="zed"' in text
    store_bin = next(
        line for line in text.splitlines() if line.startswith("export PATH=")
    )
    assert "/bin:${PATH}" in store_bin
    assert "/store/" in store_bin and "-zed/bin:" in store_bin

    # drop the env declaration — the profile must not go stale
    (repo / "modules" / "hello.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("zed", {
  config: { "configs/zed/a": trackedCopy("~/.config/zed/a") },
});
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    # no env contributions in this generation → no profile in it; the
    # generation-1 file still exists behind the old directory, but
    # current/ no longer resolves one
    assert not profile.exists()


def test_service_intent_runs_the_adapter_without_failing_apply(sandbox):
    """systemd-user adapter: no systemctl/user bus in the sandbox —
    the intent must degrade to a warning, never a failed apply
    (0001 §3.8: never roll back on post-activation failure)."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, service, trackedCopy } from "@gripsack/core";

export default module("daemon", {
  config: { "configs/daemon/a": trackedCopy("~/.config/daemon/a") },
  activate: [service("my-daemon.service")],
});
""",
    )
    (repo / "configs" / "daemon").mkdir(parents=True)
    (repo / "configs" / "daemon" / "a").write_text("a\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "my-daemon.service" in out.stdout


def test_duplicate_destination_is_a_check_time_error(sandbox):
    """E111 (N2): two modules may not declare the same destination —
    a deploy race in parallel and a lie for why-owns."""
    confdir = sandbox / "myenv" / "configs" / "x"
    confdir.mkdir(parents=True)
    (confdir / "same.conf").write_text("x\n")
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "one": """
import { module, trackedCopy } from "@gripsack/core";

export default module("one", {
  config: { "configs/x/same.conf": trackedCopy("~/.out/same.conf") },
});
""",
            "two": """
import { module, trackedCopy } from "@gripsack/core";

export default module("two", {
  config: { "configs/x/same.conf": trackedCopy("~/.out/same.conf") },
});
""",
        },
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E111" in out.stderr
    assert "same.conf" in out.stderr


def test_destination_aliases_are_rejected_before_mutation(sandbox):
    """0030 §P0-1: `~/x` and `$HOME/x` are one physical destination —
    and a symlinked ancestor collapses to one too. Both must fail at
    check/apply time, never double-transition one object."""
    confdir = sandbox / "myenv" / "configs" / "x"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    home = sandbox
    alias = home / ".config/aliased/x.conf"
    alias.parent.mkdir(parents=True)

    repo = make_env_repo(
        sandbox / "myenv",
        {
            "one": """
import { module, trackedCopy } from "@gripsack/core";

export default module("one", {
  config: { "configs/x/a.conf": trackedCopy("~/.config/aliased/x.conf") },
});
""",
            "two": f"""
import {{ module, trackedCopy }} from "@gripsack/core";

export default module("two", {{
  config: {{ "configs/x/a.conf": trackedCopy("{alias}") }},
}});
""",
        },
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "resolve to the same path" in out.stderr, out.stderr
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "resolve to the same path" in out.stderr

    # and a symlinked ancestor: ~/config-link -> ~/.config
    (home / "config-link").symlink_to(home / ".config")
    (sandbox / "myenv2").mkdir(exist_ok=True)
    (sandbox / "myenv2" / "configs" / "x").mkdir(parents=True, exist_ok=True)
    (sandbox / "myenv2" / "configs" / "x" / "a.conf").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv2",
        {
            "one": """
import { module, trackedCopy } from "@gripsack/core";

export default module("one", {
  config: { "configs/x/a.conf": trackedCopy("~/.config/aliased/x.conf") },
});
""",
            "two": """
import { module, trackedCopy } from "@gripsack/core";

export default module("two", {
  config: { "configs/x/a.conf": trackedCopy("~/config-link/aliased/x.conf") },
});
""",
        },
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "resolve to the same path" in out.stderr, out.stderr


def test_same_module_duplicate_destination_is_e111(sandbox):
    """0030 §P0-1: E111's same-module suppression is gone — two
    declarations of one destination in ONE module would double-journal
    it, the second entry overwriting the first's true prior."""
    confdir = sandbox / "myenv" / "configs" / "x"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    (confdir / "b.conf").write_text("b\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("one", {
  config: {
    "configs/x/a.conf": trackedCopy("~/.out/same.conf"),
    "configs/x/b.conf": trackedCopy("~/.out/same.conf"),
  },
});
""",
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E111" in out.stderr


def test_executable_tracked_copy_stays_coherent(sandbox):
    """0030 §H3 (Option A): a tracked copy manages executability — a
    fresh 0755 deploy lands executable, the next apply is satisfied,
    and an exec-bit change from the repo applies."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    script = confdir / "tool.sh"
    script.write_text("#!/bin/sh\necho v1\n")
    script.chmod(0o755)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/tool.sh": trackedCopy("~/.config/demo/tool.sh") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    import os

    dest = sandbox / ".config/demo/tool.sh"
    assert os.stat(dest).st_mode & 0o111, "the exec bit must land"

    # the next apply is satisfied — the exec-aware identity agrees
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert "unchanged" in out.stdout, out.stdout

    # the repo drops the exec bit — the update applies it
    script.chmod(0o644)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not os.stat(dest).st_mode & 0o111, "the exec update applies"


def test_rename_keeps_full_lineage_authority(sandbox):
    """0030 §H4: renaming a module with a content change is an
    authorized UPDATE (not preserved-as-foreign), and undeclare
    restores the original adoption origin."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("v1\n")
    original = sandbox / ".config/demo/a.toml"
    original.parent.mkdir(parents=True)
    original.write_text("ORIGINAL\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.config/demo/a.toml", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr

    # rename the module AND change the content
    adopted = repo / "configs" / "a-toml" / "a.toml"
    adopted.write_text("v2\n")
    (repo / "modules" / "a-toml.ts").unlink()
    (repo / "modules" / "renamed.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("renamed", {
  config: { "configs/a-toml/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
"""
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "updated" in out.stdout, out.stdout
    assert original.read_text() == "v2\n"

    remove_module(repo, "renamed")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert original.read_text() == "ORIGINAL\n"


def test_double_takeover_keeps_the_first_origin(sandbox):
    """0030 §H5: take-over RETAINS the epoch's origin — a second
    --take-over absorbs content but does not rebase the restore
    point. Undeclare restores the FIRST pre-adoption state."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("A\n")
    original = sandbox / ".config/demo/a.toml"
    original.parent.mkdir(parents=True)
    original.write_text("ORIGINAL\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.config/demo/a.toml", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr

    # the app drifts; a second take-over absorbs B over U
    adopted = repo / "configs" / "a-toml" / "a.toml"
    original.write_text("U\n")
    adopted.write_text("B\n")
    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert original.read_text() == "B\n"

    remove_module(repo, "a-toml")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert original.read_text() == "ORIGINAL\n", (
        "the epoch's FIRST origin must survive a second take-over"
    )


def test_legacy_run_marker_refuses_with_guidance(sandbox):
    """0030 §11: a pre-0.23 run marker (no previous_generation, no
    format field) is not auto-classified by the model-proven-unsound
    direction rule — recovery refuses and tells you what to do."""
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
    assert out.returncode != 0, "a legacy marker must block"
    assert "predates 0.23" in out.stderr, out.stderr


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


def test_jobs_one_forces_serial_execution(sandbox):
    """--jobs bounds the scheduler (N3): the 2x2s parallel proof
    inverted — with --jobs 1 it must take serial time."""
    import time

    repo = make_env_repo(
        sandbox / "myenv",
        {
            name: f"""
import {{ module }} from "@gripsack/core";

export default module("{name}", {{
  build: {{ kind: "custom_shell", script: "sleep 2" }},
}});
"""
            for name in ("slow-a", "slow-b")
        },
    )
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", "--jobs", "1", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed >= 3.5, f"--jobs 1 not respected: {elapsed:.1f}s"


def test_independent_modules_run_in_parallel(sandbox):
    """Two independent 2s builds overlap (0007 §5 — the ready-queue
    scheduler runs N = cores). Self-relative: measure a --jobs 1
    baseline in this same sandbox, then assert the default-jobs run
    beats it by the overlap — wall-clock absolutes flake on slow
    runners (the macOS round taught this)."""
    import time

    modules = {
        name: f"""
import {{ module }} from "@gripsack/core";

export default module("{name}", {{
  build: {{ kind: "custom_shell", script: "sleep 2" }},
}});
"""
        for name in ("slow-a", "slow-b")
    }

    repo = make_env_repo(sandbox / "serial", modules)
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", "--jobs", "1", cwd=repo)
    serial = time.monotonic() - start
    assert out.returncode == 0, out.stderr

    repo = make_env_repo(sandbox / "parallel", modules)
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    parallel = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    # two 2s builds: serial ~4s+overhead, parallel ~2s+overhead — the
    # speedup must recover at least half the second sleep
    assert parallel < serial - 0.8, (
        f"no overlap detected: serial {serial:.1f}s, parallel {parallel:.1f}s"
    )


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


def test_jobs_zero_is_rejected(sandbox):
    """Finding B: --jobs 0 / GRIPSACK_JOBS=0 must fail loudly, never
    silently unmanage the environment."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("a", {
  config: { "configs/a/a": trackedCopy("~/.out/a") },
});
""",
    )
    (repo / "configs" / "a").mkdir(parents=True)
    (repo / "configs" / "a" / "a").write_text("a\n")
    out = grip("apply", "--host", "testhost", "--jobs", "0", cwd=repo)
    assert out.returncode != 0
    assert "--jobs 0" in out.stderr
    assert not (sandbox / ".local/share/gripsack/generations").exists()


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


def test_fonts_and_desktop_entry_adapters_run_once_per_apply(sandbox, monkeypatch):
    """PostLink intents: fonts() runs fc-cache, desktop_entry() runs
    update-desktop-database — deduped across modules, tolerating
    absence (0001 §3.8)."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    log = sandbox / "calls.log"
    for tool in ("fc-cache", "update-desktop-database"):
        (bindir / tool).write_text(f'#!/bin/sh\necho "{tool} $@" >> {log}\n')
        (bindir / tool).chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    confdir = sandbox / "myenv" / "configs" / "font"
    confdir.mkdir(parents=True)
    (confdir / "myfont.ttf").write_text("fake font\n")
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "font-a": """
import { desktopEntry, fonts, module, symlink } from "@gripsack/core";

export default module("font-a", {
  config: { "configs/font/myfont.ttf": symlink("~/.local/share/fonts/myfont.ttf") },
  activate: [fonts(), desktopEntry()],
});
""",
            "font-b": """
import { fonts, module, symlink } from "@gripsack/core";

export default module("font-b", {
  config: { "configs/font/myfont.ttf": symlink("~/.local/share/fonts/myfont-b.ttf") },
  activate: [fonts()],
});
""",
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    calls = log.read_text().splitlines()
    assert calls.count("fc-cache -f") == 1, calls
    assert sum(1 for c in calls if "update-desktop-database" in c and "applications" in c) == 1, calls


def test_fonts_adapter_skips_cleanly_without_fc_cache(sandbox):
    """No fc-cache on PATH → a warning, never an apply error."""
    confdir = sandbox / "myenv" / "configs" / "font"
    confdir.mkdir(parents=True)
    (confdir / "myfont.ttf").write_text("fake font\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { fonts, module, symlink } from "@gripsack/core";

export default module("font", {
  config: { "configs/font/myfont.ttf": symlink("~/.local/share/fonts/myfont.ttf") },
  activate: [fonts()],
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_run_steps_execute_with_declared_outputs(sandbox):
    """run steps (0007 §3 rung 2): structured argv, no shell — declared
    outputs are the contract and a missing one is a step error."""
    payload = make_tarball(sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fetchStep, installStep, module, runStep, symlink, tarball }} from "@gripsack/core";

export default module("hello", {{
  steps: [
    fetchStep(tarball("file://{payload}")),
    runStep(["cp", "bin/hello", "bin/hello-copy"], "copy", {{ outputs: ["bin/hello-copy"] }}),
    installStep({{ "bin/hello-copy": symlink("~/.local/bin/hello-copy") }}),
  ],
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/hello-copy").is_symlink()


def test_step_form_intents_run_through_adapters(sandbox):
    """Step-form intents (class-style) execute via the activation
    adapters — a custom hook's post-activate script really runs."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { customHook, module, symlink } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.txt": symlink("~/.config/demo/a.txt") },
  activate: [customHook("echo post-activate > ~/hook-ran")],
});
""",
    )
    confdir = repo / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / "hook-ran").read_text() == "post-activate\n"


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


def test_merge_block_owner_rename_leaves_no_ghost(sandbox):
    """0026 §2b: renaming the module that owns a merge block must move
    the block — the old module's block is pruned (block ownership is
    per (module, dest)), not left as an unowned ghost beside the new
    one."""
    confdir = sandbox / "myenv" / "configs" / "shell"
    confdir.mkdir(parents=True)
    (confdir / "block.sh").write_text("export RENAMED=1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { merge, module } from "@gripsack/core";

export default module("shell", {
  config: { "configs/shell/block.sh": merge("~/.bashrc") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    bashrc = sandbox / ".bashrc"
    assert "module=shell" in bashrc.read_text()

    (repo / "modules" / "hello.ts").write_text(
        """
import { merge, module } from "@gripsack/core";

export default module("terminal", {
  config: { "configs/shell/block.sh": merge("~/.bashrc") },
});
"""
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert "module=terminal" in content
    assert "module=shell" not in content, f"ghost block left behind: {content}"
    assert content.count("export RENAMED=1") == 1


def test_config_tree_gain_repins_and_deploys_from_store(sandbox):
    """A config tree that gains a file under an unmoved transport pin:
    `grip update` moves the pin, a warm-store apply deploys the new
    file FROM THE STORE (never a link into the repo checkout), and a
    cold store re-pins instead of dying on the stale tree hash."""
    payload = make_tarball(sandbox / "tool.tar.gz", {"bin/tool": b"#!/bin/sh\n"})
    confdir = sandbox / "myenv" / "configs" / "tool"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink, tree }} from "@gripsack/core";

export default module("tool", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/tool": symlink("~/.local/bin/tool") }},
  config: {{ ...tree("configs/tool", "~/.config/tool", "owned") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    store = sandbox / ".local/share/gripsack/store"
    deployed_a = sandbox / ".config" / "tool" / "a.conf"
    assert deployed_a.is_symlink()
    assert str(deployed_a.readlink()).startswith(str(store))

    # the tree gains a file: update reports the move, apply deploys it
    (confdir / "b.conf").write_text("b\n")
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "bumped" in out.stdout, out.stdout
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed_b = sandbox / ".config" / "tool" / "b.conf"
    assert deployed_b.is_symlink()
    target = str(deployed_b.readlink())
    assert target.startswith(str(store)), f"new file deployed from the repo checkout: {target}"

    # cold store: the pin moved, so this re-pins instead of failing
    shutil.rmtree(store)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config" / "tool" / "b.conf").exists()


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



def canonical_sha(data: bytes) -> str:
    """The journal's file identity: canonical_bytes_hash (type tag +
    exec byte + contents) — what deploy's mark_after records."""
    import hashlib

    return hashlib.sha256(b"file\0" + b"\x00" + data).hexdigest()


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
        "prior": {"kind": "file", "hash": prior_sha},
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
        "prior": {"kind": "file", "hash": prior_sha},
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
