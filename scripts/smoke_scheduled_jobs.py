#!/usr/bin/env python3
"""Real systemd calendar/lifecycle fixture, never the invoking user's schedules.
Run only as a disposable unprivileged account whose passwd HOME is
/tmp/gripsack-schedule-fixture-*. See the scheduled-task example README.
No manager provisioning, root fallback, linger changes, downloads or clock edits.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import pwd
import subprocess
import shutil
import sys
import tarfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--grip", type=Path, required=True)
    parser.add_argument("--package-bundle", type=Path,
                        help="optional transferred native tool bundle, inside the fresh fixture home")
    parser.add_argument("--package-trust", type=Path,
                        help="independently provided publisher policy for that bundle")
    args = parser.parse_args()
    binary = args.grip.resolve(strict=True)
    account = pwd.getpwuid(os.getuid())
    home = Path(account.pw_dir).resolve()
    if (os.getuid() == 0 or home.parent != Path("/tmp")
            or not home.name.startswith("gripsack-schedule-fixture-")
            or Path(os.environ.get("HOME", "")).resolve() != home):
        parser.error("requires a dedicated unprivileged passwd account under /tmp/gripsack-schedule-fixture-*, never a HOME override")
    marker = home / ".gripsack-disposable-scheduler"
    if marker.read_text().strip() != str(os.getuid()):
        parser.error("disposable-account marker must contain this UID")
    if bool(args.package_bundle) != bool(args.package_trust):
        parser.error("package bundle and independent publisher policy must be supplied together")
    deno = os.environ.get("GRIPSACK_DENO")
    if not deno or not Path(deno).is_file():
        parser.error("set GRIPSACK_DENO to a preinstalled local Deno (fixture never downloads)")
    state = home / "state"
    repo = home / "workspace"
    if state.exists() or repo.exists():
        parser.error("fixture account is not fresh; preserve failed evidence and provision another account")
    repo.mkdir()
    env = dict(os.environ, GRIPSACK_HOME=str(state), XDG_CONFIG_HOME=str(home / ".config"))
    effects, checks = home / "effects", home / "checks"
    started, release = home / "started", home / "release"
    # An acquired external archive, not a captured repository input whose
    # deletion intentionally changes the current authored source snapshot.
    archive = None
    if args.package_bundle is None:
        archive = home / "tool.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            tar.add(binary, arcname="bin/grip", recursive=False)

    def command(*arguments, expected=0, approve=True, timeout=120):
        if approve and arguments[0] in {"check", "plan", "apply", "update", "task"}:
            inspection = command("trust", "inspect", str(repo), "--json", approve=False)
            facts = json.loads(inspection.stdout)
            if facts["approval"]["status"]["status"] != "approved":
                command("trust", "add", str(repo), "--bundle", facts["bundle_digest"],
                        "--policy", facts["policy_digest"], approve=False)
        result = subprocess.run([str(binary), *arguments], cwd=repo, env=env,
                                capture_output=True, text=True, timeout=timeout)
        if expected is not None and result.returncode != expected:
            raise AssertionError((arguments, result.returncode, result.stdout, result.stderr))
        return result

    def status():
        return json.loads(command("jobs", "status", "--json", approve=False).stdout)

    def selected():
        entries = status()["jobs"]
        assert len(entries) == 1, entries
        return entries[0]

    def write_workspace(label, minute, gated=False, installed=True):
        script = "from pathlib import Path; import time; "
        if gated:
            script += f"Path({str(started)!r}).write_text('admitted'); "
            script += f"deadline=time.monotonic()+90\nwhile not Path({str(release)!r}).exists():\n assert time.monotonic()<deadline\n time.sleep(0.02)\n"
        script += f"\nwith Path({str(effects)!r}).open('a') as f: f.write({label!r}+'\\n')"
        verify = f"from pathlib import Path; assert Path({str(effects)!r}).exists(); "
        verify += f"f=Path({str(checks)!r}).open('a'); f.write({label!r}+'\\n'); f.close()"
        def python(code):
            return "exec(lit(" + json.dumps(sys.executable) + ")).arg(lit('-c')).arg(lit(" + json.dumps(code) + ")).build()"
        text = '''import { defineWorkspace, workspace, pkg, provider, tarball,
          task, check, exec, lit, packageCommand, schedule, daily, profile, lock, source,
        } from "@gripsack/core";
        const shared=lock("store","fixture-mutation");
        export default defineWorkspace(() => workspace({outputs:[
          pkg("tool",{producer:PROVIDER,commands:{grip:SELECTOR},
            target:{os:"linux",arch:"x86_64",abi:"gnu"},layout:{kind:"relocatable"}}),
          check("verify",{subject:"tool",run:VERIFY}),
          task("once",{steps:[exec(packageCommand("tool","grip")).arg(lit("--version")).build(),RUN],checks:["verify"],locks:[shared]}),
          schedule("daily",{task:"once",trigger:daily(TIME)}),
          profile("personal",{schedules:SCHEDULES}),
        ]}));'''
        for key, value in {"PROVIDER":provider_expression, "SELECTOR":json.dumps(command_path),
                           "VERIFY":python(verify),
                           "RUN":python(script), "TIME":json.dumps(minute),
                           "SCHEDULES":'["daily"]' if installed else '[]'}.items():
            text = text.replace(key, value)
        (repo / "gripsack.ts").write_text(text)

    def wait(predicate, timeout=190):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.1)
        raise AssertionError("bounded fixture wait expired")

    capability = json.loads(command("jobs", "capability", "--json", approve=False).stdout)
    assert capability["status"] == "available", capability
    imported_identity = None
    imported_payload = None
    revocation_checks = 0
    policy_path = state / "sharing/trust.json"
    if args.package_bundle is not None:
        bundle = args.package_bundle.resolve(strict=True)
        if bundle == home or not bundle.is_relative_to(home):
            parser.error("transferred bundle must be inside this disposable home for explicit removal")
        policy_path.parent.mkdir(parents=True, mode=0o700)
        policy_path.write_bytes(args.package_trust.read_bytes())
        policy_path.chmod(0o600)
        admitted = json.loads(command("package", "import", "--bundle", str(bundle), approve=False).stdout)
        manifest_bytes = (bundle / "manifest.json").read_bytes()
        imported_identity = hashlib.sha256(manifest_bytes).hexdigest()
        assert admitted["manifest"] == imported_identity and admitted["layout"] == "relocatable"
        exported = json.loads(manifest_bytes)["commands"]["grip"]
        assert exported["args"] == []
        command_path = "components/" + exported["component"] + "/" + exported["executable"]
        imported_payload = Path(admitted["payload"])
        with (imported_payload / command_path).open("rb") as tool_bytes, binary.open("rb") as expected_bytes:
            assert hashlib.file_digest(tool_bytes, "sha256").digest() == hashlib.file_digest(expected_bytes, "sha256").digest()
        provider_expression = "provider(source.package({manifest:" + json.dumps(imported_identity) + "}))"
        shutil.rmtree(bundle)
        assert not bundle.exists()
        assert not (state / "current").exists()
    else:
        provider_expression = "provider(tarball(" + json.dumps(archive.as_uri()) + "))"
        command_path = "bin/grip"

    def refuse_revoked(*arguments, approve=True):
        nonlocal revocation_checks
        original_policy = policy_path.read_bytes()
        policy = json.loads(original_policy)
        policy["revoked_manifests"] = [*policy.get("revoked_manifests", []), imported_identity]
        before = (effects.read_bytes(), checks.read_bytes())
        policy_path.write_text(json.dumps(policy))
        try:
            refused = command(*arguments, expected=None, approve=approve)
            assert refused.returncode != 0, (arguments, refused.stdout, refused.stderr)
            assert (effects.read_bytes(), checks.read_bytes()) == before
            assert imported_payload.is_dir()
            revocation_checks += 1
        finally:
            policy_path.write_bytes(original_policy)

    # Future local calendar minute, not a clock manipulation or transient fake.
    minute = (datetime.datetime.now() + datetime.timedelta(minutes=2)).strftime("%H:%M")
    write_workspace("first", minute)
    command("update", "tool")
    manual = json.loads(command("task", "run", "once", "--json").stdout)
    assert manual["status"] == "succeeded"
    command("apply", "personal")
    first = selected()
    service = "gripsack-job-" + first["job"] + ".service"
    timer = service.removesuffix(".service") + ".timer"
    units = home / ".config/systemd/user"
    assert "Persistent=false" in (units / timer).read_text()
    assert "Restart=no" in (units / service).read_text()
    assert Path(first["capture"]).stat().st_mode & 0o077 == 0
    subprocess.run(["systemd-analyze", "--user", "verify", str(units / service), str(units / timer)],
                   env=env, check=True, timeout=30)
    # Installed tools must survive loss of acquisition material.
    if archive is not None:
        archive.unlink()
    wait(lambda: checks.exists() and len(checks.read_text().splitlines()) >= 2)
    assert effects.read_text().splitlines()[:2] == ["first", "first"]
    assert any(item["status"] == "succeeded" for item in status()["invocations"])
    (repo / "gripsack.ts").write_text("throw new Error('timer must not evaluate');")
    command("jobs", "dispatch", first["job"], first["revision"], "--json", approve=False)

    future = (datetime.datetime.now() + datetime.timedelta(minutes=20)).strftime("%H:%M")
    write_workspace("admitted-old", future, gated=True)
    command("apply", "personal")
    second = selected()
    assert second["revision"] != first["revision"]
    stale = command("jobs", "dispatch", first["job"], first["revision"], approve=False, expected=None)
    assert stale.returncode != 0
    running = subprocess.Popen([str(binary), "jobs", "dispatch", second["job"], second["revision"], "--json"],
                               cwd=repo, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        wait(started.exists, timeout=30)
        histories = status()["invocations"]
        assert any(item["root_retained"] for item in histories)
        if imported_identity is not None:
            refuse_revoked("build", "tool", "--json")
            assert running.poll() is None
        overlap = command("task", "run", "once", "--json", "--skip-busy", expected=75)
        assert json.loads(overlap.stdout)["status"] == "skipped-busy"
        write_workspace("replacement", future)
        command("apply", "personal")
        third = selected()
        assert third["revision"] != second["revision"]
        assert command("jobs", "dispatch", second["job"], second["revision"], approve=False, expected=None).returncode != 0
        command("jobs", "disable", second["job"], "--json", approve=False)
        assert status()["jobs"] == []
        assert command("jobs", "dispatch", second["job"], second["revision"], approve=False, expected=None).returncode != 0
        command("gc", approve=False)
        release.touch()
        stdout, stderr = running.communicate(timeout=30)
        assert running.returncode == 0, (stdout, stderr)
        report = json.loads(stdout)
        assert report["status"] == "succeeded" and not report["root_retained"]
        assert effects.read_text().splitlines()[-1] == "admitted-old"
        log = command("jobs", "log", report["invocation"], "--json", approve=False)
        assert json.loads(log.stdout)["schedule"]["revision"] == second["revision"]
    finally:
        release.touch()
        if running.poll() is None:
            running.terminate()
            running.communicate(timeout=10)
    command("rollback", approve=False)
    restored = selected()
    assert restored["revision"] == third["revision"]
    command("jobs", "dispatch", restored["job"], restored["revision"], "--json", approve=False)
    if imported_identity is not None:
        refuse_revoked("jobs", "dispatch", restored["job"], restored["revision"], "--json", approve=False)
        refuse_revoked("task", "run", "once", "--json")
    # Foreign drift is preserved and never becomes dispatcher authority.
    original = (units / timer).read_bytes()
    (units / timer).write_bytes(original + b"# fixture drift\n")
    assert command("jobs", "dispatch", restored["job"], restored["revision"], approve=False, expected=None).returncode != 0
    assert (units / timer).read_bytes().endswith(b"# fixture drift\n")
    (units / timer).write_bytes(original)
    write_workspace("removed", future, installed=False)
    command("apply", "personal")
    assert status()["jobs"] == []
    assert not (units / timer).exists() and not (units / service).exists()
    command("gc", approve=False)
    print(json.dumps({"status":"passed","capability":capability,"calendar":minute,
                      "manual_invocation":manual["invocation"],"old_revision_finished":report["invocation"],
                      "effects":effects.read_text().splitlines(),"checks":checks.read_text().splitlines(),
                      "imported_package":imported_identity,"revocation_checks":revocation_checks}))


if __name__ == "__main__":
    main()
