"""Generation environment exports and durable activation hooks."""

import subprocess

from conftest import native_target, grip, make_toolchain_tarball, make_workspace_repo, remove_output


def test_exported_env_profile_tracks_the_generation(sandbox):
    payload = make_toolchain_tarball(sandbox / "myenv" / "zed.tar.gz", {"bin/zed": b"#!/bin/sh\nprintf 'exported command\\n'\n"})
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ environment, file, fileFetch, identity, lit, pkg, profile, provider, repoFile, trackedCopyTo }} from "@gripsack/core";

export default [
pkg("zed-tool", {{ producer: provider(fileFetch("{payload.name}")), commands: {{ zed: "bin/zed" }}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
environment("zed-environment", {{ packages: ["zed-tool"], target: {native_target()}, env: {{ EDITOR: lit("zed") }} }}),
profile("zed", {{ files: [
    file({{ source: repoFile("configs/zed/a"), content: identity(), destination: trackedCopyTo("~/.config/zed/a") }})
  ], environment: "zed-environment" }})];
""",
    )
    (repo / "configs" / "zed").mkdir(parents=True)
    (repo / "configs" / "zed" / "a").write_text("a\n")
    # the profile is generation-local since 0.22, sourced through the
    # current symlink (plan/0025 §C): it activates with the flip
    profile = sandbox / ".local/share/gripsack/current/env/profile.sh"
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    exported = subprocess.run(
        ["/bin/sh", "-c", '. "$1"; printf "%s\\n" "$EDITOR"; zed',
         "profile-test", str(profile)],
        capture_output=True, text=True, timeout=20,
    )
    assert exported.returncode == 0, exported.stderr
    assert exported.stdout == "zed\nexported command\n"

    # drop the env declaration — the profile must not go stale
    (repo / "outputs" / "hello.ts").write_text(
        """
import { file, identity, profile, repoFile, trackedCopyTo } from "@gripsack/core";

export default profile("zed", { files: [
    file({ source: repoFile("configs/zed/a"), content: identity(), destination: trackedCopyTo("~/.config/zed/a") })
  ] });
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    # no env contributions in this generation → no profile in it; the
    # generation-1 file still exists behind the old directory, but
    # current/ no longer resolves one
    assert not profile.exists()


def test_failed_post_activation_hook_does_not_fail_apply(sandbox):
    """A failed post-activation command cannot roll back committed files."""
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, hook, identity, profile, repoFile, exec, lit, trackedCopyTo } from "@gripsack/core";

export default [hook("daemon-hook-0", { trigger: "post_activate", run: exec({ argv: [lit("/bin/sh"), lit("-c"), lit("exit 1")] }) }),
profile("daemon", { files: [
    file({ source: repoFile("configs/daemon/a"), content: identity(), destination: trackedCopyTo("~/.config/daemon/a") })
  ], hooks: ["daemon-hook-0"] })];
""",
    )
    (repo / "configs" / "daemon").mkdir(parents=True)
    (repo / "configs" / "daemon" / "a").write_text("a\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config/daemon/a").read_text() == "a\n"
    assert (sandbox / ".local/share/gripsack/current").exists()


def test_crash_before_adapters_resumes_next_run(sandbox, monkeypatch):
    """0032: a kill between the flip and the adapters leaves a durable
    pending record; the next run (even a satisfied one) resumes the
    intents. The pending write is PRE-flip — the TLA+ mutant with a
    post-flip write violates NoSilentSkip."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("a\n")
    marker = sandbox / "hook-ran"
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ file, hook, identity, profile, repoFile, exec, lit, trackedCopyTo }} from "@gripsack/core";

export default [hook("demo-hook-0", {{ trigger: "post_activate", run: exec({{ argv: [lit("/bin/sh"), lit("-c"), lit("touch {marker}")] }}) }}),
profile("demo", {{ files: [
    file({{ source: repoFile("configs/demo/a.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.toml") }})
  ], hooks: ["demo-hook-0"] }})];
