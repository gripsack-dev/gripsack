"""Actual check commands gate task invocations; grip check remains read-only."""
import json
import shlex

from conftest import grip


def command(script):
    return 'exec({argv: [' + ','.join(f'lit({json.dumps(arg)})' for arg in [
        "/bin/sh", "-c", script]) + ']})'


def declare(repo, effect, *, task_status=0, check_status=0):
    repo.mkdir(exist_ok=True)
    destination = shlex.quote(str(effect))
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, profile, task, check, exec, lit,
    } from "@gripsack/core";
    export default defineWorkspace(() => workspace({outputs: [
      profile("subject", {}),
      check("first", {subject: "subject", run: '''
        + command(f"printf 'check-one\\n' >> {destination}; exit {check_status}") + '''}),
      check("second", {subject: "subject", run: '''
        + command(f"printf 'check-two\\n' >> {destination}") + '''}),
      task("invoke", {steps: ['''
        + command(f"printf 'task\\n' >> {destination}; exit {task_status}")
        + '''], checks: ["first", "second"]}),
    ]}));\n''')


def test_checks_execute_after_successful_task_every_invocation(sandbox):
    repo, effect = sandbox / "checks", sandbox / "effects"
    declare(repo, effect)
    static = grip("check", cwd=repo)
    assert static.returncode == 0, static.stdout + static.stderr
    assert not effect.exists()
    for count in (1, 2):
        invoked = grip("task", "invoke", cwd=repo)
        assert invoked.returncode == 0, invoked.stdout + invoked.stderr
        assert effect.read_text().splitlines() == ["task", "check-one", "check-two"] * count
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_failed_task_does_not_run_postconditions(sandbox):
    repo, effect = sandbox / "checks", sandbox / "effects"
    declare(repo, effect, task_status=7)
    invoked = grip("task", "invoke", cwd=repo)
    assert invoked.returncode == 7, invoked.stdout + invoked.stderr
    assert effect.read_text().splitlines() == ["task"]


def test_failed_check_stops_later_checks_and_is_not_cached(sandbox):
    repo, effect = sandbox / "checks", sandbox / "effects"
    declare(repo, effect, check_status=9)
    for count in (1, 2):
        invoked = grip("task", "invoke", cwd=repo)
        assert invoked.returncode == 9, invoked.stdout + invoked.stderr
        assert effect.read_text().splitlines() == ["task", "check-one"] * count
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_task_check_source_path_binds_its_immutable_package_subject(sandbox):
    from conftest import make_tarball
    archive = make_tarball(sandbox / "data.tar.gz", {"share/value": b"required\n"})
    expected = sandbox / "expected"
    expected.write_bytes(b"required\n")
    repo = sandbox / "checks"
    repo.mkdir()
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, task, check, exec, lit, sourcePath, pkg, provider, tarball,
    } from "@gripsack/core";
    export default defineWorkspace(ctx => workspace({outputs: [
      pkg("data", {producer: provider(tarball(''' + json.dumps(archive.as_uri()) + ''')),
        commands: {}, target: {os: ctx.facts.os, arch: ctx.facts.arch},
        layout: {kind: "relocatable"}}),
      check("contents", {subject: "data", run: exec({argv: [
        lit("/usr/bin/cmp"), sourcePath("share/value"), lit(''' + json.dumps(str(expected)) + ''')
      ]})}),
      task("invoke", {steps: [exec({argv: [lit("/usr/bin/true")]})], checks: ["contents"]}),
    ]}));\n''')
    updated = grip("update", "data", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    passed = grip("task", "invoke", cwd=repo)
    assert passed.returncode == 0, passed.stdout + passed.stderr
    expected.write_bytes(b"different\n")
    failed = grip("task", "invoke", cwd=repo)
    assert failed.returncode == 1, failed.stdout + failed.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()
