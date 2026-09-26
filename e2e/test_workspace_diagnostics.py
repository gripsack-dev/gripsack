"""Source-aware workspace diagnostics through the real CLI (0052 A1-06).

One canonical structured representation (the core's Diagnostic) feeds
both surfaces: the terminal rendering and `grip check --json`'s
document must carry the same facts — codes, messages, and every
labeled span, including both collision declarations and the dedented
Bash line map. All of it is read-only: no builder provisioning, no
store, no generation.
"""

from __future__ import annotations

import json
import os
import pytest

from conftest import grip, make_env_repo


def write_repo(sandbox, name, source):
    repo = sandbox / name
    repo.mkdir()
    (repo / "gripsack.ts").write_text(source)
    return repo


def check_both(repo, *extra):
    """Run terminal and JSON check; return (terminal, parsed JSON doc)."""
    terminal = grip("check", *extra, cwd=repo)
    machine = grip("check", "--json", *extra, cwd=repo)
    assert machine.returncode == terminal.returncode, (
        f"surface disagreement: {terminal.returncode} vs {machine.returncode}\n"
        f"{terminal.stderr}\n{machine.stdout}"
    )
    doc = json.loads(machine.stdout)
    assert doc["version"] == 1
    return terminal, doc


def assert_same_facts(terminal, doc):
    """Every diagnostic fact the terminal renders is in the JSON doc."""
    assert doc["diagnostics"], "JSON surface lost the diagnostics"
    for d in doc["diagnostics"]:
        assert f"[{d['code']}]" in terminal.stderr
        assert d["message"] in terminal.stderr
        for label in d["labels"]:
            if label["span"] is not None:
                line = f"{label['span']['file']}:{label['span']['line']}"
                assert line in terminal.stderr, f"{line} missing from terminal"
                assert label["note"] in terminal.stderr
        if d.get("help") is not None:
            assert d["help"] in terminal.stderr, "help text drifted between surfaces"

def test_typo_field_reports_declaration_site_on_both_surfaces(sandbox):
    repo = write_repo(
        sandbox,
        "typo",
        'import { defineWorkspace, workspace, profile, file, literalText, trackedCopyTo } '
        'from "@gripsack/core";\n'
        "export default defineWorkspace(() => workspace({ outputs: [\n"
        '  profile("dotfiles", { fiels: [file({\n'
        '    content: literalText("setting=1\\n"),\n'
        '    destination: trackedCopyTo("~/.config/demo/settings.conf"),\n'
        "  })] }),\n"
        "] }));\n",
    )
    terminal, doc = check_both(repo)
    assert terminal.returncode == 1
    assert "E130" in terminal.stderr and "fiels" in terminal.stderr
    assert "gripsack.ts:3:3" in terminal.stderr
    assert_same_facts(terminal, doc)
    assert doc["diagnostics"][0]["labels"][0]["span"]["line"] == 3
    # no traceback noise on the structured path
    assert "frontend eval failed" not in terminal.stderr
    assert not (sandbox / ".config/demo/settings.conf").exists()

def test_treefiles_expands_a_captured_repo_directory_into_admitted_files(sandbox):
    """A1-11: bounded eval-time tree expansion — explicit per-file v5
    entries with stable enumeration, admitted by `grip check` without
    any executor or home mutation."""
    repo = write_repo(
        sandbox,
        "tree",
        'import { defineWorkspace, workspace, profile, treeFiles } from "@gripsack/core";\n'
        "export default defineWorkspace(() => workspace({ outputs: [\n"
        '  profile("dotfiles", { files: [\n'
        '    ...treeFiles("configs", "~/.config/demo", { exclude: ["skipped"] }),\n'
        "  ] }),\n"
        "] }));\n",
    )
    (repo / "configs").mkdir()
    (repo / "configs" / "b.conf").write_text("b\n")
    (repo / "configs" / "a.conf").write_text("a\n")
    (repo / "configs" / "skipped").mkdir()
    (repo / "configs" / "skipped" / "x.conf").write_text("x\n")
    terminal = grip("check", cwd=repo)
    assert terminal.returncode == 0, terminal.stderr
    assert "dotfiles" in terminal.stdout
    assert not (sandbox / ".config/demo").exists()

    # a repo symlink inside the captured tree is an authoring failure,
    # never followed out of the repo
    os.symlink(repo / "configs" / "a.conf", repo / "configs" / "leak.conf")
    hostile = grip("check", cwd=repo)
    assert hostile.returncode != 0
    assert "not a regular file or directory" in hostile.stderr


