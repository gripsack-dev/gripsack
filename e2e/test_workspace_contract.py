"""v4 workspace admission through the shipped grip binary (A1-01).

Direct --ir cases exercise the core even when no TypeScript driver is
involved: a decoded workspace cannot become an empty legacy module
profile, and malformed catalog entries must fail before effects.
"""
from __future__ import annotations

import copy
import json

from conftest import grip


def workspace_ir(*outputs):
    return {
        "ir_version": 4,
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


def test_project_workspace_check_is_read_only_and_consumers_fail(sandbox):
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
    for command in [
        ("plan",),
        ("update", "--check"),
        ("adopt", "~/.config/demo/settings.conf"),
        ("apply",),
    ]:
        result = grip(*command, cwd=repo)
        assert result.returncode != 0
        assert "E124" in result.stderr, (command, result.stderr)
        assert not (sandbox / ".config/demo/settings.conf").exists()
        assert not (sandbox / ".local/share/gripsack/current").exists()


def test_workspace_plan_refuses_instead_of_planning_empty_legacy_profile(sandbox):
    result = run_plan_ir(sandbox, workspace_ir(dotfile_profile()))
    assert result.returncode != 0
    assert "E124" in result.stderr and "gripsack.ts" in result.stderr
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
                "fetch": {"kind": "file", "path": "tool.bin"},
                "span": {"file": "gripsack.ts", "line": 6},
            },
        },
        "commands": {"tool": "bin/tool"},
        "target": {"os": "linux", "arch": "x86_64"},
        "layout": "relocatable",
    }
    result = run_plan_ir(sandbox, workspace_ir(package))
    assert result.returncode != 0
    assert "E124" in result.stderr, result.stderr  # admitted, but realization is A2-owned
    assert "E126" not in result.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_decoded_workspace_rejects_unknown_field_with_provenance(sandbox):
    profile = dotfile_profile()
    profile["unexpected_effect"] = True
    result = run_plan_ir(sandbox, workspace_ir(profile))
    assert result.returncode != 0
    assert "unexpected_effect" in result.stderr
    assert "gripsack.ts" in result.stderr
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
            "fetch": {"kind": "file", "path": "tool.bin"},
            "span": {"file": "gripsack.ts", "line": 2},
        },
        "execution": "native",
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
        "layout": "relocatable",
    }
    return recipe, package


def test_decoded_artifact_reference_rejects_non_artifact_with_both_sites(sandbox):
    task = {
        "kind": "task",
        "name": "not-an-artifact",
        "span": {"file": "gripsack.ts", "line": 15},
        "run": {
            "kind": "exec",
            "span": {"file": "gripsack.ts", "line": 15},
            "argv": [{"kind": "literal", "value": "true"}],
        },
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
    assert "gripsack.ts:15" in result.stderr
    assert "E124" not in result.stderr


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
        {
            "kind": "exec",
            "span": {"file": "gripsack.ts", "line": 4},
            "argv": [
                {"kind": "package_command", "package": "tool", "command": "tool"},
            ],
        }
    ]
    result = run_plan_ir(sandbox, workspace_ir(recipe, package))
    assert result.returncode != 0
    assert "E127" in result.stderr, result.stderr
    assert "gripsack.ts:2" in result.stderr
    assert "gripsack.ts:3" in result.stderr
    assert "E124" not in result.stderr


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
    package["layout"] = "fixed_prefix"
    environment = {
        "kind": "environment",
        "name": "tools",
        "span": {"file": "gripsack.ts", "line": 6},
        "packages": ["tool"],
        "target": package["target"],
    }
    result = run_plan_ir(sandbox, workspace_ir(recipe, package, environment))
    assert result.returncode != 0
    assert "fixed_prefix" in result.stderr
    assert "gripsack.ts:3" in result.stderr
    assert "gripsack.ts:6" in result.stderr
    assert "E124" not in result.stderr
