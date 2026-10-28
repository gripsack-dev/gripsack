"""Current workspace admission and native file execution through grip.

Direct IR9 cases exercise strict decoding alongside the real frontend.
Invalid declarations refuse before effects; native profiles deploy real files.
"""

from __future__ import annotations

import copy
import json
import shutil
from pathlib import Path

import pytest

from conftest import grip


def workspace_ir(*outputs):
    return {
        "ir_version": 9,
        "host": {"os": "linux", "arch": "x86_64", "tags": []},
        "workspace": {
            "span": {"file": "gripsack.ts", "line": 1},
            "outputs": list(outputs),
        },
    }


def dotfile_profile():
    return {
        "kind": "profile",
        "name": "dotfiles",
        "span": {"file": "gripsack.ts", "line": 9},
        "files": [
            {
                "span": {"file": "gripsack.ts", "line": 10},
                "content": {"kind": "literal", "text": "setting=1\n"},
                "destination": {"kind": "tracked_copy", "path": "~/.config/demo/settings.conf"},
            }
        ],
    }


def run_plan_ir(sandbox, doc):
    path = sandbox / "workspace.ir.json"
    path.write_text(json.dumps(doc))
    return grip("plan", "--ir", str(path))


def test_project_native_files_plan_without_effects_and_apply(sandbox):
    repo = sandbox / "project"
    repo.mkdir()
    (repo / "gripsack.ts").write_text(
        'import { defineWorkspace, workspace, profile, file, literalText, trackedCopyTo } '
        'from "@gripsack/core";\n'
        'export default defineWorkspace(() => workspace({ outputs: [\n'
        '  profile("dotfiles", { files: [file({\n'
        '    content: literalText("setting=1\\n"),\n'
        '    destination: trackedCopyTo("~/.config/demo/settings.conf"),\n'
        '  })] }),\n'
        '] }));\n'
    )
    assert not (repo / "env.toml").exists()
    assert not (repo / "hosts").exists()
    checked = grip("check", cwd=repo)
    assert checked.returncode == 0, checked.stdout + checked.stderr
    assert "dotfiles (profile)" in checked.stdout
    for command, expected in [(("plan",), 0), (("update", "--check"), 1)]:
        result = grip(*command, cwd=repo)
        assert result.returncode == expected, result.stdout + result.stderr
        assert not (sandbox / ".config/demo/settings.conf").exists()
        assert not (sandbox / ".local/share/gripsack/current").exists()
        assert not (repo / "gripsack.lock").exists()
    applied = grip("apply", cwd=repo)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    assert (sandbox / ".config/demo/settings.conf").read_text() == "setting=1\n"
    assert (sandbox / ".local/share/gripsack/current").exists()

def test_native_catalog_admits_from_one_offline_workspace(sandbox):
    fixture = Path(__file__).parent / "fixtures" / "envs" / "native-workspace"
    repo = sandbox / "native-workspace"
    shutil.copytree(fixture, repo)
    checked = grip("check", "--json", cwd=repo)
    assert checked.returncode == 0, checked.stdout + checked.stderr
    document = json.loads(checked.stdout)
    assert document["ok"] is True
    assert {output["kind"] for output in document["outputs"]} == {
        "package", "environment", "task", "profile",
    }
    planned = grip("plan", cwd=repo)
    assert planned.returncode == 0, planned.stdout + planned.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


