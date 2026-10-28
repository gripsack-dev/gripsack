"""Workspace preparation gates and lifecycle checks.

Universal step ordering, shell-build timing and module verifier receipts were
removed with that executor. Task ordering lives in test_task_graphs; builder
execution belongs to the BuildKit harness, not host-shell stand-ins.
"""
import json

from conftest import grip, make_workspace_repo


def checked_repo(sandbox, command):
    return make_workspace_repo(sandbox / "env", {
        "validate": '''import {check,exec,lit} from "@gripsack/core";
export default check("validate", {subject:"demo", run:exec({argv:[
  lit("/bin/sh"),lit("-c"),lit(''' + json.dumps(command) + ''')
]})});''',
        "demo": '''import {profile,file,literalText,trackedCopyTo} from "@gripsack/core";
export default profile("demo", {files:[file({content:literalText("candidate\\n"),
 destination:trackedCopyTo("~/.checked"),
 checks:[{check:"validate",subject:"rendered",stage:"pre_flip"}]})]});''',
    })


def test_failed_preflip_check_fails_every_retry(sandbox):
    repo = checked_repo(sandbox, "exit 17")
    for _ in range(2):
        result = grip("apply", cwd=repo)
        assert result.returncode != 0, result.stdout + result.stderr
        assert not (sandbox / ".checked").exists()
        assert not (sandbox / ".local/share/gripsack/current").exists()


def test_preflip_check_runs_on_each_apply_without_cutting_a_generation(sandbox):
    counter = sandbox / "checks"
    repo = checked_repo(sandbox, 'printf "checked\\n" >> "$HOME/checks"')
    first = grip("apply", cwd=repo)
    assert first.returncode == 0, first.stdout + first.stderr
    current = sandbox / ".local/share/gripsack/current"
    generation = current.readlink()
    assert counter.read_text() == "checked\n"
    second = grip("apply", cwd=repo)
    assert second.returncode == 0, second.stdout + second.stderr
    assert current.readlink() == generation
    # Profile checks are lifecycle gates, not cached module-verifier receipts.
    assert counter.read_text() == "checked\nchecked\n"


def test_typo_field_cannot_become_a_prune_instruction(sandbox):
    repo = make_workspace_repo(sandbox / "env", '''import {
 profile,file,literalText,trackedCopyTo} from "@gripsack/core";
export default profile("demo", {files:[file({content:literalText("original"),
 destination:trackedCopyTo("~/.config-file")})]});''')
    first = grip("apply", cwd=repo)
    assert first.returncode == 0, first.stdout + first.stderr
    current = sandbox / ".local/share/gripsack/current"
    previous = current.readlink()
    declaration = repo / "outputs/hello.ts"
    declaration.write_text(declaration.read_text().replace("files:", "fiels:"))
    result = grip("apply", cwd=repo)
    assert result.returncode != 0
    assert (sandbox / ".config-file").read_text() == "original"
    assert current.readlink() == previous