def test_wrong_kind_reference_reports_both_sites_on_both_surfaces(sandbox):
    repo = write_repo(
        sandbox,
        "wrong-kind",
        'import { defineWorkspace, workspace, profile, environment, targetPlatform, '
        'file, literalText, trackedCopyTo } from "@gripsack/core";\n'
        'const dotfiles = profile("dotfiles", { files: [file({\n'
        '  content: literalText("a=1\\n"),\n'
        '  destination: trackedCopyTo("~/.config/demo/a.conf"),\n'
        "})] });\n"
        'const env = environment("myenv", { packages: ["dotfiles"], '
        'target: targetPlatform({ os: "linux", arch: "x86_64" }) });\n'
        "export default defineWorkspace(() => workspace({ outputs: [dotfiles, env] }));\n",
    )
    terminal, doc = check_both(repo)
    assert terminal.returncode == 1
    assert "E126" in terminal.stderr
    assert_same_facts(terminal, doc)
    labels = doc["diagnostics"][0]["labels"]
    assert [label["span"]["line"] for label in labels] == [6, 2]
    assert "profile" in labels[1]["note"]
    assert "package" in doc["diagnostics"][0]["message"]
    assert not (sandbox / ".config/demo/a.conf").exists()


def test_collision_reports_both_declaration_spans_on_both_surfaces(sandbox):
    repo = write_repo(
        sandbox,
        "collision",
        'import { defineWorkspace, workspace, profile, file, literalText, trackedCopyTo } '
        'from "@gripsack/core";\n'
        'const first = profile("dotfiles", { files: [file({\n'
        '  content: literalText("a=1\\n"),\n'
        '  destination: trackedCopyTo("~/.config/demo/a.conf"),\n'
        "})] });\n"
        'const again = profile("dotfiles", { files: [file({\n'
        '  content: literalText("b=2\\n"),\n'
        '  destination: trackedCopyTo("~/.config/demo/b.conf"),\n'
        "})] });\n"
        "export default defineWorkspace(() => workspace({ outputs: [first, again] }));\n",
    )
    terminal, doc = check_both(repo)
    assert terminal.returncode == 1
    assert "E125" in terminal.stderr
    assert "gripsack.ts:2" in terminal.stderr and "gripsack.ts:6" in terminal.stderr
    assert_same_facts(terminal, doc)
    labels = doc["diagnostics"][0]["labels"]
    assert labels[0]["note"] != labels[1]["note"]
    assert [label["span"]["line"] for label in labels] == [2, 6]


def test_target_mismatch_reports_both_sites_on_both_surfaces(sandbox):
    repo = write_repo(
        sandbox,
        "target",
        'import { defineWorkspace, workspace, pkg, provider, fileFetch, environment, '
        'targetPlatform } from "@gripsack/core";\n'
        'const tool = pkg("tool", { producer: provider(fileFetch("tool.bin")),\n'
        '  commands: { tool: "bin/tool" },\n'
        '  target: targetPlatform({ os: "macos", arch: "aarch64" }),\n'
        '  layout: { kind: "relocatable" } });\n'
        'const env = environment("myenv", { packages: ["tool"], '
        'target: targetPlatform({ os: "linux", arch: "x86_64" }) });\n'
        "export default defineWorkspace(() => workspace({ outputs: [tool, env] }));\n",
    )
    terminal, doc = check_both(repo)
    assert terminal.returncode == 1
    assert "E126" in terminal.stderr
    assert_same_facts(terminal, doc)
    labels = doc["diagnostics"][0]["labels"]
    assert [label["span"]["line"] for label in labels] == [6, 2]
    assert "myenv" in labels[0]["note"]
    assert "tool" in labels[1]["note"]