@pytest.mark.parametrize(
    ("example", "outputs"),
    [
        pytest.param("01-dotfiles", [("dotfiles", "profile")], id="dotfiles-only"),
        pytest.param(
            "02-native-tool",
            [("formatter", "package"), ("development", "environment"), ("personal", "profile")],
            id="native-tool-and-profile",
        ),
        pytest.param(
            "03-source-built",
            [("build-greeter", "recipe"), ("greeter", "package"), ("dev", "environment"),
             ("smoke", "task")],
            id="source-recipe-and-command-consumer",
        ),
        pytest.param(
            "04-scheduled-task",
            [("formatter", "package"), ("dev", "environment"), ("format-now", "task"),
             ("format-nightly", "task"), ("nightly", "schedule"), ("personal", "profile")],
            id="manual-and-scheduled-task",
        ),
    ],
)
def test_graduated_examples_validate_without_realization(sandbox, example, outputs):
    source = Path(__file__).parent.parent / "examples" / "workspaces" / example
    repo = sandbox / example
    shutil.copytree(source, repo)
    checked = grip("check", "--json", cwd=repo)
    assert checked.returncode == 0, checked.stdout + checked.stderr
    document = json.loads(checked.stdout)
    assert document["ok"] is True
    assert {(output["name"], output["kind"]) for output in document["outputs"]} == set(outputs)
    if example in {"01-dotfiles", "02-native-tool"}:
        planned = grip("plan", cwd=repo)
        assert planned.returncode == 0, planned.stdout + planned.stderr
        assert not (sandbox / ".config/editor/config.toml").exists()
        assert not (sandbox / ".config/formatter/config.toml").exists()
        assert not (sandbox / ".local/share/gripsack/store").exists()
        assert not (sandbox / ".local/share/gripsack/buildkit").exists()
    else:
        # Catalog-only planning and schedule registration remain separate
        # unavailable operations, not evidence that packages cannot realize.
        planned = grip("plan", cwd=repo)
        assert planned.returncode != 0
        assert "E124" in planned.stderr, planned.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_fluent_commands_cross_the_sandbox_without_executing(sandbox):
    repo = sandbox / "fluent"
    repo.mkdir()
    (repo / "gripsack.ts").write_text(
        'import { defineWorkspace, workspace, pkg, provider, fileFetch, '
        'targetPlatform, task, bash, bashBody, packageCommand, exec, lit, profile } '
        'from "@gripsack/core";\n'
        'const shell = pkg("shell", { producer: provider(fileFetch("shell.bin")), '
        'commands: { bash: "bin/bash" }, '
        'target: targetPlatform({ os: "linux", arch: "x86_64" }), '
        'layout: { kind: "relocatable" } });\n'
        'const script = task("script", { steps: [bash(packageCommand("shell", "bash"))'
        '.body(bashBody`\n  echo "$INPUT"\n`)'
        '.env("INPUT", lit("two words")).build()] });\n'
        'const argv = task("argv", { steps: [exec(lit("echo"))'
        '.arg(lit("two words")).build()] });\n'
        'export default defineWorkspace(() => workspace({ outputs: [shell, script, argv, profile("personal", {})] }));\n'
    )
    checked = grip("check", cwd=repo)
    assert checked.returncode == 0, checked.stdout + checked.stderr
    assert "shell (package)" in checked.stdout
    assert "script (task)" in checked.stdout
    assert "argv (task)" in checked.stdout
    result = grip("plan", cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_bash_interpolation_rejected_by_frontend_and_decoded_ir(sandbox):
    repo = sandbox / "literal-bash"
    repo.mkdir()
    (repo / "gripsack.ts").write_text(
        'import { bashBody } from "@gripsack/core";\n'
        'const bad = bashBody`\n'
        '  echo \\${HOME}\n'
        '`;\n'
        'export default bad;\n'
    )
    frontend = grip("check", cwd=repo)
    assert frontend.returncode != 0
    assert "E130" in frontend.stderr
    assert "gripsack.ts:3" in frontend.stderr

    recipe, package = recipe_and_package()
    task = {
        "kind": "task", "name": "bad", "span": {"file": "bad.ts", "line": 5},
        "steps": [{"command": {
            "kind": "run_bash", "span": {"file": "bad.ts", "line": 8},
            "options": ["-e", "-u", "-o", "pipefail"],
            "interpreter": {
                "kind": "package_command", "package": "tool", "command": "tool",
            },
            "body": "echo ok\necho ${HOME}",
            "line_map": [9, 55],
        }}],
        "context": {"kind": "host", "mutable_paths": []},
    }
    decoded = run_plan_ir(sandbox, workspace_ir(recipe, package, task))
    assert decoded.returncode != 0
    assert "E130" in decoded.stderr
    assert "bad.ts:55" in decoded.stderr
    assert "E124" not in decoded.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_raw_workspace_plan_prepares_native_files_without_deploying(sandbox):
    result = run_plan_ir(sandbox, workspace_ir(dotfile_profile()))
    assert result.returncode == 0, result.stdout + result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()
    assert not (sandbox / ".config/demo/settings.conf").exists()


def test_provider_package_needs_no_synthetic_recipe_or_host_entry(sandbox):
    package = {
        "kind": "package",
        "name": "native-tool",
        "span": {"file": "gripsack.ts", "line": 5},
        "producer": {
            "kind": "provider",
            "provider": {
                "kind": "fetch", "fetch": {"kind": "file", "path": "tool.bin"},
                "span": {"file": "gripsack.ts", "line": 6},
            },
        },
        "commands": {"tool": "bin/tool"},
        "target": {"os": "linux", "arch": "x86_64"},
        "layout": {"kind": "relocatable"},
    }
    result = run_plan_ir(sandbox, workspace_ir(package, dotfile_profile()))
    assert result.returncode == 0, result.stdout + result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_decoded_workspace_rejects_unknown_field_with_provenance(sandbox):
    profile = dotfile_profile()
    profile["unexpected_effect"] = True
    result = run_plan_ir(sandbox, workspace_ir(profile))
    assert result.returncode != 0
    assert "unexpected_effect" in result.stderr
    assert "gripsack.ts" in result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_decoded_task_rejects_run_instead_of_steps(sandbox):
    task = {
        "kind": "task",
        "name": "invalid",
        "span": {"file": "gripsack.ts", "line": 5},
        "context": {"kind": "host", "mutable_paths": []},
        "run": {
            "kind": "exec",
            "span": {"file": "gripsack.ts", "line": 6},
            "argv": [{"kind": "literal", "value": "true"}],
        },
    }
    result = run_plan_ir(sandbox, workspace_ir(task))
    assert result.returncode != 0
    assert "run" in result.stderr
    assert "gripsack.ts:5" in result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_duplicate_named_outputs_report_both_declaration_sites(sandbox):
    second = copy.deepcopy(dotfile_profile())
    second["span"]["line"] = 19
    result = run_plan_ir(sandbox, workspace_ir(dotfile_profile(), second))
    assert result.returncode != 0
    assert "dotfiles" in result.stderr
    assert "gripsack.ts:9" in result.stderr
    assert "gripsack.ts:19" in result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def recipe_and_package():
    target = {"os": "linux", "arch": "x86_64"}
    recipe = {
        "kind": "recipe",
        "name": "build",
        "span": {"file": "gripsack.ts", "line": 2},
        "source": {
            "kind": "fetch", "fetch": {"kind": "file", "path": "tool.bin"},
            "span": {"file": "gripsack.ts", "line": 2},
        },
        "execution": {
            "kind": "isolated_linux", "worker": "buildkit",
            "platform": {"os": "linux", "arch": "x86_64"},
            "toolchain": {"reference": "docker.io/library/alpine@sha256:" + "a" * 64},
        },
        "output_kind": "tree",
        "target": target,
    }
    package = {
        "kind": "package",
        "name": "tool",
        "span": {"file": "gripsack.ts", "line": 3},
        "producer": {"kind": "recipe", "recipe": "build"},
        "commands": {"tool": "bin/tool"},
        "target": target,
        "layout": {"kind": "relocatable"},
    }
    return recipe, package


def test_decoded_artifact_reference_rejects_non_artifact_at_consumer(sandbox):
    task = {
        "kind": "task",
        "name": "not-an-artifact",
        "span": {"file": "gripsack.ts", "line": 15},
        "steps": [{"command": {
            "kind": "exec",
            "span": {"file": "gripsack.ts", "line": 15},
            "argv": [{"kind": "literal", "value": "true"}],
        }}],
        "context": {"kind": "host", "mutable_paths": []},
    }
    profile = dotfile_profile()
    profile["files"][0]["source"] = {
        "kind": "artifact_file",
        "output": "not-an-artifact",
        "selector": "config",
    }
    profile["files"][0]["content"] = {"kind": "identity"}
    result = run_plan_ir(sandbox, workspace_ir(task, profile))
    assert result.returncode != 0
    assert "E126" in result.stderr, result.stderr
    assert "gripsack.ts:10" in result.stderr
    assert "E124" not in result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()
    assert not (sandbox / ".config/demo/settings.conf").exists()


def test_decoded_check_subject_must_exist_before_execution(sandbox):
    check = {
        "kind": "check",
        "name": "verify",
        "span": {"file": "gripsack.ts", "line": 16},
        "subject": "no-such-output",
        "run": {
            "kind": "exec",
            "span": {"file": "gripsack.ts", "line": 16},
            "argv": [{"kind": "literal", "value": "true"}],
        },
    }
    result = run_plan_ir(sandbox, workspace_ir(check))
    assert result.returncode != 0
    assert "E126" in result.stderr and "no-such-output" in result.stderr
    assert "gripsack.ts:16" in result.stderr
    assert "E124" not in result.stderr


def test_decoded_recipe_tool_cycle_is_rejected_before_execution(sandbox):
    recipe, package = recipe_and_package()
    recipe["steps"] = [
        {"command": {
            "kind": "exec",
            "span": {"file": "gripsack.ts", "line": 4},
            "argv": [
                {"kind": "package_command", "package": "tool", "command": "tool"},
            ],
        }}
    ]
    result = run_plan_ir(sandbox, workspace_ir(recipe, package))
    assert result.returncode != 0
    assert "E127" in result.stderr, result.stderr
    assert "gripsack.ts:2" in result.stderr
    assert "E124" not in result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_decoded_artifact_selector_rejects_parent_escape(sandbox):
    recipe, package = recipe_and_package()
    profile = dotfile_profile()
    profile["files"][0]["source"] = {
        "kind": "artifact_file",
        "output": "tool",
        "selector": "../secrets",
    }
    profile["files"][0]["content"] = {"kind": "identity"}
    result = run_plan_ir(sandbox, workspace_ir(recipe, package, profile))
    assert result.returncode != 0
    assert "selector" in result.stderr and "gripsack.ts:10" in result.stderr
    assert "E124" not in result.stderr


def test_decoded_package_target_mismatch_rejects_with_both_sites(sandbox):
    recipe, package = recipe_and_package()
    package["target"] = {"os": "macos", "arch": "aarch64"}
    result = run_plan_ir(sandbox, workspace_ir(recipe, package))
    assert result.returncode != 0
    assert "target" in result.stderr
    assert "gripsack.ts:2" in result.stderr
    assert "gripsack.ts:3" in result.stderr
    assert "E124" not in result.stderr


def test_decoded_fixed_prefix_package_cannot_enter_prefixless_environment(sandbox):
    recipe, package = recipe_and_package()
    package["layout"] = {"kind": "fixed_prefix", "prefix": "/opt/tool"}
    environment = {
        "kind": "environment",
        "name": "tools",
        "span": {"file": "gripsack.ts", "line": 6},
        "packages": ["tool"],
        "target": package["target"],
    }
    result = run_plan_ir(sandbox, workspace_ir(recipe, package, environment))
    assert result.returncode != 0
    assert "E128" in result.stderr, result.stderr
    assert "gripsack.ts:6" in result.stderr
    assert "E124" not in result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_publication_check_cycle_is_admitted_without_pruning_or_execution(sandbox):
    recipe, package = recipe_and_package()
    recipe["checks"] = ["smoke"]
    smoke = {
        "kind": "check",
        "name": "smoke",
        "span": {"file": "gripsack.ts", "line": 8},
        "subject": "tool",
        "run": {
            "kind": "exec",
            "span": {"file": "gripsack.ts", "line": 8},
            "argv": [{"kind": "literal", "value": "true"}],
        },
    }
    result = run_plan_ir(sandbox, workspace_ir(recipe, package, smoke, dotfile_profile()))
    assert result.returncode == 0, result.stdout + result.stderr
    assert "E131" not in result.stderr
    assert "E127" not in result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_tagged_execution_and_layout_fields_are_closed_with_output_spans(sandbox):
    recipe, package = recipe_and_package()
    for node, field, line in [(recipe, "execution", 2), (package, "layout", 3)]:
        malformed = copy.deepcopy(node)
        malformed[field]["ambient_effect"] = True
        outputs = (malformed, package) if field == "execution" else (recipe, malformed)
        result = run_plan_ir(sandbox, workspace_ir(*outputs))
        assert result.returncode != 0
        assert "ambient_effect" in result.stderr
        assert f"gripsack.ts:{line}" in result.stderr
        assert "E124" not in result.stderr


def test_minimum_os_and_abi_compare_declared_targets_not_checking_host(sandbox):
    recipe, package = recipe_and_package()
    target = {
        "os": "linux", "arch": "x86_64", "abi": "gnu",
        "minimum_os": {"major": 5, "minor": 15},
    }
    recipe["target"] = copy.deepcopy(target)
    package["target"] = copy.deepcopy(target)
    environment = {
        "kind": "environment", "name": "tools",
        "span": {"file": "gripsack.ts", "line": 6},
        "packages": ["tool"],
        "target": {**target, "minimum_os": {"major": 6, "minor": 1}},
    }
    admitted = run_plan_ir(sandbox, workspace_ir(recipe, package, environment, dotfile_profile()))
    assert admitted.returncode == 0, admitted.stdout + admitted.stderr
    environment["target"]["minimum_os"] = {"major": 4, "minor": 19}
    rejected = run_plan_ir(sandbox, workspace_ir(recipe, package, environment))
    assert rejected.returncode != 0
    assert "E128" in rejected.stderr, rejected.stderr
    assert "gripsack.ts:3" in rejected.stderr and "gripsack.ts:6" in rejected.stderr

    environment["target"] = {**target, "abi": "musl"}
    wrong_abi = run_plan_ir(sandbox, workspace_ir(recipe, package, environment))
    assert wrong_abi.returncode != 0
    assert "E128" in wrong_abi.stderr, wrong_abi.stderr
    assert "gripsack.ts:3" in wrong_abi.stderr and "gripsack.ts:6" in wrong_abi.stderr
    environment["target"].pop("abi")
    missing_abi = run_plan_ir(sandbox, workspace_ir(recipe, package, environment))
    assert missing_abi.returncode != 0
    assert "E128" in missing_abi.stderr, missing_abi.stderr
    assert "gripsack.ts:3" in missing_abi.stderr and "gripsack.ts:6" in missing_abi.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_fixed_prefix_selection_requires_exact_declared_environment_path(sandbox):
    recipe, package = recipe_and_package()
    package["layout"] = {"kind": "fixed_prefix", "prefix": "/opt/tool"}
    environment = {
        "kind": "environment", "name": "tools",
        "span": {"file": "gripsack.ts", "line": 6},
        "packages": ["tool"], "target": package["target"],
        "prefix": "/opt/tool",
    }
    admitted = run_plan_ir(sandbox, workspace_ir(recipe, package, environment, dotfile_profile()))
    assert admitted.returncode == 0, admitted.stdout + admitted.stderr
    environment["prefix"] = "/different"
    rejected = run_plan_ir(sandbox, workspace_ir(recipe, package, environment))
    assert rejected.returncode != 0
    assert "E128" in rejected.stderr, rejected.stderr
    assert "gripsack.ts:6" in rejected.stderr
    package["layout"]["prefix"] = "/opt/../escape"
    unsafe = run_plan_ir(sandbox, workspace_ir(recipe, package))
    assert unsafe.returncode != 0
    assert "E130" in unsafe.stderr and "gripsack.ts:3" in unsafe.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()
