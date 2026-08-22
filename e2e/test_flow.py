"""Flow tests (plan/0003 §5). Skipped until the flows land; unskip in the
same PR that implements them (gripsack-e2e skill)."""

import pytest

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


@pytest.mark.skip(reason="0004: grip apply not implemented")
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


@pytest.mark.skip(reason="0004: grip apply not implemented")
def test_rollback_restores_previous_generation(sandbox):
    repo = make_env_repo(sandbox / "myenv", "# modules go here\n")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    current = sandbox / ".local/share/gripsack/current"
    assert current.resolve().name == "1"