def test_ambient_bash_interpreter_is_a_structured_capability_error(sandbox):
    """0052 §3.2: host runBash without a declared toolchain pin is an
    unavailable capability — E128 at admission, never silent fallback."""
    repo = write_repo(
        sandbox,
        "ambient",
        'import { defineWorkspace, workspace, task, runBash, lit } from "@gripsack/core";\n'
        "export default defineWorkspace(() => workspace({ outputs: [\n"
        '  task("script", { run: runBash({ interpreter: lit("bash"), body: "true" }) }),\n'
        "] }));\n",
    )
    terminal, doc = check_both(repo)
    assert terminal.returncode == 1
    assert "E128" in terminal.stderr
    assert "gripsack.ts:3" in terminal.stderr
    assert_same_facts(terminal, doc)
    assert doc["diagnostics"][0]["code"] == "E128"
    # admission rejected it; nothing ran
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_dedented_bash_interpolation_maps_to_the_original_source_line(sandbox):
    """The label points at the user's template line (9), not the
    dedented body line (2) — the frontend's line map and the core's
    rendering agree."""
    repo = write_repo(
        sandbox,
        "dedent",
        'import { defineWorkspace, workspace, pkg, provider, fileFetch, targetPlatform, '
        'task, bash, bashBody, packageCommand } from "@gripsack/core";\n'
        'const shell = pkg("shell", { producer: provider(fileFetch("shell.bin")),\n'
        '  commands: { bash: "bin/bash" },\n'
        '  target: targetPlatform({ os: "linux", arch: "x86_64" }),\n'
        '  layout: { kind: "relocatable" } });\n'
        'const script = task("script", { run: bash(packageCommand("shell", "bash"))\n'
        "  .body(bashBody`\n"
        "    echo first\n"
        "    echo second \\${HOME}\n"
        "    echo third\n"
        "  `).build() });\n"
        "export default defineWorkspace(() => workspace({ outputs: [shell, script] }));\n",
    )
    terminal, doc = check_both(repo)
    assert terminal.returncode == 1
    assert "E130" in terminal.stderr
    assert "gripsack.ts:9" in terminal.stderr
    assert "echo second \\${HOME}" in terminal.stderr  # snippet shows the source line
    assert_same_facts(terminal, doc)
    span = doc["diagnostics"][0]["labels"][0]["span"]
    assert span["line"] == 9, "generated (dedented) line 2 must map back to source line 9"


def test_core_line_map_renders_the_mapped_source_snippet(sandbox):
    """Decoded IR (no frontend): the core maps a dedented body's
    generated line back through line_map and renders the snippet from
    the real file at the mapped line."""
    repo = sandbox / "decoded"
    repo.mkdir()
    source_lines = ["// filler"] * 60
    source_lines[54] = "echo ${HOME}  # original line 55"
    (repo / "bad.ts").write_text("\n".join(source_lines) + "\n")
    recipe = {
        "kind": "recipe", "name": "build", "span": {"file": "bad.ts", "line": 2},
        "source": {"fetch": {"kind": "file", "path": "src.tar.gz"}, "span": {"file": "bad.ts", "line": 3}},
        "execution": {"kind": "host", "access": "unconfined"},
        "output_kind": "tree",
        "target": {"os": "linux", "arch": "x86_64"},
    }
    package = {
        "kind": "package", "name": "tool", "span": {"file": "bad.ts", "line": 4},
        "producer": {"kind": "recipe", "recipe": "build"},
        "commands": {"tool": "bin/tool"},
        "target": {"os": "linux", "arch": "x86_64"},
        "layout": {"kind": "relocatable"},
    }
    task = {
        "kind": "task", "name": "bad", "span": {"file": "bad.ts", "line": 5},
        "run": {
            "kind": "run_bash", "span": {"file": "bad.ts", "line": 8},
            "interpreter": {"kind": "package_command", "package": "tool", "command": "tool"},
            "body": "echo ok\necho ${HOME}",
            "line_map": [9, 55],
        },
    }
    doc = {
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64", "tags": []},
        "workspace": {"span": {"file": "bad.ts", "line": 1}, "outputs": [recipe, package, task]},
    }
    ir_path = repo / "workspace.ir.json"
    ir_path.write_text(json.dumps(doc))
    result = grip("plan", "--ir", str(ir_path), cwd=repo)
    assert result.returncode != 0
    assert "E130" in result.stderr
    assert "bad.ts:55" in result.stderr, result.stderr
    assert "original line 55" in result.stderr, "snippet must render the mapped source line"
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_decoded_diagnostic_span_cannot_read_outside_repo(sandbox):
    private = sandbox / "private.txt"
    private.write_text("must never appear in a diagnostic\n")
    repo = sandbox / "outside"
    repo.mkdir()
    package = {
        "kind": "package", "name": "shell", "span": {"file": "gripsack.ts", "line": 2},
        "producer": {"kind": "provider", "provider": {
            "fetch": {"kind": "file", "path": "shell.bin"},
            "span": {"file": "gripsack.ts", "line": 2},
        }},
        "commands": {"bash": "bin/bash"},
        "target": {"os": "linux", "arch": "x86_64"},
        "layout": {"kind": "relocatable"},
    }
    task = {
        "kind": "task", "name": "bad", "span": {"file": "gripsack.ts", "line": 3},
        "run": {
            "kind": "run_bash", "span": {"file": str(private), "line": 1},
            "interpreter": {"kind": "package_command", "package": "shell", "command": "bash"},
            "body": "echo ${HOME}",
        },
    }
    doc = {
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {
            "span": {"file": "gripsack.ts", "line": 1},
            "outputs": [package, task],
        },
    }
    path = repo / "workspace.ir.json"
    path.write_text(json.dumps(doc))
    result = grip("plan", "--ir", str(path), cwd=repo)
    assert result.returncode != 0
    assert "E130" in result.stderr
    assert str(private) in result.stderr
    assert "must never appear in a diagnostic" not in result.stderr


