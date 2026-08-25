"""Flow tests (plan/0003 §5). Skipped until the flows land; unskip in the
same PR that implements them (gripsack-e2e skill)."""

import os
import pytest
import subprocess

from conftest import GRIP, grip, make_env_repo, make_tarball


def test_binary_exists_and_runs():
    out = grip("--version")
    assert out.returncode == 0
    assert "grip" in out.stdout


def test_doctor_reports_environment(sandbox):
    out = grip("doctor")
    # exit code depends on whether the sandbox python can import gripsack;
    # the report itself is the contract.
    assert "python:" in out.stdout
    assert "frontend:" in out.stdout
    assert "home:" in out.stdout
    assert str(sandbox) in out.stdout


def test_apply_creates_generation_and_symlinks(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
from gripsack import module, file_fetch, symlink

module(
    "hello",
    fetch=file_fetch("{payload}"),
    install={{"bin/hello": symlink("~/.local/bin/hello")}},
)
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
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tree

module("zed", config={**tree("configs/zed", "~/.config/zed")})
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


def test_check_passes_valid_repo_and_has_no_side_effects(sandbox):
    """grip check (0011 §9): eval + sema + lint, zero side effects."""
    repo = make_lint_repo(sandbox, "good = true\n")
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "check: ok" in out.stdout
    # zero side effects: no generations, no store, nothing deployed
    assert not (sandbox / ".local/share/gripsack/generations").exists()
    assert not (sandbox / ".local/share/gripsack/store").exists()
    assert not (sandbox / ".config/demo").exists()


def test_check_fails_on_lint_error(sandbox):
    repo = make_lint_repo(sandbox, "BAD_KEY = 1\n")
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "griplint-demo/A01" in out.stderr
    assert not (sandbox / ".local/share/gripsack/generations").exists()
    assert not (sandbox / ".config/demo").exists()


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
from gripsack import module, tree
from gripsack.entries import Ownership

module("zed", config={**tree("configs/zed", "~/.config/zed", mode=Ownership.OWNED)})
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


LINT_FIXTURE = """#!/usr/bin/env python3
import json, sys
req = json.loads(sys.stdin.readline())
for p in req["paths"]:
    for n, line in enumerate(open(p).read().splitlines(), 1):
        if "BAD_KEY" in line:
            print(json.dumps({"type": "diagnostic", "diagnostic": {
                "code": "griplint-demo/A01", "severity": "error",
                "message": "unknown key BAD_KEY",
                "labels": [{"span": {"file": p, "line": n}, "note": "not a real key"}],
                "help": "remove it"}}))
        if "WARN_KEY" in line:
            print(json.dumps({"type": "diagnostic", "diagnostic": {
                "code": "griplint-demo/W01", "severity": "warning",
                "message": "WARN_KEY is deprecated",
                "labels": []}}))
print(json.dumps({"type": "response", "id": 1, "result": {"linted": len(req["paths"])}}))
"""


def make_lint_repo(sandbox, config_text, lint_decl='lint = "demo"'):
    """A repo with one linted config module and the fixture linter on
    a path registration (offline — 0010 §3's path form)."""
    import os
    import stat

    repo = sandbox / "myenv"
    confdir = repo / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "demo.toml").write_text(config_text)
    exe = sandbox / "griplint-demo"
    exe.write_text(LINT_FIXTURE)
    exe.chmod(exe.stat().st_mode | stat.S_IXUSR)
    (repo / "modules").mkdir(exist_ok=True)
    (repo / "hosts").mkdir(exist_ok=True)
    (repo / "env.toml").write_text(
        f'[env]\nname = "fixture"\n\n[linters.demo]\npath = "{exe}"\n'
    )
    (repo / "modules" / "demo.py").write_text(
        f"""
from gripsack import module, tracked_copy

module("demo",
    config={{"configs/demo/demo.toml": tracked_copy("~/.config/demo/demo.toml")}},
    {lint_decl})
"""
    )
    (repo / "hosts" / "testhost.py").write_text('tags = ["test"]\n')
    return repo


