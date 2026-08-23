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