def test_legacy_env_check_json_carries_modules_and_tags(sandbox):
    repo = make_env_repo(
        sandbox / "legacy",
        {
            "demo": 'import { module, trackedCopy } from "@gripsack/core";\n'
            'export default module("demo", {\n'
            '  config: { "configs/demo/demo.toml": trackedCopy("~/.config/demo/demo.toml") },\n'
            "});\n"
        },
    )
    # E110: a fetch-less module deploys repo files — the source must exist
    (repo / "configs/demo").mkdir(parents=True)
    (repo / "configs/demo/demo.toml").write_text("setting=1\n")
    terminal, doc = check_both(repo, "--host", "testhost")
    assert terminal.returncode == 0
    assert doc["ok"] is True
    assert doc["modules"] == ["demo"]
    assert doc["host"]["tags"] == ["test"]
    assert not (sandbox / ".config/demo/demo.toml").exists()


def test_check_success_json_lists_the_named_outputs(sandbox):
    repo = write_repo(
        sandbox,
        "success",
        'import { defineWorkspace, workspace, profile, file, literalText, trackedCopyTo } '
        'from "@gripsack/core";\n'
        "export default defineWorkspace(() => workspace({ outputs: [\n"
        '  profile("dotfiles", { files: [file({\n'
        '    content: literalText("setting=1\\n"),\n'
        '    destination: trackedCopyTo("~/.config/demo/settings.conf"),\n'
        "  })] }),\n"
        "] }));\n",
    )
    terminal, doc = check_both(repo)
    assert terminal.returncode == 0
    assert "dotfiles (profile)" in terminal.stdout
    assert doc["ok"] is True
    assert doc["diagnostics"] == []
    assert doc["host"]["os"] and doc["host"]["arch"]
    outputs = doc["outputs"]
    assert [o["name"] for o in outputs] == ["dotfiles"]
    assert outputs[0]["kind"] == "profile"
    assert outputs[0]["span"]["line"] == 3
    assert not (sandbox / ".config/demo/settings.conf").exists()


