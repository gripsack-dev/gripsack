"""`grip self-update` against a local fake release host (debug builds honour
GRIPSACK_RELEASE_BASE_URL; release binaries always talk to GitHub)."""
import shutil
import subprocess

import pytest
from conftest import GRIP
from fake_release import FakeRelease, peer


@pytest.fixture
def release():
    server = FakeRelease()
    yield server
    server.close()


@pytest.fixture
def exe(sandbox):
    path = sandbox / "bin/grip"
    path.parent.mkdir()
    shutil.copyfile(GRIP.resolve(), path)
    path.chmod(0o755)
    return path


def current_version(exe) -> str:
    return subprocess.run([str(exe), "--version"], capture_output=True, text=True, check=True).stdout.split()[1]


def self_update(exe, release, sandbox, *args):
    env = {"HOME": str(sandbox), "GRIPSACK_HOME": str(sandbox / ".local/share/gripsack"),
           "GRIPSACK_RELEASE_BASE_URL": release.url, "PATH": "/usr/bin:/bin"}
    return subprocess.run([str(exe), "self-update", *args], env=env, capture_output=True, text=True, timeout=120)


def test_replaces_an_older_binary(release, exe, sandbox):
    release.publish("99.0.0")
    result = self_update(exe, release, sandbox)
    assert result.returncode == 0, result.stderr
    assert exe.read_bytes() == peer("99.0.0")


def test_check_reports_without_replacing(release, exe, sandbox):
    release.publish("99.0.0")
    before = exe.read_bytes()
    result = self_update(exe, release, sandbox, "--check")
    assert result.returncode == 0, result.stderr
    assert "99.0.0" in result.stdout
    assert exe.read_bytes() == before
    assert not any(".tar.gz" in path for path in release.requests)


def test_same_version_is_a_no_op_without_reinstall(release, exe, sandbox):
    release.publish(current_version(exe))
    before = exe.read_bytes()
    result = self_update(exe, release, sandbox)
    assert result.returncode == 0, result.stderr
    assert "is current" in result.stdout
    assert exe.read_bytes() == before
    assert not any(".tar.gz" in path for path in release.requests)


def test_checksum_mismatch_preserves_the_installed_binary(release, exe, sandbox):
    release.publish("99.0.0", corrupt_checksum=True)
    before = exe.read_bytes()
    result = self_update(exe, release, sandbox)
    assert result.returncode != 0
    assert "checksum mismatch" in result.stderr
    assert exe.read_bytes() == before
