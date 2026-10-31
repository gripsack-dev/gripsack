"""Offline engine-unavailable admission; the PostgreSQL journey uses a real Engine."""
from conftest import grip


def test_missing_engine_fails_without_panicking_or_creating_ownership(sandbox, monkeypatch):
    repo = sandbox / "services"
    repo.mkdir()
    (repo / "gripsack.ts").write_text('''import {defineWorkspace,workspace,service} from "@gripsack/core";
export default defineWorkspace(() => workspace({outputs:[service("database",{
  image:"postgres@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  health:{kind:"command",argv:["true"],interval_seconds:1,timeout_seconds:1,retries:1}
})]}));
''')
    monkeypatch.setenv("DOCKER_HOST", "unix://" + str(sandbox / "missing-docker.sock"))
    update = grip("update", cwd=repo)
    assert update.returncode == 0, update.stderr
    result = grip("up", "--timeout-seconds", "5", cwd=repo)
    # A dependency being absent is an operational failure, not an internal
    # panic (101). No container ownership intent exists before Engine admission.
    assert result.returncode == 1, result.stderr
    assert not (sandbox / ".local/share/gripsack/services").exists()
    assert not (sandbox / ".local/share/gripsack/current").exists()