def test_check_never_provisions_a_builder(sandbox):
    """A1 no-builder guarantee: even an isolated_linux recipe checks
    clean without BuildKit/Lima materializing anywhere."""
    repo = write_repo(
        sandbox,
        "isolated",
        'import { defineWorkspace, workspace, recipe, targetPlatform, fileFetch } from "@gripsack/core";\n'
        'const build = recipe("build", { source: fileFetch("src.tar.gz"),\n'
        '  execution: { kind: "isolated_linux", worker: "buildkit" },\n'
        '  output_kind: "tree",\n'
        '  target: targetPlatform({ os: "linux", arch: "x86_64" }) });\n'
        "export default defineWorkspace(() => workspace({ outputs: [build] }));\n",
    )
    checked = grip("check", cwd=repo)
    assert checked.returncode == 0, checked.stdout + checked.stderr
    assert "build (recipe)" in checked.stdout
    builders = [
        p
        for p in sandbox.rglob("*")
        if any(marker in p.name.lower() for marker in ("buildkit", "lima", "builder"))
    ]
    assert builders == [], f"check provisioned a builder: {builders}"
    # but execution is refused with the capability diagnostic
    planned = grip("plan", cwd=repo)
    assert planned.returncode != 0
    assert "E124" in planned.stderr
    assert "isolated_linux" in planned.stderr and "B2" in planned.stderr
    assert "gripsack.ts:2" in planned.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_schedule_and_task_prerequisite_capabilities_keep_their_owners(sandbox):
    repo = sandbox / "inert-schedule"
    repo.mkdir()
    task = {
        "kind": "task", "name": "build", "span": {"file": "tasks.ts", "line": 8},
        "run": {
            "kind": "exec", "span": {"file": "tasks.ts", "line": 9},
            "argv": [{"kind": "literal", "value": "true"}],
        },
    }
    schedule = {
        "kind": "schedule", "name": "daily", "span": {"file": "schedule.ts", "line": 4},
        "task": "build", "trigger": {"kind": "daily", "time": "03:30"}, "scope": "user",
    }
    document = {
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {
            "span": {"file": "gripsack.ts", "line": 1},
            "outputs": [schedule, task],
        },
    }
    path = repo / "schedule.ir.json"
    path.write_text(json.dumps(document))
    inert = grip("plan", "--ir", str(path), cwd=repo)
    assert inert.returncode != 0
    assert "E124" in inert.stderr and "schedule registration" in inert.stderr
    assert "E2/E3" in inert.stderr and "schedule.ts:4" in inert.stderr

    prerequisite = {
        **task, "deps": ["verify"],
    }
    verify = {
        **task, "name": "verify", "span": {"file": "tasks.ts", "line": 12},
    }
    document["workspace"]["outputs"] = [prerequisite, verify]
    path.write_text(json.dumps(document))
    invoked = grip("plan", "--ir", str(path), cwd=repo)
    assert invoked.returncode != 0
    assert "E124" in invoked.stderr and "task prerequisite" in invoked.stderr
    assert "E1" in invoked.stderr and "tasks.ts:8" in invoked.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


@pytest.mark.parametrize(
    ("first", "rest", "owner", "capability"),
    [
        pytest.param(
            {
                "kind": "image", "name": "container", "span": {"file": "outputs.ts", "line": 7},
                "packages": [], "target": {"os": "linux", "arch": "x86_64"},
            },
            [],
            "B4",
            "image materialization",
            id="image-worker",
        ),
        pytest.param(
            {
                "kind": "environment", "name": "dev", "span": {"file": "outputs.ts", "line": 7},
                "packages": [], "target": {"os": "linux", "arch": "x86_64"},
            },
            [],
            "A2-P",
            "environment activation",
            id="environment-invocation",
        ),
        pytest.param(
            {
                "kind": "check", "name": "verify", "span": {"file": "outputs.ts", "line": 7},
                "run": {
                    "kind": "exec", "span": {"file": "outputs.ts", "line": 8},
                    "argv": [{"kind": "literal", "value": "true"}],
                },
                "subject": "run",
            },
            [
                {
                    "kind": "task", "name": "run", "span": {"file": "outputs.ts", "line": 12},
                    "run": {
                        "kind": "exec", "span": {"file": "outputs.ts", "line": 13},
                        "argv": [{"kind": "literal", "value": "true"}],
                    },
                },
            ],
            "A2/E1",
            "check execution",
            id="check-owner-before-subject",
        ),
    ],
)
def test_unavailable_capability_names_first_declared_output_and_owner(
    sandbox, first, rest, owner, capability,
):
    repo = sandbox / "capability-owner"
    repo.mkdir()
    path = repo / "workspace.ir.json"
    path.write_text(json.dumps({
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {
            "span": {"file": "outputs.ts", "line": 1},
            "outputs": [first, *rest],
        },
    }))
    planned = grip("plan", "--ir", str(path), cwd=repo)
    assert planned.returncode != 0
    assert "E124" in planned.stderr, planned.stderr
    assert first["name"] in planned.stderr
    assert capability in planned.stderr and f"belongs to {owner}" in planned.stderr
    assert "outputs.ts:7" in planned.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()


