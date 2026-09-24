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
