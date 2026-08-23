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