@pytest.mark.parametrize(
    ("first", "owner", "capability"),
    [
        pytest.param(
            {
                "kind": "recipe", "name": "compile",
                "span": {"file": "outputs.ts", "line": 7},
                "source": {
                    "fetch": {"kind": "file", "path": "source.tar.gz"},
                    "span": {"file": "outputs.ts", "line": 8},
                },
                "execution": {"kind": "host", "access": "unconfined"},
                "output_kind": "tree",
                "target": {"os": "linux", "arch": "x86_64"},
            },
            "A2", "host recipe realization", id="host-recipe",
        ),
        pytest.param(
            {
                "kind": "package", "name": "tool",
                "span": {"file": "outputs.ts", "line": 7},
                "producer": {
                    "kind": "provider",
                    "provider": {
                        "fetch": {"kind": "file", "path": "tool.tar.gz"},
                        "span": {"file": "outputs.ts", "line": 8},
                    },
                },
                "commands": {"tool": "bin/tool"},
                "target": {"os": "linux", "arch": "x86_64"},
                "layout": {"kind": "relocatable"},
            },
            "A2", "package realization", id="provider-package",
        ),
        pytest.param(
            {
                "kind": "task", "name": "invoke",
                "span": {"file": "outputs.ts", "line": 7},
                "run": {
                    "kind": "exec", "span": {"file": "outputs.ts", "line": 8},
                    "argv": [{"kind": "literal", "value": "true"}],
                },
            },
            "A2-P", "task invocation", id="task-without-prerequisites",
        ),
        pytest.param(
            {
                "kind": "profile", "name": "dotfiles",
                "span": {"file": "outputs.ts", "line": 7},
            },
            "A2", "profile deployment", id="profile-before-image",
        ),
        pytest.param(
            {
                "kind": "hook", "name": "after",
                "span": {"file": "outputs.ts", "line": 7},
                "trigger": "post_activate",
                "run": {
                    "kind": "exec", "span": {"file": "outputs.ts", "line": 8},
                    "argv": [{"kind": "literal", "value": "true"}],
                },
            },
            "A2", "hook execution", id="post-activate-hook",
        ),
    ],
)
def test_first_declared_unavailable_output_preserves_its_distinct_owner(
    sandbox, first, owner, capability,
):
    repo = sandbox / "first-capability"
    repo.mkdir()
    later_image = {
        "kind": "image", "name": "later", "span": {"file": "outputs.ts", "line": 12},
        "packages": [], "target": {"os": "linux", "arch": "x86_64"},
    }
    path = repo / "workspace.ir.json"
    path.write_text(json.dumps({
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {
            "span": {"file": "outputs.ts", "line": 1},
            "outputs": [first, later_image],
        },
    }))
    planned = grip("plan", "--ir", str(path), cwd=repo)
    assert planned.returncode != 0
    assert "error[E124]" in planned.stderr, planned.stderr
    assert f"output `{first['name']}`" in planned.stderr
    assert f"{capability} belongs to {owner}" in planned.stderr
    assert "outputs.ts:7" in planned.stderr
    assert "belongs to B4" not in planned.stderr, "later image must not win"
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_invalid_decoded_command_coordinates_keep_labels_without_reading_source(sandbox):
    repo = sandbox / "invalid-coordinate"
    repo.mkdir()
    (repo / "source.ts").write_text("private source line must not appear\n")
    task = {
        "kind": "task", "name": "bad", "span": {"file": "source.ts", "line": 7},
        "run": {
            "kind": "exec", "span": {"file": "source.ts", "line": 0, "col": 0},
            "argv": [{"kind": "literal", "value": "true"}],
        },
    }
    path = repo / "workspace.ir.json"
    path.write_text(json.dumps({
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {
            "span": {"file": "source.ts", "line": 1},
            "outputs": [task],
        },
    }))
    planned = grip("plan", "--ir", str(path), cwd=repo)
    assert planned.returncode != 0
    assert planned.stderr.count("error[E129]") == 2, planned.stderr
    assert planned.stderr.count("--> source.ts:0:0") == 2
    assert "private source line must not appear" not in planned.stderr
    assert "E124" not in planned.stderr, "invalid provenance precedes executor refusal"
    assert not (sandbox / ".local/share/gripsack/current").exists()



