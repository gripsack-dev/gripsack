"""Flow tests (plan/0003 §5). Skipped until the flows land; unskip in the
same PR that implements them (gripsack-e2e skill)."""

import os
import shutil
from pathlib import Path

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


def test_concurrent_applies_serialize_and_lose_nothing(sandbox):
    """Finding A: two applies over disjoint subsets must not lose a
    manifest update — the lifecycle holds apply.flock."""
    import subprocess

    repo = sandbox / "myenv"
    for name in ("amod", "bmod"):
        confdir = repo / "configs" / name
        confdir.mkdir(parents=True)
        (confdir / f"{name}.conf").write_text(f"{name}\n")
    (repo / "modules").mkdir(parents=True)
    (repo / "hosts").mkdir()
    (repo / "env.toml").write_text('[env]\nname = "fixture"\n')
    (repo / "hosts" / "testhost.py").write_text('tags = ["test"]\n')
    for name in ("amod", "bmod"):
        (repo / "modules" / f"{name}.py").write_text(
            f"""
from gripsack import module, tracked_copy

module("{name}", config={{"configs/{name}/{name}.conf": tracked_copy("~/.out/{name}.conf")}})
""",
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
from gripsack import module, tracked_copy

module("a", config={"configs/a/a": tracked_copy("~/.out/a")})
""",
    )
    (repo / "configs" / "a").mkdir(parents=True)
    (repo / "configs" / "a" / "a").write_text("a\n")
    out = grip("apply", "--host", "testhost", "--jobs", "0", cwd=repo)
    assert out.returncode != 0
    assert "--jobs 0" in out.stderr
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_deferred_identity_is_stable_across_applies(sandbox):
    """Finding C: fetch kinds whose payload hash isn't knowable up
    front (git/pixi/plugin) must produce ONE store path and satisfy
    on the second apply, not a spurious second generation."""
    import subprocess as sp

    remote = sandbox / "remote"
    remote.mkdir()
    env = dict(
        GIT_AUTHOR_NAME="t",
        GIT_AUTHOR_EMAIL="t@t",
        GIT_COMMITTER_NAME="t",
        GIT_COMMITTER_EMAIL="t@t",
        PATH="/usr/bin:/bin:/usr/local/bin",
        HOME=str(sandbox),
    )
    sp.run(["git", "init", "--quiet"], cwd=remote, env=env, check=True)
    (remote / "bin.txt").write_text("payload\n")
    sp.run(["git", "add", "."], cwd=remote, env=env, check=True)
    sp.run(["git", "commit", "--quiet", "-m", "init"], cwd=remote, env=env, check=True)
    rev = sp.run(
        ["git", "rev-parse", "HEAD"], cwd=remote, env=env, check=True, capture_output=True, text=True
    ).stdout.strip()

    repo = make_env_repo(
        sandbox / "myenv",
        f"""
from gripsack import module, git, symlink

module("tool", fetch=git("file://{remote}", "{rev}"),
    install={{"bin.txt": symlink("~/.local/bin/tool")}})
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "applied" in out.stdout
    store = sandbox / ".local/share/gripsack/store"
    paths_after_first = sorted(p.name for p in store.iterdir())
    assert len(paths_after_first) == 1

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "already satisfied" in out.stdout, out.stdout
    assert sorted(p.name for p in store.iterdir()) == paths_after_first


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


def test_merge_mode_owns_one_block_in_a_foreign_file(sandbox):
    """merge: gripsack owns exactly one delimited block; everything
    outside the markers is never touched (0001 §3.7)."""
    confdir = sandbox / "myenv" / "configs" / "shell"
    confdir.mkdir(parents=True)
    (confdir / "block.sh").write_text('export PATH="$HOME/.local/bin:$PATH"\n')
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, merge

module("shell", config={"configs/shell/block.sh": merge("~/.bashrc")})
""",
    )
    # foreign file with pre-existing content
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("# user stuff\nexport EDITOR=hx\n")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert content.startswith("# user stuff\nexport EDITOR=hx\n")
    assert "# >>> gripsack module=shell >>>" in content
    assert 'export PATH="$HOME/.local/bin:$PATH"' in content
    assert "# <<< gripsack <<<" in content

    # re-apply is satisfied; user drift INSIDE the block self-heals,
    # user content outside is untouched
    healed = content.replace("export PATH", "# user edited\nexport PATH")
    bashrc.write_text(healed + "\n# more user stuff\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert content.count("# >>> gripsack module=shell >>>") == 1
    assert "# user edited" not in content
    assert content.endswith("# more user stuff\n")

    # undeclare prunes only the block; the foreign file stays
    (sandbox / "myenv" / "modules" / "hello.py").write_text("")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert "gripsack" not in content
    assert "# user stuff" in content
    assert "# more user stuff" in content


def test_template_mode_renders_vars_at_deploy(sandbox):
    """template: {{ name }} placeholders render from entry vars at
    deploy time; undefined variables fail loudly (0001 §3.7)."""
    confdir = sandbox / "myenv" / "configs" / "git"
    confdir.mkdir(parents=True)
    (confdir / "id.toml").write_text(
        'email = "{{ email }}"\nname = "{{ name }}"\nliteral = "{{{{ keep }}"\n'
    )
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, template

module("git", config={"configs/git/id.toml": template(
    "~/.config/git/id.toml",
    vars={"email": "a@b.c", "name": "T"},
)})
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".config" / "git" / "id.toml"
    assert deployed.read_text() == 'email = "a@b.c"\nname = "T"\nliteral = "{{ keep }}"\n'

    # changing a var updates the rendered dest on the next apply
    (sandbox / "myenv" / "modules" / "hello.py").write_text(
        """
from gripsack import module, template

module("git", config={"configs/git/id.toml": template(
    "~/.config/git/id.toml",
    vars={"email": "x@y.z", "name": "T"},
)})
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert 'email = "x@y.z"' in deployed.read_text()

    # an undefined variable fails at apply, never silently empty
    (sandbox / "myenv" / "modules" / "hello.py").write_text(
        """
from gripsack import module, template

module("git", config={"configs/git/id.toml": template(
    "~/.config/git/id.toml",
    vars={"email": "x@y.z"},
)})
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "undefined variable" in out.stderr


def test_init_scaffolds_a_working_env_repo(sandbox):
    """grip init: embedded template (offline, version-matched), never
    clobbers an existing env repo."""
    repo = sandbox / "fresh"
    repo.mkdir()
    out = grip("init", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / "env.toml").exists()
    assert (repo / "modules" / "hello.py").exists()
    assert (repo / "modules" / "examples.py").exists()
    assert (repo / "configs" / "hello" / "hello.toml").exists()
    hosts = list((repo / "hosts").glob("*.py"))
    assert len(hosts) == 1
    assert (repo / ".git").is_dir()

    # the scaffold is a working repo: check and apply succeed offline
    out = grip("check", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config" / "hello" / "hello.toml").is_symlink()

    # init never clobbers an existing env repo
    out = grip("init", cwd=repo)
    assert out.returncode != 0
    assert "already looks like an env repo" in out.stderr


FETCH_FIXTURE = """#!/usr/bin/env python3
import json, os, sys
req = json.loads(sys.stdin.readline())
if req["op"] == "capabilities":
    print(json.dumps({"type": "response", "result": {
        "capabilities": {"throttle": {"demo.local": "1/s"}}}}))
elif req["op"] == "fetch":
    dest = req["dest_dir"]
    os.makedirs(os.path.join(dest, "bin"), exist_ok=True)
    with open(os.path.join(dest, "bin", "demo"), "w") as f:
        f.write("#!/bin/sh\\necho demo\\n")
    print(json.dumps({"type": "response", "result": {}}))
sys.stdout.flush()
"""


def test_throttle_token_bucket_serializes_plugin_fetches(sandbox, monkeypatch):
    """[throttle] (0002): a fetcher declares its rate budget via the
    capabilities op; the core's token bucket enforces it across
    concurrent modules — two fetches at 1/s take >= ~1s wall."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, plugin_fetch, symlink

module("a", fetch=plugin_fetch("demo"), install={"bin/demo": symlink("~/.local/bin/demo-a")})
module("b", fetch=plugin_fetch("demo"), install={"bin/demo": symlink("~/.local/bin/demo-b")})
""",
    )
    import time

    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed >= 0.9, f"two 1/s fetches finished in {elapsed:.2f}s — bucket not enforced"
    assert (sandbox / ".local/bin/demo-a").is_symlink()
    assert (sandbox / ".local/bin/demo-b").is_symlink()


def test_throttle_user_override_beats_plugin_budget(sandbox, monkeypatch):
    """env.toml [throttle] outranks the fetcher's own declaration."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, plugin_fetch, symlink

module("a", fetch=plugin_fetch("demo"), install={"bin/demo": symlink("~/.local/bin/demo-a")})
module("b", fetch=plugin_fetch("demo"), install={"bin/demo": symlink("~/.local/bin/demo-b")})
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[throttle]\n"demo.local" = "100/s"\n')
    import time

    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed < 0.9, f"override not applied — took {elapsed:.2f}s"


CRASHY_LINTER = """#!/usr/bin/env python3
import json, sys
req = json.loads(sys.stdin.readline())
print(json.dumps({"type": "diagnostic", "diagnostic": {
    "code": "griplint-demo/E99", "severity": "error",
    "message": "linter crashed: boom", "labels": []}}))
print(json.dumps({"type": "response", "id": 1, "result": {"linted": 0}}))
"""


def test_crash_class_lint_codes_are_warnings_core_side(sandbox):
    """0012: a linter's self-reported severity for crash-class codes
    (E99/E02) is not evidence — the CORE host classifies by code,
    always warning, so lint= never becomes an availability dependency
    (review finding E, enforcement round)."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    linter = bindir / "griplint-demo"
    linter.write_text(CRASHY_LINTER)
    linter.chmod(0o755)
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "demo.toml").write_text("good = true\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("demo", config={"configs/demo/demo.toml": tracked_copy("~/.config/demo/demo.toml")},
       lint="demo")
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write(f'\n[linters.demo]\npath = "{linter}"\n')
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "griplint-demo/E99" in out.stderr
    assert "warning" in out.stderr.lower()


CHATTY_LINTER = """#!/usr/bin/env python3
import json, sys
req = json.loads(sys.stdin.readline())
sys.stderr.write("noise\\n" * 20000)  # >64KB — fills an undrained pipe
sys.stderr.flush()
print(json.dumps({"type": "response", "id": 1, "result": {"linted": len(req["paths"])}}))
"""


def test_chatty_linter_does_not_deadlock_the_exchange(sandbox):
    """0012 host hardening: a linter writing >64KB to stderr fills the
    pipe; without a concurrent drain the child blocks before its
    response and the exchange burns the 120s deadline into a false
    'linter is broken' warning (the fetch host's F1 rule, inherited)."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    linter = bindir / "griplint-demo"
    linter.write_text(CHATTY_LINTER)
    linter.chmod(0o755)
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "demo.toml").write_text("good = true\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("demo", config={"configs/demo/demo.toml": tracked_copy("~/.config/demo/demo.toml")},
       lint="demo")
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write(f'\n[linters.demo]\npath = "{linter}"\n')
    import time

    start = time.monotonic()
    out = grip("check", "--host", "testhost", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed < 30, f"chatty linter took {elapsed:.0f}s — stderr not drained concurrently"


def test_unmatched_host_errors_instead_of_empty_tags(sandbox):
    """A hosts/ dir with no matching file must not silently yield empty
    tags — every when(tags=[...]) module would drop and the run would
    report success (enterprise review finding)."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("demo", config={"configs/demo/a": tracked_copy("~/.config/demo/a")})
""",
    )
    (sandbox / "myenv" / "configs" / "demo").mkdir(parents=True)
    (sandbox / "myenv" / "configs" / "demo" / "a").write_text("a\n")
    out = grip("check", "--host", "nosuchhost", cwd=repo)
    assert out.returncode != 0
    assert "no hosts/nosuchhost.py" in out.stderr


def test_default_host_resolves_role_named_entrypoint(sandbox):
    """[env] default_host: containers with random hostnames still get a
    deterministic host entrypoint (enterprise review finding)."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("demo", config={"configs/demo/a": tracked_copy("~/.config/demo/a")})
""",
    )
    (sandbox / "myenv" / "configs" / "demo").mkdir(parents=True)
    (sandbox / "myenv" / "configs" / "demo" / "a").write_text("a\n")
    (sandbox / "myenv" / "hosts" / "testhost.py").unlink()
    (sandbox / "myenv" / "hosts" / "role.py").write_text('tags = ["container"]\n')
    env_toml = repo / "env.toml"
    env_toml.write_text(env_toml.read_text().replace(
        '[env]\nname = "fixture"', '[env]\nname = "fixture"\ndefault_host = "role"'))
    out = grip("check", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "container" in out.stdout


def _seed_plugin_store(sandbox, exe, fixture, tag="1.0"):
    """Pre-seed the managed plugin store as if a prior provision ran."""
    home = sandbox / ".local/share/gripsack"
    bindir = home / "plugins" / exe / tag
    bindir.mkdir(parents=True)
    (bindir / exe).write_text(fixture)
    (bindir / exe).chmod(0o755)
    (home / "plugins" / exe / "current").symlink_to(f"{tag}/")
    (home / "plugins" / "receipts").mkdir(parents=True)
    (home / "plugins" / "receipts" / f"{exe}.toml").write_text(
        f'source = "acme/{exe}"\ntag = "{tag}"\nsha256 = "ab"\n'
    )


def test_fetcher_package_ref_resolves_from_the_plugin_store(sandbox):
    """0012 move 2: [fetchers.x] package = "owner/repo@tag" — with the
    receipt already recording the tag, provisioning is a satisfied
    no-op (no network) and the fetch runs the store's binary, not PATH."""
    _seed_plugin_store(sandbox, "gripfetch-demo", FETCH_FIXTURE)
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, plugin_fetch, symlink

module("a", fetch=plugin_fetch("demo"), install={"bin/demo": symlink("~/.local/bin/demo-a")})
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()


def test_linter_repo_ref_resolves_from_the_plugin_store(sandbox):
    """0012 move 2: a repo-ref [linters.x] package resolves the
    provisioned store binary (the wheel meaning stays for bare names)."""
    _seed_plugin_store(sandbox, "griplint-demo", LINT_FIXTURE)
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "demo.toml").write_text("BAD_KEY = 1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("demo", config={"configs/demo/demo.toml": tracked_copy("~/.config/demo/demo.toml")},
       lint="demo")
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[linters.demo]\npackage = "acme/griplint-demo@1.0"\n')
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "griplint-demo/A01" in out.stderr


def test_builtin_pack_lints_in_process_without_registration(sandbox):
    """0012 move 3: lint="helix" with NO [linters] entry runs the
    embedded pack in-process — no venv, no provisioning, no plugin."""
    confdir = sandbox / "myenv" / "configs" / "helix"
    confdir.mkdir(parents=True)
    (confdir / "config.toml").write_text('[editor]\nscrollof = 5\n')
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("helix", config={"configs/helix/config.toml": tracked_copy("~/.config/helix/config.toml")},
       lint="helix")
""",
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "A01" in out.stderr
    assert "scrolloff" in out.stderr
    # and the satisfied path: the fix goes green
    (confdir / "config.toml").write_text('[editor]\nscrolloff = 5\n')
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_typescript_frontend_evals_and_applies(sandbox):
    """frontend = "typescript": the provisioned bun runs the driver,
    modules register through the shared @gripsack/core instance, and
    apply deploys (0012 — the eval seam lands)."""
    import shutil

    bun = os.environ.get("GRIPSACK_BUN") or shutil.which("bun")
    if not bun:
        pytest.skip("bun not installed (CI's typescript job covers this)")
    # pre-seed the provisioned frontend: the in-repo build of
    # @gripsack/core at the core's version (what npm would serve)
    import gripsack

    core_version = gripsack.__version__
    pkg = (
        sandbox
        / ".local/share/gripsack/frontend-ts"
        / core_version
        / "node_modules/@gripsack/core"
    )
    pkg.mkdir(parents=True)
    ts_src = Path(__file__).parent.parent / "typescript"
    shutil.copytree(ts_src / "dist", pkg / "dist")
    shutil.copy(ts_src / "package.json", pkg / "package.json")

    repo = sandbox / "tsenv"
    (repo / "modules").mkdir(parents=True)
    (repo / "hosts").mkdir()
    (repo / "configs" / "demo").mkdir(parents=True)
    (repo / "env.toml").write_text('[env]\nname = "tsenv"\nfrontend = "typescript"\n')
    (repo / "hosts" / "testhost.ts").write_text('export const tags = ["test"];\n')
    (repo / "configs" / "demo" / "demo.toml").write_text('greeting = "hi"\n')
    (repo / "modules" / "demo.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

module("demo", {
  config: { "configs/demo/demo.toml": trackedCopy("~/.config/demo/demo.toml") },
});
"""
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config" / "demo" / "demo.toml").exists()


def test_fetcher_path_registers_an_offline_executable(sandbox):
    """[fetchers.x] path = ... registers the executable directly —
    the offline/air-gapped route, symmetric with [linters.x] path
    (enterprise review). No PATH lookup, no provisioning."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, plugin_fetch, symlink

module("a", fetch=plugin_fetch("demo"), install={"bin/demo": symlink("~/.local/bin/demo-a")})
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write(f'\n[fetchers.demo]\npath = "{fetcher}"\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()


def test_changed_fetch_spec_re_resolves_instead_of_mirror_blame(sandbox):
    """A fetch-spec edit must not fail as 'the mirror changed' — the
    args are the declaration and the pin follows them (enterprise
    review's stale-plugin-lock papercut)."""
    _seed_plugin_store(sandbox, "gripfetch-demo", FETCH_FIXTURE)
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, plugin_fetch, symlink

module("a", fetch=plugin_fetch("demo"), install={"bin/demo": symlink("~/.local/bin/demo-a")})
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    # same module, now pinned — the lock entry for the OLD args must
    # not compare against the NEW spec
    (sandbox / "myenv" / "modules" / "hello.py").write_text(
        """
from gripsack import module, plugin_fetch, symlink

module("a", fetch=plugin_fetch("demo", package="hello", version="1.0"),
       install={"bin/demo": symlink("~/.local/bin/demo-a")})
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "mirror" not in out.stderr


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
        """
from gripsack import desktop_entry, fonts, module, symlink

module("font-a", config={"configs/font/myfont.ttf": symlink("~/.local/share/fonts/myfont.ttf")},
       activate=[fonts(), desktop_entry()])
module("font-b", config={"configs/font/myfont.ttf": symlink("~/.local/share/fonts/myfont-b.ttf")},
       activate=[fonts()])
""",
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
from gripsack import fonts, module, symlink

module("font", config={"configs/font/myfont.ttf": symlink("~/.local/share/fonts/myfont.ttf")},
       activate=[fonts()])
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_tracked_copy_drift_is_kept_never_clobbered(sandbox):
    """The killer drift policy (0001 §3.7): a user edit inside a
    tracked_copy destination is detected and KEPT — gripsack never
    silently overwrites it (review finding G: this path had zero
    coverage)."""
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "settings.json").write_text('{"theme": "mocha"}\n')
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("zed", config={"configs/zed/settings.json": tracked_copy("~/.config/zed/settings.json")})
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".config" / "zed" / "settings.json"

    # user edits the deployed file (zed rewrites its own config) — the
    # next apply detects drift and KEEPS it
    dest.write_text('{"theme": "nord", "user": true}\n')
    (confdir / "settings.json").write_text('{"theme": "latte"}\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == '{"theme": "nord", "user": true}\n'

    # drift resolved by hand (dest back to the pinned content): gripsack
    # can't tell a restore from a new drift, so it keeps once — and the
    # next apply converges and updates (bounded, no lockfile surgery)
    dest.write_text('{"theme": "mocha"}\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == '{"theme": "latte"}\n'


def test_plan_diffs_against_the_current_generation(sandbox):
    """plan shows what apply WOULD do: new/update/satisfied/prune, plus
    the take-over warning for foreign paths (0004 pass 5)."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import module, tracked_copy

module("demo", config={"configs/demo/a.toml": tracked_copy("~/.config/demo/a.toml")})
""",
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "+ configs/demo/a.toml" in out.stdout

    grip("apply", "--host", "testhost", cwd=repo)
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "= ~/.config/demo/a.toml (satisfied)" in out.stdout

    (confdir / "a.toml").write_text("b\n")
    (confdir / "b.toml").write_text("b\n")
    (sandbox / "myenv" / "modules" / "hello.py").write_text(
        """
from gripsack import module, tracked_copy

module("demo", config={
    "configs/demo/a.toml": tracked_copy("~/.config/demo/a.toml"),
    "configs/demo/b.toml": tracked_copy("~/.config/demo/b.toml"),
})
"""
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "~ configs/demo/a.toml" in out.stdout
    assert "+ configs/demo/b.toml" in out.stdout

    # apply the two-entry module, then drop b → the next plan prunes it
    grip("apply", "--host", "testhost", cwd=repo)
    (sandbox / "myenv" / "modules" / "hello.py").write_text(
        """
from gripsack import module, tracked_copy

module("demo", config={"configs/demo/a.toml": tracked_copy("~/.config/demo/a.toml")})
"""
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "- ~/.config/demo/b.toml (prune)" in out.stdout


def test_rollback_restores_template_rendered_and_merge_block(sandbox):
    """rollback through the ONE engine (0001 §3.5, review verification):
    template destinations get the previous generation's RENDERED bytes
    (re-rendered with recorded vars), and merge entries re-upsert only
    the block — the foreign file's other content survives."""
    confdir = sandbox / "myenv" / "configs" / "app"
    confdir.mkdir(parents=True)
    (confdir / "id.toml").write_text('email = "{{ email }}"\n')
    (confdir / "block.sh").write_text("export A=1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
from gripsack import merge, module, template

module("app", config={
    "configs/app/id.toml": template("~/.config/app/id.toml", vars={"email": "a@b.c"}),
    "configs/app/block.sh": merge("~/.bashrc"),
})
""",
    )
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("# user stuff\nexport EDITOR=hx\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    # generation 2: new template content and new block content
    (confdir / "id.toml").write_text('email = "rendered-v2"\nname = "{{ email }}"\n')
    (confdir / "block.sh").write_text("export A=2\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "rendered-v2" in (sandbox / ".config/app/id.toml").read_text()
    assert "export A=2" in bashrc.read_text()

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    # template: back to the previous generation's rendered bytes
    assert (sandbox / ".config/app/id.toml").read_text() == 'email = "a@b.c"\n'
    # merge: the block reverted, the foreign content untouched
    content = bashrc.read_text()
    assert content.startswith("# user stuff\nexport EDITOR=hx\n")
    assert "export A=1" in content
    assert "export A=2" not in content


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
from gripsack import module, tree
from gripsack.entries import Ownership

module("demo", config={**tree("configs/demo", "~/.config/demo", mode=Ownership.OWNED)})
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


def test_dual_frontend_parity_golden_corpus(sandbox):
    """plan/0007's conformance test, finally: the same fixture env
    evaluated through BOTH frontends must emit identical IR modulo
    provenance (span file/line). Absence-class drift (a missing TS
    feature) has no shadow to hide in."""
    import shutil

    bun = os.environ.get("GRIPSACK_BUN") or shutil.which("bun")
    if not bun:
        pytest.skip("bun not installed (CI's typescript job runs this)")
    fixture_py = Path(__file__).parent / "fixtures" / "parity" / "python"
    fixture_ts = Path(__file__).parent / "fixtures" / "parity" / "ts"
    for repo in (fixture_py, fixture_ts):
        shutil.copytree(repo, sandbox / repo.name, dirs_exist_ok=True)

    # the TS eval needs the provisioned frontend, pre-seeded like the
    # TS e2e (GRIPSACK_TS_FRONTEND escape hatch)
    ts_front = sandbox / "tsfront"
    (ts_front / "node_modules/@gripsack/core").mkdir(parents=True)
    ts_src = Path(__file__).parent.parent / "typescript"
    shutil.copytree(ts_src / "dist", ts_front / "node_modules/@gripsack/core/dist")
    shutil.copy(ts_src / "package.json", ts_front / "node_modules/@gripsack/core/package.json")
    os.environ["GRIPSACK_TS_FRONTEND"] = str(ts_front)

    def eval_ir(repo: Path) -> dict:
        out = grip("check", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        # grip check prints no IR — use the eval via a plan run? no:
        # the frontend's envelope IS the IR; call the frontend the way
        # the core does, via check's side effect? Cleaner: `grip check`
        # succeeded for both — now diff via `grip plan --ir`? plan needs
        # an IR file. The honest diff: run each frontend directly.
        raise NotImplementedError

    import json, subprocess, sys

    py_env = dict(os.environ, PYTHONPATH=str(Path(__file__).parent.parent / "python")
    )
    ir_py = json.loads(
        subprocess.run(
            [sys.executable, "-m", "gripsack", ".", "--host", "testhost"],
            cwd=sandbox / "python", env=py_env, capture_output=True, text=True,
        ).stdout
    )["ir"]
    ir_ts = json.loads(
        subprocess.run(
            [os.environ["GRIPSACK_BUN"] if os.environ.get("GRIPSACK_BUN") else bun,
             str(ts_front / "node_modules/@gripsack/core/dist/src/cli.js"),
             ".", "--host", "testhost"],
            cwd=sandbox / "ts",
            env=dict(os.environ, NODE_PATH=str(ts_front / "node_modules")),
            capture_output=True, text=True,
        ).stdout
    )["ir"]

    def strip_provenance(node):
        if isinstance(node, dict):
            return {
                k: strip_provenance(v)
                for k, v in node.items()
                if k != "span"
            }
        if isinstance(node, list):
            return [strip_provenance(v) for v in node]
        return node

    py_proj, ts_proj = strip_provenance(ir_py), strip_provenance(ir_ts)
    assert py_proj == ts_proj, (
        "frontend IR drift:\n"
        + json.dumps({k: {"py": py_proj["modules"].get(k), "ts": ts_proj["modules"].get(k)}
                      for k in set(py_proj["modules"]) | set(ts_proj["modules"])
                      if py_proj["modules"].get(k) != ts_proj["modules"].get(k)},
                     indent=1)[:4000]
    )
