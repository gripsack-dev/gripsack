"""Portable pins and native publication, without a builder or generation."""
import json
from pathlib import Path

from conftest import grip, make_tarball, make_toolchain_tarball


def test_native_provider_survey_and_frozen_acquisition(sandbox, monkeypatch):
    archive = make_tarball(sandbox / "package.tar.gz", {"share/value": b"first\n"})
    repo = sandbox / "workspace"
    repo.mkdir()
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, pkg, provider, tarball,
    } from "@gripsack/core";
    export default defineWorkspace(() => workspace({ outputs: [pkg("data", {
      producer: provider(tarball(''' + json.dumps(archive.as_uri()) + ''')),
      commands: {}, target: {os:"linux",arch:"x86_64"}, layout: {kind:"relocatable"},
    })] }));
    ''')
    state = sandbox / ".local/share/gripsack"
    lock = repo / "gripsack.lock"

    def no_publication(home):
        store = home / "store"
        assert not store.exists() or not list(store.iterdir())
        assert not (home / "buildkit").exists()
        assert not (home / "current").exists()

    survey = grip("update", "--check", cwd=repo)
    assert survey.returncode == 1, survey.stdout + survey.stderr
    assert not lock.exists()
    no_publication(state)
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    first_lock = lock.read_bytes()
    first = grip("build", "data", "--json", cwd=repo)
    assert first.returncode == 0, first.stdout + first.stderr
    retained = Path(json.loads(first.stdout)["outputs"][0]["path"])
    assert (retained / "share/value").read_bytes() == b"first\n"
    assert lock.read_bytes() == first_lock
    assert not (state / "buildkit").exists()
    assert not (state / "current").exists()

    make_tarball(archive, {"share/value": b"changed at the same URL\n"})
    warm = grip("build", "data", "--json", cwd=repo)
    assert warm.returncode == 0, warm.stdout + warm.stderr
    assert json.loads(warm.stdout) == json.loads(first.stdout)
    cold_state = sandbox / "cold-state"
    monkeypatch.setenv("GRIPSACK_HOME", str(cold_state))
    refused = grip("build", "data", "--json", cwd=repo)
    assert refused.returncode != 0
    rejected = json.loads(refused.stdout)
    assert rejected["ok"] is False
    assert any(diagnostic["code"] == "E201" for diagnostic in rejected["diagnostics"])
    no_publication(cold_state)
    assert lock.read_bytes() == first_lock
    survey = grip("update", "--check", cwd=repo)
    assert survey.returncode == 1, survey.stdout + survey.stderr
    no_publication(cold_state)
    assert lock.read_bytes() == first_lock
    updated = grip("update", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    second = grip("build", "data", "--json", cwd=repo)
    assert second.returncode == 0, second.stdout + second.stderr
    current = Path(json.loads(second.stdout)["outputs"][0]["path"])
    assert (current / "share/value").read_bytes() == b"changed at the same URL\n"
    assert (retained / "share/value").read_bytes() == b"first\n"
    assert not (cold_state / "buildkit").exists()
    assert not (cold_state / "current").exists()


def test_selected_output_roots_do_not_retain_an_unrelated_old_package(sandbox):
    repo = sandbox / "separate-selections"
    repo.mkdir()
    for name in ("left", "right"):
        (repo / name).mkdir()
        (repo / name / "value").write_text(name + " original\n")
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, pkg, provider, fileFetch,
    } from "@gripsack/core";
    export default defineWorkspace(() => workspace({ outputs: ["left", "right"].map(name =>
      pkg(name, {producer:provider(fileFetch(name)), commands:{},
        target:{os:"linux",arch:"x86_64"}, layout:{kind:"relocatable"}})
    ) }));
    ''')

    def build(*names):
        result = grip("build", *names, "--json", cwd=repo)
        assert result.returncode == 0, result.stdout + result.stderr
        return {output["name"]: Path(output["path"]) for output in json.loads(result.stdout)["outputs"]}

    original = build("left", "right")
    (repo / "left/value").write_text("left changed\n")
    current = build("left")
    collected = grip("gc", cwd=repo)
    assert collected.returncode == 0, collected.stdout + collected.stderr
    assert not original["left"].exists(), "right's selection retained unrelated old left bytes"
    assert (original["right"] / "value").read_text() == "right original\n"
    assert (current["left"] / "value").read_text() == "left changed\n"


def test_task_declared_path_cannot_shadow_selected_environment_commands(sandbox):
    archive = make_toolchain_tarball(
        sandbox / "commands.tar.gz", {"bin/probe": b"#!/bin/sh\nprintf admitted-probe\n"}
    )
    shadow = sandbox / "shadow"
    shadow.mkdir()
    (shadow / "probe").write_text("#!/bin/sh\nprintf shadow-probe\n")
    (shadow / "probe").chmod(0o755)
    repo = sandbox / "task-path"
    repo.mkdir()
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, pkg, provider, tarball, environment, task, exec, lit,
    } from "@gripsack/core";
    export default defineWorkspace((ctx) => {
      const target = {os: ctx.facts.os, arch: ctx.facts.arch};
      return workspace({outputs: [
        pkg("tools", {producer:provider(tarball(''' + json.dumps(archive.as_uri()) + ''')),
          commands:{probe:"bin/probe"},target,layout:{kind:"relocatable"}}),
        environment("dev", {packages:["tools"],target}),
        task("nested", {environment:"dev",steps:[exec({
          argv:[lit("/bin/sh"),lit("-c"),lit("exec probe")],
          env:{PATH:lit(''' + json.dumps(str(shadow)) + ''')}
        })]})
      ]});
    });
    ''')
    updated = grip("update", "tools", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    result = grip("task", "nested", cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout == "admitted-probe"