""",
    )

    # the run dies after the flip, before adapters
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "before-adapters")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, "the crash hook must kill the run"
    assert not marker.exists(), "the hook must NOT have run"
    pending = sandbox / ".local/share/gripsack/activation.json"
    assert pending.exists(), "the pending record is durable"

    # the next run resumes the intents — even though nothing changed
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert marker.exists(), "the resumed hook ran"
    assert not pending.exists(), "the record drained"


def test_stale_pending_record_is_discarded_never_run(sandbox, monkeypatch):
    """0032: a pending record naming a generation that never became
    current (crash between pending-write and flip) is discarded by
    the next run — adapters never run for non-current state."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("a\n")
    marker = sandbox / "hook-ran"
    repo = make_workspace_repo(
        sandbox / "myenv",
        f"""
import {{ file, hook, identity, profile, repoFile, exec, lit, trackedCopyTo }} from "@gripsack/core";

export default [hook("demo-hook-0", {{ trigger: "post_activate", run: exec({{ argv: [lit("/bin/sh"), lit("-c"), lit("touch {marker}")] }}) }}),
profile("demo", {{ files: [
    file({{ source: repoFile("configs/demo/a.toml"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.toml") }})
  ], hooks: ["demo-hook-0"] }})];
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert marker.exists()
    marker.unlink()
    current = sandbox / ".local/share/gripsack/current"
    previous = current.readlink()

    # Preserve an authentic pending plan, but model its generation never
    # becoming current. No legacy action or hand-crafted plan is admitted.
    (confdir / "a.toml").write_text("candidate\n")
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "before-adapters")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, "the crash hook must kill the run"
    assert not marker.exists()
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    pending = sandbox / ".local/share/gripsack/activation.json"
    assert pending.exists()
    current.unlink()
    current.symlink_to(previous)
    (confdir / "a.toml").write_text("a\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not marker.exists(), "a non-current generation's adapters never run"
    assert not pending.exists(), "the stale record is discarded"


def test_on_remove_hook_fires_at_removal_not_install(sandbox):
    """0035 F9: an on_remove intent runs when the module leaves the
    repo — not on install."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    marker = sandbox / "removed-hook-ran"
    repo = make_workspace_repo(
        sandbox / "myenv",
        {
            "demo": f"""
import {{ file, hook, identity, profile, repoFile, exec, lit, trackedCopyTo }} from "@gripsack/core";

export default [hook("demo-hook-0", {{ trigger: "on_remove", run: exec({{ argv: [lit("/bin/sh"), lit("-c"), lit("touch {marker}")] }}) }}),
profile("demo", {{ files: [
    file({{ source: repoFile("configs/demo/a.conf"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.conf") }})
  ], hooks: ["demo-hook-0"] }})];
"""
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not marker.exists(), "on_remove must not fire at install"

    make_workspace_repo(repo, {"keep": 'import { profile } from "@gripsack/core"; export default profile("keep", {});'})
    remove_output(repo, "demo")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert marker.exists(), "on_remove fires when the module leaves"
    assert not (sandbox / ".config/demo/a.conf").exists(), "the file pruned too"


def test_activation_record_failure_compensates(sandbox):
    """0035 F5: a fallible write between the first mutation and the
    flip is INSIDE the transaction boundary — the run fails AND the
    destination returns to the previous generation's content."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("v1\n")
    repo = make_workspace_repo(
        sandbox / "myenv",
        {
            "demo": f"""
import {{ file, hook, identity, profile, repoFile, exec, lit, trackedCopyTo }} from "@gripsack/core";

export default [hook("demo-hook-0", {{ trigger: "post_activate", run: exec({{ argv: [lit("/bin/sh"), lit("-c"), lit("true")] }}) }}),
profile("demo", {{ files: [
    file({{ source: repoFile("configs/demo/a.conf"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.conf") }})
  ], hooks: ["demo-hook-0"] }})];
"""
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".config/demo/a.conf"
    assert dest.read_text() == "v1\n"

    # the reviewer's fault: the activation-record pathname is a
    # directory — the pending write must fail
    record = sandbox / ".local/share/gripsack/activation.json"
    record.mkdir()
    (confdir / "a.conf").write_text("v2\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, "the pending write must fail"
    assert dest.read_text() == "v1\n", (
        "the failure compensated — live state matches generation 1"
    )
    assert (
        (sandbox / ".local/share/gripsack/current").readlink()
        .name
        == "1"
    ), "generation 1 is still current"


def test_rollback_runs_the_target_generations_hooks(sandbox):
    """0037: rollback restores the RUNNING environment, not just the
    bytes — the target generation's intents re-run."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("v1\n")
    marker = sandbox / "hook-log"
    repo = make_workspace_repo(
        sandbox / "myenv",
        {
            "demo": f"""
import {{ file, hook, identity, profile, repoFile, exec, lit, trackedCopyTo }} from "@gripsack/core";

export default [hook("demo-hook-0", {{ trigger: "post_activate", run: exec({{ argv: [lit("/bin/sh"), lit("-c"), lit("echo activated >> {marker}")] }}) }}),
profile("demo", {{ files: [
    file({{ source: repoFile("configs/demo/a.conf"), content: identity(), destination: trackedCopyTo("~/.config/demo/a.conf") }})
  ], hooks: ["demo-hook-0"] }})];
"""
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    (confdir / "a.conf").write_text("v2\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    before = marker.read_text().count("activated")

    out = grip("rollback", "1", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert marker.read_text().count("activated") == before + 1, (
        "the target generation's hook re-ran"
    )


def test_rollback_fires_on_remove_for_undeclared_modules(sandbox):
    """0037: a module present now but absent from the rollback target
    is undeclared BY the rollback — its on_remove hook fires."""
    marker = sandbox / "remove-hook-log"
    repo = make_workspace_repo(
        sandbox / "myenv",
        {
            "demo": """
import { profile } from "@gripsack/core";

export default profile("demo", {  });
""",
            "ephemeral": f"""
import {{ hook, profile, exec, lit }} from "@gripsack/core";

export default [hook("ephemeral-hook-0", {{ trigger: "on_remove", run: exec({{ argv: [lit("/bin/sh"), lit("-c"), lit("echo removed >> {marker}")] }}) }}),
profile("ephemeral", {{ hooks: ["ephemeral-hook-0"] }})];
""",
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)  # gen 1: both modules
    assert out.returncode == 0, out.stderr
    remove_output(repo, "ephemeral")
    out = grip("apply", "--host", "testhost", cwd=repo)  # gen 2: demo only
    assert out.returncode == 0, out.stderr
    assert marker.exists(), "on_remove fired at the undeclaring apply"

    marker.unlink()
    out = grip("rollback", "1", cwd=repo)  # back to a world WITH ephemeral
    assert out.returncode == 0, out.stderr
    assert not marker.exists(), "no removal hook when the module returns"

    out = grip("rollback", "2", cwd=repo)  # rolling forward undeclares it
    assert out.returncode == 0, out.stderr
    assert marker.exists(), "on_remove fires for a module the rollback undeclares"






def test_profile_hook_runs_after_activation(sandbox):
    """A declared workspace hook executes its real post-activate command."""
    repo = make_workspace_repo(
        sandbox / "myenv",
        """
import { file, hook, identity, profile, repoFile, exec, lit, symlinkTo } from "@gripsack/core";

export default [hook("demo-hook-0", { trigger: "post_activate", run: exec({ argv: [lit("/bin/sh"), lit("-c"), lit("echo post-activate > ~/hook-ran")] }) }),
profile("demo", { files: [
    file({ source: repoFile("configs/demo/a.txt"), content: identity(), destination: symlinkTo("~/.config/demo/a.txt") })
  ], hooks: ["demo-hook-0"] })];
""",
    )
    confdir = repo / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / "hook-ran").read_text() == "post-activate\n"
