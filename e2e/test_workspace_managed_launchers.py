"""Managed command entries: exact activation, retained admission and lifecycle."""
import json
import platform
import shutil
import subprocess
from pathlib import Path

import pytest

import conftest
from conftest import grip
from test_workspace_gnu_loader import shell_payload

pytestmark = pytest.mark.skipif(
    platform.system() != "Linux" or platform.machine() != "x86_64",
    reason="retained Linux/x86_64 GNU fixture",
)


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def declare(sandbox, *, value="spaces '$HOME' ; $(touch forbidden)", commands=None,
            host_runtime=None, files=None):
    repo = sandbox / "managed-workspace"
    repo.mkdir(exist_ok=True)
    payload = repo / "payload"
    if not payload.exists():
        for name, data in (shell_payload() if files is None else files).items():
            path = payload / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            path.chmod(0o755 if name.startswith("bin/") else 0o644)
    destination = sandbox / "editor's commands; literal"
    declaration = {
        "directory": str(destination),
        "commands": ["probe"] if commands is None else commands,
    }
    runtime = "" if host_runtime is None else ",hostRuntime:" + json.dumps(host_runtime)
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, pkg, provider, fileFetch, environment, profile, lit,
    } from "@gripsack/core";
    export default defineWorkspace(() => {
      const target = {os:"linux",arch:"x86_64",abi:"gnu"} as const;
      return workspace({outputs:[
        pkg("tools",{producer:provider(fileFetch("payload")),
          commands:{probe:"bin/probe",peer:"bin/probe"},target,
          layout:{kind:"relocatable"}''' + runtime + '''}),
        environment("dev",{packages:["tools"],target,env:{
          APP_CONFIG:lit(''' + json.dumps(value) + '''),EMPTY:lit("")
        }}),
        profile("personal",{environment:"dev",launchers:''' + json.dumps(declaration) + '''}),
      ]});
    });
    ''')
    return repo, destination / "probe"


def launch(path, *arguments, input=None, cwd=None, env=None):
    return subprocess.run([str(path), *arguments], input=input, cwd=cwd, env={} if env is None else env,
                          capture_output=True, text=True, timeout=120)


def test_clean_editor_launch_preserves_exact_values_args_cwd_stdin_exit_and_nested_exports(sandbox):
    literal = "spaces '$HOME' ; $(touch forbidden)\nnext line"
    repo, command = declare(sandbox, value=literal)
    run(repo, "apply")
    assert command.stat().st_mode & 0o111
    assert not command.is_symlink(), "managed entries use tracked-copy drift ownership"
    result = launch(command, "-c",
                    'printf "%s|%s|%s|%s|%s|%s\\n" "$APP_CONFIG" "${EMPTY+x}" '
                    '"${UNDECLARED-unset}" "$PWD" "$1" "$2"; '
                    'IFS= read -r line; printf "input=%s\\n" "$line"; '
                    'exec peer -c \'printf "nested=%s" "$APP_CONFIG"\'',
                    "selected shell argv0", "", "two words ' ; $HOME", input="untouched stdin\n", cwd=sandbox,
                    env={"APP_CONFIG": "ambient must not override", "UNDECLARED": "not granted"})
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout == (
        f"{literal}|x|unset|{sandbox}||two words ' ; $HOME\n"
        f"input=untouched stdin\nnested={literal}"
    )
    assert not (sandbox / "forbidden").exists()
    alias = launch(command, "-c", 'printf "%s" "$0"')
    assert alias.returncode == 0, alias.stderr
    assert alias.stdout.endswith("/bin/probe"), "public alias must preserve the selected multicall argv0"
    assert launch(command, "-c", "exit 7").returncode == 7
    assert launch(command, "-c", "kill -TERM $$").returncode in (-15, 143)


@pytest.mark.parametrize("commands", [["probe", "probe"], ["missing"]])
def test_invalid_exports_do_not_mutate_destination_or_generation(sandbox, commands):
    repo, command = declare(sandbox, commands=commands)
    result = grip("apply", cwd=repo)
    assert result.returncode != 0
    assert not command.parent.exists()
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_changed_activation_rollback_drift_and_gc(sandbox):
    repo, command = declare(sandbox, value="first")
    run(repo, "apply")
    state = sandbox / ".local/share/gripsack"
    first = json.loads((state / "current/manifest.json").read_text())["number"]
    original = command.read_bytes()
    declare(sandbox, value="second")
    run(repo, "apply")
    assert launch(command, "-c", 'printf "%s" "$APP_CONFIG"').stdout == "second"
    run(repo, "gc")
    run(repo, "rollback", str(first))
    result = launch(command, "-c", 'printf "%s" "$APP_CONFIG"')
    assert result.returncode == 0, result.stderr
    assert result.stdout == "first"
    assert command.read_bytes() == original
    command.write_bytes(original + b"# user drift\n")
    run(repo, "apply")
    assert command.read_bytes() == original + b"# user drift\n"
    assert launch(command, "-c", 'printf "%s" "$APP_CONFIG"').stdout == "first"
    run(repo, "apply", "--take-over")
    assert launch(command, "-c", 'printf "%s" "$APP_CONFIG"').stdout == "second"


def test_process_lease_roots_all_profile_packages_through_gc(sandbox):
    repo, command = declare(sandbox)
    run(repo, "apply")
    state = sandbox / ".local/share/gripsack"
    process = subprocess.Popen([str(command), "-c", 'printf "ready\\n"; IFS= read -r line; printf "%s" "$line"'],
                               env={}, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True)
    try:
        assert process.stdout.readline() == "ready\n"
        roots = list((state / "roots/processes").glob("*.json"))
        assert roots, "policy-free packages also need live process retention"
        paths = {path for root in roots for path in json.loads(root.read_text())["store_paths"]}
        assert any("workspace-package" in path for path in paths)
        run(repo, "gc")
        assert all(Path(path).exists() for path in paths)
        stdout, stderr = process.communicate("survived\n", timeout=120)
        assert process.returncode == 0, stderr
        assert stdout == "survived"
        assert not list((state / "roots/processes").glob("*.json"))
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate(timeout=120)


def test_moved_core_refuses_path_fallback_and_reapply_repairs_binding(sandbox, monkeypatch):
    core = sandbox / "installed core's directory" / "grip"
    core.parent.mkdir()
    shutil.copy2(conftest.GRIP.resolve(), core)
    monkeypatch.setattr(conftest, "GRIP", core)
    repo, command = declare(sandbox)
    run(repo, "apply")
    relocated = core.with_name("relocated-grip")
    core.rename(relocated)
    shadow = sandbox / "shadow"
    shadow.mkdir()
    (shadow / "grip").write_text("#!/bin/sh\nprintf forbidden-fallback\n")
    (shadow / "grip").chmod(0o755)
    result = launch(command, "-c", "printf forbidden-command", env={"PATH": str(shadow)})
    assert result.returncode == 127
    assert result.stdout == ""
    assert "reapply" in result.stderr and str(core) in result.stderr
    monkeypatch.setattr(conftest, "GRIP", relocated)
    run(repo, "apply")
    repaired = launch(command, "-c", "printf repaired")
    assert repaired.returncode == 0, repaired.stderr
    assert repaired.stdout == "repaired"


def test_changed_host_runtime_is_readmitted_before_every_launch(sandbox):
    libraries = sandbox / "reviewed host runtime"
    libraries.mkdir()
    library = libraries / "libreview.so"
    source = sandbox / "probe.c"
    source.write_text("int reviewed(void) { return 42; }\n")
    subprocess.run(["/usr/bin/cc", "-shared", "-fPIC", "-Wl,-soname,libreview.so",
                    str(source), "-o", str(library)], check=True, capture_output=True)
    source.write_text('#include <stdio.h>\nextern int reviewed(void);\nint main(void) { printf("%d", reviewed()); }\n')
    executable = sandbox / "probe"
    subprocess.run(["/usr/bin/cc", str(source), str(library), f"-Wl,-rpath,{libraries}",
                    "-o", str(executable)], check=True, capture_output=True)
    repo, command = declare(sandbox, files={"bin/probe": executable.read_bytes()},
                            host_runtime={"libraryDirectories": [str(libraries)]})
    run(repo, "apply")
    admitted = launch(command)
    assert admitted.returncode == 0, admitted.stderr
    assert admitted.stdout == "42"
    library.write_bytes(b"not an ELF dependency anymore")
    refused = launch(command)
    assert refused.returncode != 0
    assert refused.stdout == "", "the target must not execute after mutable host admission fails"


def test_tampered_retained_payload_and_projection_fail_closed(sandbox):
    repo, command = declare(sandbox)
    run(repo, "apply")
    original = command.read_text()
    command.write_text(original.replace('"APP_CONFIG"', '"UNDECLARED"', 1))
    projection = launch(command, "-c", "printf forbidden")
    assert projection.returncode != 0 and projection.stdout == ""
    command.write_text(original)
    selected = launch(command, "-c", 'printf "%s" "$0"')
    assert selected.returncode == 0, selected.stderr
    retained = Path(selected.stdout)
    retained.chmod(0o755)
    retained.write_bytes(b"tampered retained executable")
    payload = launch(command, "-c", "printf forbidden")
    assert payload.returncode != 0 and payload.stdout == ""


def test_managed_launch_restores_policy_free_runtime_graph_and_other_environment_exports(sandbox):
    repo = sandbox / "split-runtime-workspace"
    repo.mkdir()
    for name, data in shell_payload().items():
        package = "program" if name.startswith("bin/") else "runtime"
        path = repo / package / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755 if name.startswith("bin/") else 0o644)
    destination = sandbox / "split-bin"
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, pkg, provider, fileFetch, environment, profile,
    } from "@gripsack/core";
    export default defineWorkspace(() => {
      const target = {os:"linux",arch:"x86_64",abi:"gnu"} as const;
      const layout = {kind:"relocatable"} as const;
      return workspace({outputs:[
        pkg("runtime",{producer:provider(fileFetch("runtime")),commands:{},target,layout}),
        pkg("tools",{producer:provider(fileFetch("program")),commands:{probe:"bin/probe"},
          runtime:["runtime"],target,layout}),
        pkg("companion",{producer:provider(fileFetch("program")),commands:{peer:"bin/probe"},
          runtime:["runtime"],target,layout}),
        environment("dev",{packages:["tools","companion"],target}),
        profile("personal",{environment:"dev",launchers:{
          directory:''' + json.dumps(str(destination)) + ''',commands:["probe"]}}),
      ]});
    });
    ''')
    run(repo, "apply")
    run(repo, "gc")
    command = destination / "probe"
    result = launch(command, "-c", 'exec peer -c "printf retained-runtime-graph"')
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout == "retained-runtime-graph"
    assert not (destination / "peer").exists(), "only requested commands become public entries"
