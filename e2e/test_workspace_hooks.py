"""Workspace hooks use the same durable activation and replay protocol as modules."""
import json
import shlex

import pytest
from conftest import grip


IMPORTS = '''import {
  defineWorkspace, workspace, profile, file, literalText, trackedCopyTo,
  hook, exec, lit,
} from "@gripsack/core";
'''


def declare(repo, hooks, *, content="active", selected=None):
    repo.mkdir(exist_ok=True)
    definitions = []
    names = []
    for name, trigger, argv in hooks:
        definitions.append(
            f'const {name} = hook({json.dumps(name)}, {{trigger: {json.dumps(trigger)}, '
            'run: exec({argv: [' + ','.join(f'lit({json.dumps(arg)})' for arg in argv) + ']})});'
        )
        names.append(name)
    profiles = selected if selected is not None else names
    (repo / "gripsack.ts").write_text(
        IMPORTS + '\n'.join(definitions)
        + '\nexport default defineWorkspace(() => workspace({outputs: ['
        + ','.join(names)
        + (',' if names else '')
        + 'profile("personal", {files: [file({content: literalText('
        + json.dumps(content)
        + '), destination: trackedCopyTo("~/.workspace-hook-owned")})], hooks: ['
        + ','.join(json.dumps(name) for name in profiles) + ']})]}));\n'
    )


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def outcomes(repo):
    return json.loads(run(repo, "hooks", "list", "--json").stdout)["intents"]


def current(sandbox):
    return (sandbox / ".local/share/gripsack/current").readlink()


def append(path, label="", exit_code=0):
    script = (
        f"printf '%s:%s:%s\\n' {shlex.quote(label)} "
        '"$GRIPSACK_ACTIVATION_INTENT_ID" "$GRIPSACK_ACTIVATION_ATTEMPT" '
        f'>> {shlex.quote(str(path))}; exit {exit_code}'
    )
    return ["/bin/sh", "-c", script]


def test_public_literal_hook_executes_and_readonly_commands_do_not(sandbox):
    repo = sandbox / "hooks"
    effect = sandbox / "effects"
    declare(repo, [("after", "post_link", ["/usr/bin/true"]),
                   ("effect", "post_activate", append(effect))])
    run(repo, "check")
    run(repo, "plan")
    assert not effect.exists()
    assert not (sandbox / ".workspace-hook-owned").exists()
    run(repo, "apply")
    rows = outcomes(repo)
    assert len(rows) == 2
    assert all(row["state"] == {"kind": "succeeded", "attempt": 1} for row in rows)
    assert (sandbox / ".workspace-hook-owned").read_text() == "active"
    before = effect.read_bytes()
    generation = current(sandbox)
    run(repo, "apply")
    assert effect.read_bytes() == before
    assert current(sandbox) == generation
    assert outcomes(repo) == rows


def test_hooks_run_after_flip_in_declared_order_and_inert_hooks_stay_inert(sandbox):
    repo = sandbox / "hooks"
    effect = sandbox / "order"
    owned = sandbox / ".workspace-hook-owned"
    # Lexical hook name order deliberately differs from profile declaration order.
    first = ["/bin/sh", "-c", f'test "$(cat {shlex.quote(str(owned))})" = active && '
             + append(effect, "first")[2]]
    declare(repo, [("zfirst", "post_link", first),
                   ("asecond", "post_activate", append(effect, "second")),
                   ("unused", "post_link", append(effect, "must-not-run"))],
            selected=["zfirst", "asecond"])
    run(repo, "apply")
    assert [line.split(':')[0] for line in effect.read_text().splitlines()] == ["first", "second"]
    assert all(row["state"]["kind"] == "succeeded" for row in outcomes(repo))