def test_escaping_profile_file_destination_is_a_structured_error(sandbox):
    """A1-11: `..`/relative/bare-~ destination paths reject at admission
    with the file's own declaration span — before any E124 or effect."""
    repo = sandbox / "escaping-destination"
    repo.mkdir()
    profile = {
        "kind": "profile", "name": "dotfiles", "span": {"file": "grip.ts", "line": 2},
        "files": [{
            "span": {"file": "grip.ts", "line": 3},
            "content": {"kind": "literal", "text": "x\n"},
            "destination": {"kind": "tracked_copy", "path": "~/../../etc/passwd"},
        }],
    }
    path = repo / "workspace.ir.json"
    path.write_text(json.dumps({
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {"span": {"file": "grip.ts", "line": 1}, "outputs": [profile]},
    }))
    result = grip("plan", "--ir", str(path), cwd=repo)
    assert result.returncode != 0
    assert "error[E102]" in result.stderr, result.stderr
    assert "~/../../etc/passwd" in result.stderr
    assert "grip.ts:3" in result.stderr
    assert "E124" not in result.stderr, "invalid destination precedes executor refusal"
    assert not (sandbox / "etc").exists()
    assert not (sandbox / ".local/share/gripsack/current").exists()

def test_escaped_repository_file_origin_rejects_before_executor(sandbox):
    """Decoded v5 repo_file cannot address an ambient or parent file."""
    repo = sandbox / "escaped-repository-origin"
    repo.mkdir()
    input_file = repo / "workspace.ir.json"
    for outside in ("../private", "/etc/passwd"):
        profile = {
            "kind": "profile", "name": "dotfiles", "span": {"file": "grip.ts", "line": 2},
            "files": [{
                "span": {"file": "grip.ts", "line": 3},
                "source": {"kind": "repo_file", "path": outside},
                "content": {"kind": "identity"},
                "destination": {"kind": "tracked_copy", "path": "~/.config/dotfile"},
            }],
        }
        input_file.write_text(json.dumps({
            "ir_version": 5,
            "host": {"os": "linux", "arch": "x86_64"},
            "workspace": {"span": {"file": "grip.ts", "line": 1}, "outputs": [profile]},
        }))
        result = grip("plan", "--ir", str(input_file), cwd=repo)
        assert result.returncode != 0
        assert "error[E130]" in result.stderr, result.stderr
        assert "grip.ts:3" in result.stderr
        assert "repository-relative" in result.stderr
        assert "E124" not in result.stderr, "invalid origin precedes executor refusal"
    assert not (sandbox / ".local/share/gripsack/current").exists()

def test_duplicate_profile_destination_names_both_owners_before_any_executor(sandbox):
    """A1-11: one path has one owner — the module grammar's E111 race
    rule extended to workspace profile files, labeling both files."""
    repo = sandbox / "duplicate-destination"
    repo.mkdir()

    def owned(line, path):
        return {
            "span": {"file": "grip.ts", "line": line},
            "source": {"kind": "repo_file", "path": "cfg/tool.conf"},
            "content": {"kind": "identity"},
            "destination": {"kind": "tracked_copy", "path": path},
        }

    profile = {
        "kind": "profile", "name": "dotfiles", "span": {"file": "grip.ts", "line": 2},
        "files": [owned(3, "~/.config/tool.conf"), owned(7, "~/.CONFIG/TOOL.CONF")],
    }
    path = repo / "workspace.ir.json"
    path.write_text(json.dumps({
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {"span": {"file": "grip.ts", "line": 1}, "outputs": [profile]},
    }))
    result = grip("plan", "--ir", str(path), cwd=repo)
    assert result.returncode != 0
    assert "error[E111]" in result.stderr, result.stderr
    assert "grip.ts:3" in result.stderr and "grip.ts:7" in result.stderr
    assert "E124" not in result.stderr, "ownership conflicts precede executor refusal"
    assert not (sandbox / ".local/share/gripsack/current").exists()

