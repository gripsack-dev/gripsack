"""install.sh against a local fake release host."""
import os
import subprocess
from pathlib import Path

import pytest
from fake_release import FakeRelease, peer

INSTALL_SH = Path(__file__).resolve().parent.parent / "install.sh"


@pytest.fixture
def release():
    server = FakeRelease()
    yield server
    server.close()


def install(release, tmp_path, *args, **env):
    environment = os.environ | {"GRIPSACK_BASE_URL": release.url, "GRIPSACK_BIN": str(tmp_path / "bin")}
    environment.pop("GRIPSACK_VERSION", None)
    return subprocess.run(["sh", str(INSTALL_SH), *args], env=environment | env,
                          capture_output=True, text=True, timeout=60)


def test_installs_the_latest_release(release, tmp_path):
    release.publish("1.2.3")
    result = install(release, tmp_path)
    assert result.returncode == 0, result.stderr
    assert "1.2.3" in result.stdout
    assert (tmp_path / "bin/grip").read_bytes() == peer("1.2.3")
    assert os.access(tmp_path / "bin/grip", os.X_OK)
    assert not [p for p in (tmp_path / "bin").iterdir() if p.name != "grip"]


@pytest.mark.parametrize("how", ["flag", "env"])
def test_installs_an_exact_version(release, tmp_path, how):
    release.publish("1.2.3")
    release.publish("1.3.0")
    args, env = (("--version", "1.2.3"), {}) if how == "flag" else ((), {"GRIPSACK_VERSION": "1.2.3"})
    result = install(release, tmp_path, *args, **env)
    assert result.returncode == 0, result.stderr
    assert (tmp_path / "bin/grip").read_bytes() == peer("1.2.3")
    assert "/releases/latest" not in release.requests


def test_checksum_mismatch_installs_nothing(release, tmp_path):
    release.publish("1.2.3", corrupt_checksum=True)
    (tmp_path / "bin").mkdir()
    (tmp_path / "bin/grip").write_bytes(b"old")
    result = install(release, tmp_path)
    assert result.returncode != 0
    assert "checksum mismatch" in result.stderr
    assert (tmp_path / "bin/grip").read_bytes() == b"old"
    assert [p.name for p in (tmp_path / "bin").iterdir()] == ["grip"]