def test_failed_hook_is_degraded_without_rollback_or_automatic_retry(sandbox):
    repo = sandbox / "hooks"
    effect = sandbox / "failed"
    declare(repo, [("after", "post_link", append(effect, exit_code=9))])
    run(repo, "apply")
    rows = outcomes(repo)
    assert rows[0]["state"] == {"kind": "failed", "attempt": 1}
    assert rows[0]["processes"][0]["exit_code"] == 9
    assert (sandbox / ".workspace-hook-owned").read_text() == "active"
    generation = current(sandbox)
    before = effect.read_bytes()
    run(repo, "apply")
    assert current(sandbox) == generation
    assert effect.read_bytes() == before
    assert outcomes(repo) == rows
    refused = grip("rollback", cwd=repo)
    assert refused.returncode != 0
    assert "nothing to roll back to" in refused.stderr
    assert current(sandbox) == generation
    assert effect.read_bytes() == before
    assert outcomes(repo) == rows
    # A different generation can activate without retrying the failed hook.
    # Explicit rollback is deliberate reactivation, not automatic recovery.
    declare(repo, [], content="replacement")
    run(repo, "apply")
    assert current(sandbox).name != generation.name
    assert (sandbox / ".workspace-hook-owned").read_text() == "replacement"
    assert effect.read_bytes() == before
    run(repo, "rollback", generation.name)
    assert current(sandbox).name == generation.name
    assert (sandbox / ".workspace-hook-owned").read_text() == "active"
    assert len(effect.read_text().splitlines()) == 2
    restored = outcomes(repo)
    assert len(restored) == 2
    assert len({row["intent"] for row in restored}) == 2
    assert len({row["activation"] for row in restored}) == 2
    assert all(row["state"] == {"kind": "failed", "attempt": 1} for row in restored)


@pytest.mark.parametrize("cut,before_count,attempt,after_count", [
    ("hook-after-start", 0, 2, 1),
    ("hook-after-effect", 1, 2, 2),
    ("hook-after-receipt", 1, 1, 1),
    ("hooks-before-cleanup", 1, 1, 1),
])
def test_workspace_hook_replay_keeps_identity_and_durable_outcomes(
        sandbox, monkeypatch, cut, before_count, attempt, after_count):
    repo = sandbox / "hooks"
    effect = sandbox / "effects"
    declare(repo, [("after", "post_link", append(effect))])
    monkeypatch.setenv("GRIPSACK_ACTIVATION_INTENT_ID", "forged")
    monkeypatch.setenv("GRIPSACK_ACTIVATION_ATTEMPT", "999")
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", cut)
    assert grip("apply", cwd=repo).returncode != 0
    before = effect.read_text().splitlines() if effect.exists() else []
    assert len(before) == before_count
    pending = outcomes(repo)
    identity = pending[0]["intent"]
    assert identity != "forged"
    generation = current(sandbox)
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    run(repo, "apply")
    assert current(sandbox) == generation
    lines = effect.read_text().splitlines()
    assert len(lines) == after_count
    assert lines[-1] == f":{identity}:{attempt}"
    final = outcomes(repo)
    assert len(final) == 1
    assert final[0]["intent"] == identity
    assert final[0]["state"] == {"kind": "succeeded", "attempt": attempt}
    assert not final[0]["pending"]


def test_removal_and_rollback_use_retained_hook_not_current_source(sandbox):
    repo = sandbox / "hooks"
    effect = sandbox / "removed"
    declare(repo, [("removed", "on_remove", append(effect, "old")),
                   ("after", "post_link", ["/usr/bin/true"])])
    run(repo, "apply")
    original = current(sandbox)
    assert not effect.exists()
    # Removing the whole owner fires on_remove; merely editing its hook list does not.
    (repo / "gripsack.ts").write_text(
        IMPORTS + 'export default defineWorkspace(() => workspace({outputs: [profile("empty", {})]}));\n'
    )
    run(repo, "apply")
    assert effect.read_text().splitlines()[0].startswith("old:")
    assert not (sandbox / ".workspace-hook-owned").exists()
    run(repo, "rollback", original.name)
    assert (sandbox / ".workspace-hook-owned").read_text() == "active"
    assert len(effect.read_text().splitlines()) == 1
    restored = [row for row in outcomes(repo) if row["contributors"] == [
        {"module": "personal", "trigger": "post_link"}]]
    assert len(restored) == 2
    assert len({row["intent"] for row in restored}) == 2