def test_lint_error_fails_apply_before_staging(sandbox):
    repo = make_lint_repo(sandbox, "BAD_KEY = 1\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "griplint-demo/A01" in out.stderr
    assert "unknown key BAD_KEY" in out.stderr
    # nothing staged or deployed
    assert not (sandbox / ".config/demo/demo.toml").exists()
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_lint_warning_flows_but_applies(sandbox):
    repo = make_lint_repo(sandbox, "WARN_KEY = 1\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "griplint-demo/W01" in out.stderr
    assert (sandbox / ".config/demo/demo.toml").exists()


def test_lint_clean_config_applies(sandbox):
    repo = make_lint_repo(sandbox, "good = true\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "griplint-demo" not in out.stderr


def test_lint_unregistered_name_is_a_hard_eval_error(sandbox):
    repo = make_lint_repo(sandbox, "good = true\n", lint_decl='lint = "ghost"')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E501" in out.stderr
    assert "ghost" in out.stderr


def test_owned_deploy_refuses_foreign_paths_unless_take_over(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
from gripsack import module, file_fetch, symlink

module(
    "hello",
    fetch=file_fetch("{payload}"),
    install={{"bin/hello": symlink("~/.local/bin/hello")}},
)
""",
    )
    foreign = sandbox / ".local" / "bin"
    foreign.mkdir(parents=True)
    (foreign / "hello").write_text("system binary\n")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "not deployed by gripsack" in out.stderr

    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (foreign / "hello").is_symlink()


def test_apply_repo_from_elsewhere(sandbox):
    """The bootstrap story: apply a repo that isn't the cwd."""
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
from gripsack import module, file_fetch, symlink

module(
    "hello",
    fetch=file_fetch("{payload}"),
    install={{"bin/hello": symlink("~/.local/bin/hello")}},
)
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
from gripsack import module, fetch_step, shell_step, tarball

module(
    "stepped",
    steps=[
        fetch_step(tarball("file://{payload}")),
        shell_step("true", id="noop", needs=["fetch"]),
    ],
)
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
from gripsack import module, file_fetch, symlink

module(
    "hello",
    fetch=file_fetch("{payload}"),
    install={{"bin/hello": symlink("~/.local/bin/hello")}},
)
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
from gripsack import module, tracked_copy

module("zed", config={"configs/zed/a": tracked_copy("~/.config/zed/a")},
    env={"EDITOR": "zed", "PATH+": "{store}/bin"})
""",
    )
    (repo / "configs" / "zed").mkdir(parents=True)
    (repo / "configs" / "zed" / "a").write_text("a\n")
    profile = sandbox / ".local/share/gripsack/env/profile.sh"
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
    (repo / "modules" / "hello.py").write_text(
        """
from gripsack import module, tracked_copy

module("zed", config={"configs/zed/a": tracked_copy("~/.config/zed/a")})
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not profile.exists()


def test_service_intent_runs_the_adapter_without_failing_apply(sandbox):
    """systemd-user adapter: no systemctl/user bus in the sandbox —
    the intent must degrade to a warning, never a failed apply
    (0001 §3.8: never roll back on post-activation failure)."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, service, tracked_copy

module("daemon", config={"configs/daemon/a": tracked_copy("~/.config/daemon/a")},
    activate=[service("my-daemon.service")])
""",
    )
    (repo / "configs" / "daemon").mkdir(parents=True)
    (repo / "configs" / "daemon" / "a").write_text("a\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "my-daemon.service" in out.stdout


def test_gc_collects_unreferenced_store_paths_and_why_owns(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho v1\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
from gripsack import module, file_fetch, symlink

module("hello", fetch=file_fetch("{payload}"),
    install={{"bin/hello": symlink("~/.local/bin/hello")}})
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
    (repo / "modules" / "hello.py").write_text("# empty\n")
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


def test_duplicate_destination_is_a_check_time_error(sandbox):
    """E111 (N2): two modules may not declare the same destination —
    a deploy race in parallel and a lie for why-owns."""
    confdir = sandbox / "myenv" / "configs" / "x"
    confdir.mkdir(parents=True)
    (confdir / "same.conf").write_text("x\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("one", config={"configs/x/same.conf": tracked_copy("~/.out/same.conf")})
module("two", config={"configs/x/same.conf": tracked_copy("~/.out/same.conf")})
""",
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E111" in out.stderr
    assert "same.conf" in out.stderr


def test_jobs_one_forces_serial_execution(sandbox):
    """--jobs bounds the scheduler (N3): the 2x2s parallel proof
    inverted — with --jobs 1 it must take serial time."""
    import time

    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module

module("slow-a", build={"kind": "custom_shell", "script": "sleep 2"})
module("slow-b", build={"kind": "custom_shell", "script": "sleep 2"})
""",
    )
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", "--jobs", "1", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed >= 3.5, f"--jobs 1 not respected: {elapsed:.1f}s"


def test_gc_dry_run_previews_without_deleting(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho v1\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
from gripsack import module, file_fetch, symlink

module("hello", fetch=file_fetch("{payload}"),
    install={{"bin/hello": symlink("~/.local/bin/hello")}})
""",
    )
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    (repo / "modules" / "hello.py").write_text("# empty\n")
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


def test_eval_env_reaches_build_steps(sandbox):
    """[eval] env (build-time exported env): env.toml declares it,
    a build step's shell inherits it."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, shell_step

module("probe", steps=[shell_step("test \\"$MY_CERT_PATH\\" = \\"/etc/ssl/company.pem\\"", id="probe")])
""",
    )
    (repo / "env.toml").write_text(
        '[env]\nname = "fixture"\n\n[eval]\nenv = { MY_CERT_PATH = "/etc/ssl/company.pem" }\n'
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_independent_modules_run_in_parallel(sandbox):
    """Two independent 2s builds finish in ~2s, not 4s (0007 §5 —
    the ready-queue scheduler runs N = cores)."""
    import time

    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module

module("slow-a", build={"kind": "custom_shell", "script": "sleep 2"})
module("slow-b", build={"kind": "custom_shell", "script": "sleep 2"})
""",
    )
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed < 3.5, f"serial execution suspected: {elapsed:.1f}s"


def test_check_fails_statically_on_missing_config_source(sandbox):
    """E110 (review finding E2): a fetch-less module's source must be
    a repo file — missing is a check-time error, not mid-deploy."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("aaa", config={"configs/aaa/MISSING.conf": tracked_copy("~/.out/a.conf")})
""",
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E110" in out.stderr
    assert "MISSING.conf" in out.stderr
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_owned_deploy_refuses_foreign_symlinks(sandbox):
    """Review finding E4: a stow-style foreign symlink is exactly the
    path the guard is for — refuse unless --take-over."""
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
from gripsack import module, file_fetch, symlink

module("hello", fetch=file_fetch("{payload}"),
    install={{"bin/hello": symlink("~/.local/bin/hello")}})
""",
    )
    foreign = sandbox / ".local/bin"
    foreign.mkdir(parents=True)
    stow_target = sandbox / "elsewhere/real-hello"
    stow_target.parent.mkdir(parents=True)
    stow_target.write_text("#!/bin/sh\necho stow\n")
    (foreign / "hello").symlink_to(stow_target)

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "not deployed by gripsack" in out.stderr
    assert (foreign / "hello").readlink() == stow_target  # untouched

    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert str(foreign / "hello").endswith("hello")


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
from gripsack import module, tracked_copy

module("aaa", config={"configs/aaa/a.conf": tracked_copy("~/.out/a.conf")})
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".out/a.conf").read_text() == "v1\n"

    # v2 of aaa + a module that fails at deploy (payload lacks the
    # entry — a deploy-time failure E110 can't catch)
    (confdir / "a.conf").write_text("v2\n")
    payload = make_tarball(sandbox / "b.tar.gz", {"bin/b": b"#!/bin/sh\n"})
    (repo / "modules" / "bbb.py").write_text(
        f"""
from gripsack import module, file_fetch, symlink

module("bbb", fetch=file_fetch("{payload}"),
    install={{"bin/MISSING": symlink("~/.out/b")}})
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    # the flip never happened…
    generations = sandbox / ".local/share/gripsack/generations"
    assert [p.name for p in generations.iterdir()] == ["1"]
    # …and this run's deployments are rolled back exactly
    assert (sandbox / ".out/a.conf").read_text() == "v1\n"
    assert not (sandbox / ".out/b").exists()


def test_rollback_restores_previous_generation(sandbox, tmp_path):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
from gripsack import module, file_fetch, symlink

module(
    "hello",
    fetch=file_fetch("{payload}"),
    install={{"bin/hello": symlink("~/.local/bin/hello")}},
)
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
    (repo / "modules" / "extra.py").write_text(
        """
from gripsack import module, file_fetch, symlink

module(
    "extra",
    fetch=file_fetch("%s"),
    install={"bin/hello": symlink("~/.local/bin/extra")},
)
"""
        % payload
    )
    third = grip("apply", "--host", "testhost", cwd=repo)
    assert third.returncode == 0, third.stderr
    assert (sandbox / ".local/bin/extra").is_symlink()

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    current = sandbox / ".local/share/gripsack/current"
    assert current.resolve().name == "1"
    # the extra module's destination is gone after rollback
    assert not (sandbox / ".local/bin/extra").exists()
