"""Read-only and unavailable-manager cases; never use the developer's manager.
Real install/update/dispatch/rollback/GC smoke: scripts/smoke_scheduled_jobs.py
in a dedicated disposable user-manager account (see scheduled example README).
"""
import json
import os

from conftest import grip


def declaration(repo, installed=False):
    repo.mkdir(exist_ok=True)
    selection = '["daily"]' if installed else '[]'
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, task, profile, exec, lit, schedule, daily,
    } from "@gripsack/core";
    export default defineWorkspace(() => workspace({outputs:[
      task("once",{steps:[exec(lit("/bin/true")).build()]}),
      schedule("daily",{task:"once",trigger:daily("09:30")}),
      profile("personal",{schedules:''' + selection + '''}),
    ]}));''')


def test_capability_is_effect_free_with_unreachable_manager(sandbox, monkeypatch):
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=" + str(sandbox / "absent-bus"))
    monkeypatch.setenv("XDG_RUNTIME_DIR", str(sandbox / "runtime"))
    home = sandbox / "unmaterialized-store"
    result = grip("jobs", "--home", str(home), "capability", "--json", approve=False)
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["status"] != "available"
    assert not home.exists()
    assert not (sandbox / ".config/systemd/user").exists()


def test_inert_schedule_does_not_need_manager_and_failed_prepare_preserves_generation(sandbox, monkeypatch):
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=" + str(sandbox / "absent-bus"))
    monkeypatch.setenv("XDG_RUNTIME_DIR", str(sandbox / "runtime"))
    repo = sandbox / "workspace"
    declaration(repo)
    preview = grip("task", "plan", "once", "--json", cwd=repo)
    assert preview.returncode == 0, preview.stderr
    applied = grip("apply", "personal", cwd=repo)
    assert applied.returncode == 0, applied.stderr
    current = sandbox / ".local/share/gripsack/current"
    before = os.readlink(current)
    declaration(repo, installed=True)
    failed = grip("apply", "personal", cwd=repo)
    assert failed.returncode != 0
    assert "systemd" in failed.stderr.lower() or "manager" in failed.stderr.lower()
    assert os.readlink(current) == before
    assert not (sandbox / ".config/systemd/user").exists()


def test_untrusted_dispatch_arguments_cannot_name_manifest_paths(sandbox):
    home = sandbox / "store"
    bad = grip("jobs", "--home", str(home), "dispatch", "../../manifest", "a" * 64, approve=False)
    assert bad.returncode != 0
    assert not home.exists()