def test_workspace_actions_cannot_enter_legacy_plan_version(sandbox, monkeypatch):
    repo, effect = sandbox / "hooks", sandbox / "effects"
    declare(repo, [("after", "post_link", append(effect))])
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "before-adapters")
    assert grip("apply", cwd=repo).returncode != 0
    home = sandbox / ".local/share/gripsack"
    pointer_path = home / "activation.json"
    pointer = json.loads(pointer_path.read_text())
    assert pointer["version"] == 2
    plan_path = home / "activation" / pointer["instance"] / "plan.json"
    plan = json.loads(plan_path.read_text())
    assert plan["version"] == 2
    plan["version"] = 1
    pointer["version"] = 1
    plan_path.write_text(json.dumps(plan))
    pointer_path.write_text(json.dumps(pointer))
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    refused = grip("apply", cwd=repo)
    assert refused.returncode != 0
    assert not effect.exists()
    assert pointer_path.exists()
    assert (sandbox / ".workspace-hook-owned").read_text() == "active"


def test_workspace_manifest_requires_versioned_envelope_before_rollback(sandbox):
    repo, effect = sandbox / "hooks", sandbox / "effects"
    declare(repo, [("after", "post_link", append(effect))])
    run(repo, "apply")
    original = effect.read_bytes()
    path = sandbox / ".local/share/gripsack/current/manifest.json"
    manifest = json.loads(path.read_text())
    assert manifest["version"] == 2 and "number" not in manifest
    path.write_text(json.dumps(manifest["generation"]))
    refused = grip("rollback", current(sandbox).name, cwd=repo)
    assert refused.returncode != 0
    assert effect.read_bytes() == original


