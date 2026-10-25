"""E1 executable smoke: pytest -q e2e/test_task_graphs.py e2e/test_workspace_checks.py.
All effects, process IDs and store-scoped locks belong to disposable sandboxes.
"""
import fcntl
import json
import os
import pty
import shlex
import signal
import subprocess
import sys
import termios
import time

import pytest

from conftest import GRIP, grip, start_grip, make_tarball
from test_workspace_checks import command


def workspace(repo, outputs, prelude=""):
    repo.mkdir(exist_ok=True)
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, task, profile, check, environment,
      exec, lit, lock, ensureArtifact, pkg, provider, tarball,
    } from "@gripsack/core";
    ''' + prelude + '''
    export default defineWorkspace(ctx => workspace({outputs: [
    ''' + ",\n".join(outputs) + "]}));\n")


def task(name, script, *, deps=(), env=None, checks=(), locks=False):
    return f'task({json.dumps(name)}, {{steps:[{command(script)}], deps:{json.dumps(deps)}, checks:{json.dumps(checks)}' + (
        f', environment:{json.dumps(env)}' if env else "") + (",locks:[shared]" if locks else "") + "})"


def invoke(repo, name="root", *flags):
    result = grip("task", "run", name, "--json", *flags, cwd=repo)
    return result, json.loads(result.stdout)


def signal_command(termination):
    # Noninteractive shells can ignore QUIT; explicitly restore the fixture
    # command's default disposition, without changing the supervisor's.
    program = "import os, signal; "
    if termination != signal.SIGKILL:
        program += f"signal.signal({int(termination)}, signal.SIG_DFL); "
    program += f"os.kill(os.getpid(), {int(termination)})"
    return f"ulimit -c 0; exec {shlex.quote(sys.executable)} -c {shlex.quote(program)}"


def test_diamond_dedup_is_per_invocation_and_environment_bindings_are_distinct(sandbox):
    repo, effect = sandbox / "graph", sandbox / "effects"
    write = f'printf "%s\\n" "$VALUE" >> {shlex.quote(str(effect))}'
    workspace(repo, [
        'environment("one", {packages:[],target:{os:ctx.facts.os,arch:ctx.facts.arch},env:{VALUE:lit("one")}})',
        'environment("two", {packages:[],target:{os:ctx.facts.os,arch:ctx.facts.arch},env:{VALUE:lit("two")}})',
        task("shared", "printf 'shared\\n' >> " + shlex.quote(str(effect))),
        task("left", write, deps=["shared"], env="one"),
        task("right", write, deps=["shared"], env="two"),
        task("root", "printf 'root\\n' >> " + shlex.quote(str(effect)), deps=["right", "left"]),
    ])
    planned = grip("task", "plan", "root", "--json", cwd=repo)
    assert planned.returncode == 0, planned.stderr
    assert json.loads(planned.stdout)["order"] == ["shared", "left", "right", "root"]
    assert not effect.exists()
    occasions = []
    for count in (1, 2):
        result, report = invoke(repo)
        assert result.returncode == 0, result.stderr
        assert report["status"] == "succeeded"
        assert effect.read_text().splitlines() == ["shared", "one", "two", "root"] * count
        assert [node["task"] for node in report["nodes"]] == ["shared", "left", "right", "root"]
        occasions.append(report)
    assert occasions[0]["invocation"] != occasions[1]["invocation"]
    assert occasions[0]["attempt"] != occasions[1]["attempt"]
    assert occasions[0]["revision"] == occasions[1]["revision"]
    status = grip("task", "status", occasions[0]["invocation"], "--json", cwd=repo)
    assert json.loads(status.stdout) == occasions[0]


@pytest.mark.parametrize("verification", [False, True])
def test_failed_prerequisite_or_verification_blocks_successors(sandbox, verification):
    repo, effect = sandbox / "graph", sandbox / "effects"
    workspace(repo, [
        'profile("subject", {})',
        'check("verify", {subject:"subject",run:' + command("exit 9") + '})',
        task("before", f"printf 'before\\n' >> {shlex.quote(str(effect))}; exit " + ("0" if verification else "7"),
             checks=["verify"] if verification else []),
        task("root", f"printf 'forbidden\\n' >> {shlex.quote(str(effect))}", deps=["before"]),
    ])
    result, report = invoke(repo)
    assert result.returncode != 0
    assert report["status"] == "blocked"
    assert [node["status"] for node in report["nodes"]] == ["failed", "blocked"]
    assert effect.read_text() == "before\n"
    assert not report["root_retained"]


@pytest.mark.parametrize("termination", [signal.SIGSEGV, signal.SIGKILL])
def test_child_crash_fails_without_crashing_cli_or_cancelling_independent_work(sandbox, termination):
    repo, effect = sandbox / "graph", sandbox / "effects"
    crash = signal_command(termination)
    workspace(repo, [
        task("crash", crash),
        task("dependent", "echo forbidden-dependent >> " + shlex.quote(str(effect)), deps=["crash"]),
        task("independent", "echo independent >> " + shlex.quote(str(effect))),
        task("root", "echo forbidden-root >> " + shlex.quote(str(effect)), deps=["dependent", "independent"]),
    ])
    direct, direct_report = invoke(repo, "crash")
    assert direct.returncode == 128 + termination, direct.stderr
    assert direct_report["status"] == "failed"
    receipt = direct_report["nodes"][0]["processes"][0]
    assert receipt["disposition"] == "exited"
    assert receipt["signal"] == termination and receipt["exit_code"] is None
    assert direct_report["nodes"][0]["completed_steps"] == 0
    assert not direct_report["root_retained"]
    saved = grip("task", "status", direct_report["invocation"], "--json", cwd=repo)
    assert saved.returncode == 1
    assert json.loads(saved.stdout) == direct_report

    result, report = invoke(repo)
    assert result.returncode == 3, result.stderr
    assert report["status"] == "blocked"
    assert [(node["task"], node["status"]) for node in report["nodes"]] == [
        ("crash", "failed"), ("dependent", "blocked"),
        ("independent", "succeeded"), ("root", "blocked"),
    ]
    assert report["nodes"][0]["processes"][0]["signal"] == termination
    assert effect.read_text() == "independent\n"
    assert not report["root_retained"]


@pytest.mark.parametrize("termination", [signal.SIGHUP, signal.SIGINT, signal.SIGQUIT, signal.SIGTERM])
def test_direct_child_control_signal_cancels_graph(sandbox, termination):
    repo, forbidden = sandbox / "graph", sandbox / "forbidden"
    workspace(repo, [
        task("before", signal_command(termination)),
        task("independent", "echo forbidden >> " + shlex.quote(str(forbidden))),
        task("root", "echo forbidden >> " + shlex.quote(str(forbidden)), deps=["before", "independent"]),
    ])
    result, report = invoke(repo)
    assert result.returncode == 128 + termination, result.stderr
    assert report["status"] == "cancelled"
    assert [node["status"] for node in report["nodes"]] == ["cancelled", "blocked", "blocked"]
    receipt = report["nodes"][0]["processes"][0]
    assert receipt["disposition"] == "exited"
    assert receipt["signal"] == termination and receipt["exit_code"] is None
    assert not forbidden.exists()
    assert not report["root_retained"]


def test_ordered_steps_stop_before_later_steps_and_postconditions(sandbox):
    repo, effect = sandbox / "graph", sandbox / "effects"
    destination = shlex.quote(str(effect))
    workspace(repo, [
        'profile("subject", {})',
        'check("verify", {subject:"subject",run:' + command(f"echo forbidden-check >> {destination}") + '})',
        'task("root", {steps:[' + ",".join([
            command(f"echo first >> {destination}"), command("exit 8"),
            command(f"echo forbidden-step >> {destination}")]) + '],checks:["verify"]})',
    ])
    result, report = invoke(repo)
    assert result.returncode == 8
    assert effect.read_text() == "first\n"
    assert report["nodes"][0]["completed_steps"] == 1
    assert report["nodes"][0]["completed_checks"] == 0


def test_ensure_artifact_only_uses_common_reusable_store_without_fake_process(sandbox):
    repo = sandbox / "graph"
    archive = make_tarball(sandbox / "data.tar.gz", {"value": b"immutable\n"})
    workspace(repo, [
        'pkg("data", {producer:provider(tarball(' + json.dumps(archive.as_uri()) + ''')),commands:{},
        target:{os:ctx.facts.os,arch:ctx.facts.arch},layout:{kind:"relocatable"}})''',
        'task("root", {steps:[ensureArtifact("data")]})',
    ])
    updated = grip("update", "data", cwd=repo)
    assert updated.returncode == 0, updated.stderr
    first, first_report = invoke(repo)
    assert first.returncode == 0, first.stderr
    archive.unlink()  # warm invocation must not reacquire the vanished provider.
    second, second_report = invoke(repo)
    assert second.returncode == 0, second.stderr
    for report in (first_report, second_report):
        assert report["nodes"][0]["processes"] == []
        assert report["nodes"][0]["completed_steps"] == 1
        assert report["status"] == "succeeded" and not report["root_retained"]


def wait_file(path, process):
    deadline = time.monotonic() + 30
    while (not path.exists() or path.stat().st_size == 0) and time.monotonic() < deadline:
        assert process.poll() is None, process.communicate()
        time.sleep(0.025)
    assert path.exists() and path.stat().st_size > 0, "task did not reach bounded fixture barrier"


def locked_workspace(repo, marker, release):
    workspace(repo, [task("root", f"echo $$ > {shlex.quote(str(marker))}; "
        f"while test ! -e {shlex.quote(str(release))}; do sleep 0.05; done", locks=True)],
        'const shared = lock("store", "manual-overlap");')


def test_concurrent_manual_callers_skip_busy_then_new_run_executes(sandbox):
    repo, marker, release = sandbox / "graph", sandbox / "started", sandbox / "release"
    locked_workspace(repo, marker, release)
    first = start_grip([str(GRIP.resolve()), "task", "run", "root", "--json"], cwd=repo,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        wait_file(marker, first)
        changed = sandbox / "new-revision-effect"
        workspace(repo, [task("root", "echo changed > " + shlex.quote(str(changed)), locks=True)],
                  'const shared = lock("store", "manual-overlap");')
        second, report = invoke(repo, "root", "--skip-busy")
        assert second.returncode == 75, second.stderr
        assert report["status"] == "skipped-busy" and report["nodes"] == []
        assert not changed.exists()
        release.touch()
        stdout, stderr = first.communicate(timeout=30)
        assert first.returncode == 0, stderr
        completed = json.loads(stdout)
        assert completed["status"] == "succeeded"
        assert completed["revision"] != report["revision"]
        third, report = invoke(repo)
        assert third.returncode == 0 and report["status"] == "succeeded", third.stderr
        assert changed.read_text() == "changed\n"
    finally:
        release.touch()
        if first.poll() is None:
            first.terminate()
            first.communicate(timeout=10)


def test_supervisor_death_keeps_unknown_claim_after_child_cleanup(sandbox):
    repo, marker, release = sandbox / "graph", sandbox / "started", sandbox / "release"
    locked_workspace(repo, marker, release)
    first = start_grip([str(GRIP.resolve()), "task", "run", "root", "--json"], cwd=repo,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    child = None
    try:
        wait_file(marker, first)
        child = int(marker.read_text())
        first.kill()
        first.wait(timeout=10)
        # Cooperating child may still hold its process-root descriptor. Even
        # after explicit fixture cleanup, absence alone cannot prove outcome.
        os.killpg(child, signal.SIGKILL)
        child = None
        result, report = invoke(repo, "root", "--skip-busy")
        assert result.returncode == 70, result.stderr
        assert report["status"] == "indeterminate" and report["nodes"] == []
        histories = sandbox / ".local/share/gripsack/task-invocations"
        records = [json.loads(path.read_text()) for path in histories.glob("*.json")]
        assert any(record["root_retained"] and record["status"] == "indeterminate" for record in records)
    finally:
        release.touch()
        if child is not None:
            try:
                os.killpg(child, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if first.poll() is None:
            first.kill()
        first.communicate(timeout=10)


def test_deadline_includes_postcondition_and_closed_stdin(sandbox):
    repo = sandbox / "graph"
    workspace(repo, [
        'profile("subject", {})',
        'check("slow", {subject:"subject",run:' + command("sleep 30") + '})',
        task("root", 'test -z "$(cat)"', checks=["slow"]),
    ])
    result, report = invoke(repo, "root", "--closed-stdin", "--timeout-seconds", "5")
    assert result.returncode == 1, result.stderr
    assert report["status"] == "failed"
    assert report["nodes"][0]["completed_steps"] == 1
    assert report["nodes"][0]["completed_checks"] == 0
    assert report["nodes"][0]["processes"][-1]["disposition"] == "deadline"


@pytest.mark.parametrize("termination", [signal.SIGHUP, signal.SIGINT, signal.SIGQUIT, signal.SIGTERM])
def test_cancellation_blocks_dependents_and_releases_confirmed_claims(sandbox, termination):
    repo, marker, forbidden = sandbox / "graph", sandbox / "started", sandbox / "forbidden"
    workspace(repo, [
        # Ignoring the forwarded signal forces bounded SIGKILL cleanup. The
        # supervisor disposition, not that cleanup signal, owns cancellation.
        task("before", f"trap '' HUP INT QUIT TERM; echo $$ > {shlex.quote(str(marker))}; sleep 30", locks=True),
        task("independent", f"echo forbidden > {shlex.quote(str(forbidden))}"),
        task("root", f"echo forbidden > {shlex.quote(str(forbidden))}", deps=["before", "independent"]),
    ], 'const shared = lock("store", "cancelled-invocation");')
    process = start_grip([str(GRIP.resolve()), "task", "run", "root", "--json"], cwd=repo,
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        wait_file(marker, process)
        process.send_signal(termination)
        stdout, stderr = process.communicate(timeout=15)
        assert process.returncode == 130, stderr
        report = json.loads(stdout)
        assert report["status"] == "cancelled"
        assert [node["status"] for node in report["nodes"]] == ["cancelled", "blocked", "blocked"]
        receipt = report["nodes"][0]["processes"][0]
        assert receipt["disposition"] == "cancelled"
        assert receipt["signal"] == signal.SIGKILL and receipt["exit_code"] is None
        saved = grip("task", "status", report["invocation"], "--json", cwd=repo)
        assert saved.returncode == 130
        assert json.loads(saved.stdout) == report
        assert not forbidden.exists()
        assert not report["root_retained"]
        claims = sandbox / ".local/share/gripsack/task-mutation-locks"
        assert list(claims.glob("*.json")) == []
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate(timeout=10)


@pytest.mark.parametrize("termination,control", [(signal.SIGINT, b"\x03"), (signal.SIGQUIT, b"\x1c")])
def test_foreground_terminal_signal_cancels_without_supervisor_signal(sandbox, termination, control):
    repo, marker, forbidden = sandbox / "graph", sandbox / "started", sandbox / "forbidden"
    program = (
        "import os, pathlib, signal; "
        "signal.signal(signal.SIGINT, signal.SIG_DFL); "
        "signal.signal(signal.SIGQUIT, signal.SIG_DFL); "
        f"pathlib.Path({str(marker)!r}).write_text(str(os.getpid())); "
        "signal.pause()"
    )
    workspace(repo, [
        task("before", f"ulimit -c 0; exec {shlex.quote(sys.executable)} -c {shlex.quote(program)}"),
        task("independent", "echo forbidden > " + shlex.quote(str(forbidden))),
        task("root", "echo forbidden > " + shlex.quote(str(forbidden)), deps=["before", "independent"]),
    ])
    master, slave = pty.openpty()

    def claim_terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    process = None
    try:
        process = start_grip([str(GRIP.resolve()), "task", "run", "root", "--json"],
                             cwd=repo, stdin=slave, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, preexec_fn=claim_terminal)
        wait_file(marker, process)
        child = int(marker.read_text())
        deadline = time.monotonic() + 5
        while os.tcgetpgrp(master) != child and time.monotonic() < deadline:
            assert process.poll() is None
            time.sleep(0.025)
        assert os.tcgetpgrp(master) == child
        os.write(master, control)
        stdout, stderr = process.communicate(timeout=15)
        assert process.returncode == 128 + termination, stderr
        report = json.loads(stdout)
        assert report["status"] == "cancelled"
        assert [node["status"] for node in report["nodes"]] == ["cancelled", "blocked", "blocked"]
        receipt = report["nodes"][0]["processes"][0]
        assert receipt["disposition"] == "exited"
        assert receipt["signal"] == termination and receipt["exit_code"] is None
        assert not report["root_retained"]
        assert not forbidden.exists()
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.communicate(timeout=10)
        os.close(master)
        os.close(slave)


def test_task_log_preserves_lines_without_replaying_terminal_controls(sandbox):
    repo = sandbox / "graph"
    workspace(repo, [task("root", r"printf 'first\n\033[2Jsecond\n\377\n'")])
    result, report = invoke(repo)
    assert result.returncode == 0, result.stderr
    log = grip("task", "log", report["invocation"], cwd=repo)
    assert log.returncode == 0, log.stderr
    assert "first\n" in log.stdout
    assert "second\n" in log.stdout
    assert "\x1b" not in log.stdout
    assert "\\xff\n" in log.stdout