def test_help_text_agrees_between_terminal_and_json_surfaces(sandbox):
    """A1-06 named case: a diagnostic carrying `help` (E111) shows the
    same help text on both surfaces — the fact cannot drift."""
    repo = write_repo(
        sandbox,
        "help-parity",
        'import { defineWorkspace, workspace, profile, file, repoFile, identity, '
        'trackedCopyTo } from "@gripsack/core";\n'
        "export default defineWorkspace(() => workspace({ outputs: [\n"
        '  profile("dotfiles", { files: [\n'
        '    file({ source: repoFile("cfg/a.conf"), content: identity(),\n'
        '           destination: trackedCopyTo("~/.config/a.conf") }),\n'
        '    file({ source: repoFile("cfg/b.conf"), content: identity(),\n'
        '           destination: trackedCopyTo("~/.CONFIG/A.CONF") }),\n'
        "  ] }),\n"
        "] }));\n",
    )
    terminal, doc = check_both(repo)
    assert terminal.returncode == 1
    assert "error[E111]" in terminal.stderr
    assert_same_facts(terminal, doc)
    helps = [d["help"] for d in doc["diagnostics"] if d["code"] == "E111"]
    assert helps and helps[0], "the JSON surface must carry the E111 help text"
    assert helps[0] in terminal.stderr
    assert not (sandbox / ".config").exists()



def test_user_exception_stays_a_traceback_on_both_surfaces(sandbox):
    """A1-06 boundary: a user exception is a real defect, not an
    authoring typo — both surfaces pass the traceback through and the
    JSON document honestly carries no synthetic E130."""
    repo = write_repo(
        sandbox,
        "user-throw",
        'import { defineWorkspace } from "@gripsack/core";\n'
        "export default defineWorkspace(() => {\n"
        '  throw new Error("user boom");\n'
        "});\n",
    )
    terminal = grip("check", cwd=repo)
    machine = grip("check", "--json", cwd=repo)
    assert terminal.returncode != 0
    assert machine.returncode == terminal.returncode
    assert "user boom" in terminal.stderr
    assert "user boom" in machine.stderr
    assert "frontend eval failed" in terminal.stderr
    assert "frontend eval failed" in machine.stderr
    assert "E130" not in terminal.stderr + terminal.stdout
    doc = json.loads(machine.stdout)
    assert doc["version"] == 1
    assert doc["ok"] is False
    assert doc["diagnostics"] == [], "a traceback must not mint synthetic diagnostics"


def test_engine_error_stays_a_traceback_on_both_surfaces(sandbox):
    """Engine errors (TypeError) are defects in the authoring program,
    not invalid declarations — same traceback contract, never E130."""
    repo = write_repo(
        sandbox,
        "type-error",
        'import { defineWorkspace, workspace } from "@gripsack/core";\n'
        "export default defineWorkspace(() => {\n"
        '  const nothing = JSON.parse("null")\n'
        "  return workspace({ outputs: [nothing.outputs] })\n"
        "});\n",
    )
    terminal = grip("check", cwd=repo)
    machine = grip("check", "--json", cwd=repo)
    assert terminal.returncode != 0
    assert machine.returncode == terminal.returncode
    assert "TypeError" in terminal.stderr
    assert "TypeError" in machine.stderr
    assert "E130" not in terminal.stderr + terminal.stdout
    doc = json.loads(machine.stdout)
    assert doc["ok"] is False
    assert doc["diagnostics"] == []


def test_unallocated_frontend_diagnostic_cannot_masquerade_as_core_error(sandbox):
    """A user-thrown error cannot invent a stable E-code on either CLI surface."""
    repo = write_repo(
        sandbox,
        "forged-code",
        'import { defineWorkspace, workspace } from "@gripsack/core";\n'
        "export default defineWorkspace(() => {\n"
        '  const failure = new Error("unallocated diagnostic");\n'
        '  failure.name = "DiagnosticError";\n'
        '  (failure as Error & { diagnostic: unknown }).diagnostic = {\n'
        '    code: "E999", severity: "error", message: "forged E999", labels: []\n'
        "  };\n"
        "  throw failure;\n"
        "});\n",
    )
    terminal = grip("check", cwd=repo)
    machine = grip("check", "--json", cwd=repo)
    assert terminal.returncode != 0
    assert machine.returncode == terminal.returncode
    assert "unallocated diagnostic" in terminal.stderr
    assert "unallocated diagnostic" in machine.stderr
    assert "frontend eval failed" in terminal.stderr
    assert "frontend eval failed" in machine.stderr
    assert "error[E999]:" not in terminal.stderr
    assert json.loads(machine.stdout)["diagnostics"] == []