def test_package_hook_rollback_reuses_frozen_package_and_context(sandbox):
    from conftest import make_toolchain_tarball
    effect = sandbox / "package-effects"
    script = b"#!/bin/sh\n" + append(effect, "package")[2].encode() + b"\n"
    archive = make_toolchain_tarball(sandbox / "hooks.tar.gz", {"bin/probe": script})
    repo = sandbox / "hooks"
    repo.mkdir()
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, profile, hook, exec, packageCommand,
      pkg, provider, tarball,
    } from "@gripsack/core";
    export default defineWorkspace(ctx => workspace({outputs: [
      pkg("tools", {producer: provider(tarball(''' + json.dumps(archive.as_uri()) + ''')),
        commands: {probe: "bin/probe"}, target: {os: ctx.facts.os, arch: ctx.facts.arch},
        layout: {kind: "relocatable"}}),
      hook("after", {trigger: "post_link", run: exec({argv: [packageCommand("tools", "probe")]})}),
      profile("personal", {hooks: ["after"]}),
    ]}));\n''')
    run(repo, "update", "tools")
    run(repo, "apply")
    original = current(sandbox)
    assert len(effect.read_text().splitlines()) == 1
    (repo / "gripsack.ts").write_text(
        IMPORTS + 'export default defineWorkspace(() => workspace({outputs: [profile("empty", {})]}));\n'
    )
    archive.unlink()
    run(repo, "apply")
    run(repo, "rollback", original.name)
    assert len(effect.read_text().splitlines()) == 2
    assert all(line.startswith("package:") for line in effect.read_text().splitlines())
    assert all(row["state"]["kind"] == "succeeded" for row in outcomes(repo))


def test_replay_refuses_changed_retained_context_before_effect(sandbox, monkeypatch):
    from pathlib import Path
    repo, effect = sandbox / "hooks", sandbox / "effects"
    declare(repo, [("after", "post_link", append(effect))])
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-start")
    assert grip("apply", cwd=repo).returncode != 0
    home = sandbox / ".local/share/gripsack"
    manifest = json.loads((home / "current/manifest.json").read_text())["generation"]
    action = manifest["modules"]["personal"]["intents"][0]["action"]
    context = Path(action["context"])
    context.chmod(0o600)
    context.write_bytes(context.read_bytes() + b"\n")
    identity = outcomes(repo)[0]["intent"]
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    run(repo, "apply")
    assert not effect.exists()
    row = outcomes(repo)[0]
    assert row["intent"] == identity
    assert row["state"] == {"kind": "failed", "attempt": 2}
    assert not row["processes"]
    assert row["failure"]["kind"] == "admission"
    assert (sandbox / ".workspace-hook-owned").read_text() == "active"


def test_run_bash_package_interpreter_freezes_argv_and_replays_identity(sandbox, monkeypatch):
    import hashlib
    from conftest import make_toolchain_tarball
    effect = sandbox / "bash-effects"
    directory = sandbox / "bash-working-directory"
    directory.mkdir()
    # A real package-owned Bash launcher: the exact script and its platform
    # interpreter are byte-bound by native admission, and it forwards the
    # frozen strict options, -c body and $0 label to actual Bash.
    launcher = b'#!/bin/bash\nexec /bin/bash "$@"\n'
    archive = make_toolchain_tarball(sandbox / "bash.tar.gz", {"bin/bash": launcher})
    digest = hashlib.sha256(launcher).hexdigest()
    body = (
        '[[ -n $BASH_VERSION ]] || exit 20; '
        '[[ $0 == gripsack-bash ]] || exit 21; '
        '[[ $- == *e* && $- == *u* ]] || exit 22; '
        '[[ :$SHELLOPTS: == *:pipefail:* ]] || exit 23; '
        '[[ $MARKER == "two words" && $PWD == "$EXPECTED_CWD" ]] || exit 24; '
        'printf "%s:%s:%s:%s\\n" "$MARKER" "$0" '
        '"$GRIPSACK_ACTIVATION_INTENT_ID" "$GRIPSACK_ACTIVATION_ATTEMPT" >> "$EFFECT"'
    )
    repo = sandbox / "bash-hooks"
    repo.mkdir()
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, profile, hook, runBash, packageCommand, lit,
      pkg, provider, tarball,
    } from "@gripsack/core";
    export default defineWorkspace(ctx => workspace({outputs: [
      pkg("shell", {producer: provider(tarball(''' + json.dumps(archive.as_uri()) + ''')),
        commands: {bash: "bin/bash"}, target: {os: ctx.facts.os, arch: ctx.facts.arch},
        layout: {kind: "relocatable"}}),
      hook("after", {trigger: "post_link", run: runBash({
        interpreter: packageCommand("shell", "bash", ''' + json.dumps(digest) + '''),
        body: ''' + json.dumps(body) + ''',
        cwd: lit(''' + json.dumps(str(directory)) + '''),
        env: {MARKER: lit("two words"), EFFECT: lit(''' + json.dumps(str(effect)) + '''),
          EXPECTED_CWD: lit(''' + json.dumps(str(directory)) + ''')}
      })}),
      profile("personal", {hooks: ["after"]}),
    ]}));\n''')
    run(repo, "update", "shell")
    run(repo, "check")
    assert not effect.exists()
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-effect")
    assert grip("apply", cwd=repo).returncode != 0
    pending = outcomes(repo)
    assert len(pending) == 1
    assert pending[0]["state"] == {"kind": "started", "attempt": 1}
    identity = pending[0]["intent"]
    assert effect.read_text().splitlines() == [f"two words:gripsack-bash:{identity}:1"]
    generation = current(sandbox)
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    run(repo, "apply")
    expected = [f"two words:gripsack-bash:{identity}:{attempt}" for attempt in (1, 2)]
    assert effect.read_text().splitlines() == expected
    final = outcomes(repo)
    assert len(final) == 1
    assert final[0]["intent"] == identity
    assert final[0]["state"] == {"kind": "succeeded", "attempt": 2}
    assert not final[0]["pending"]
    assert current(sandbox) == generation
    run(repo, "apply")
    assert effect.read_text().splitlines() == expected
    assert outcomes(repo) == final
