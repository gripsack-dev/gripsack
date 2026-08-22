"""E2E harness (plan/0003 §5, .agents/skills/gripsack-e2e).

Rules:
- everything under tmp_path; HOME and GRIPSACK_HOME are redirected there —
  a test can never touch the developer's real profile;
- offline only: sources are file:// fixture tarballs built here, or local
  git repos. Network in e2e is a bug;
- the binary comes from GRIPSACK_BIN (set in docker; the gate stage has
  already compiled it — never rebuild from e2e).
"""

from __future__ import annotations

import io
import os
import subprocess
import tarfile
from pathlib import Path

import pytest

GRIP = Path(os.environ.get("GRIPSACK_BIN", "target/debug/grip"))


def make_tarball(path: Path, files: dict[str, bytes]) -> Path:
    """Build a fixture tarball: module payload without the network."""
    with tarfile.open(path, "w:gz") as tar:
        for name, content in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            tar.addfile(info, io.BytesIO(content))
    return path


def make_env_repo(root: Path, modules_py: str) -> Path:
    """A fixture env repo mirroring plan/0001 §5."""
    (root / "modules").mkdir(parents=True)
    (root / "hosts").mkdir()
    (root / "env.toml").write_text('[env]\nname = "fixture"\n')
    (root / "modules" / "hello.py").write_text(modules_py)
    (root / "hosts" / "testhost.py").write_text('tags = ["test"]\n')
    return root


@pytest.fixture
def sandbox(tmp_path, monkeypatch):
    """Redirect everything gripsack touches into tmp_path."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("GRIPSACK_HOME", str(tmp_path / ".local/share/gripsack"))
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)
    return tmp_path


def grip(*args: str, cwd: Path | None = None) -> subprocess.CompletedProcess:
    assert GRIP.exists(), f"grip binary not found at {GRIP} (build first)"
    return subprocess.run(
        [str(GRIP.resolve()), *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=120,
    )
