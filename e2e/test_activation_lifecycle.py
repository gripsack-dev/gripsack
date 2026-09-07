"""Generation environment exports and durable activation hooks."""

import os
from conftest import grip, make_env_repo, remove_module


def test_exported_env_profile_tracks_the_generation(sandbox):
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("zed", {
  config: { "configs/zed/a": trackedCopy("~/.config/zed/a") },
  env: { EDITOR: "zed", "PATH+": "{store}/bin" },
});
""",
    )
    (repo / "configs" / "zed").mkdir(parents=True)
    (repo / "configs" / "zed" / "a").write_text("a\n")
    # the profile is generation-local since 0.22, sourced through the
    # current symlink (plan/0025 §C): it activates with the flip
    profile = sandbox / ".local/share/gripsack/current/env/profile.sh"
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    text = profile.read_text()
    assert 'export EDITOR="zed"' in text
    store_bin = next(
        line for line in text.splitlines() if line.startswith("export PATH=")
    )
    assert "/bin:${PATH}" in store_bin
    assert "/store/" in store_bin and "-zed/bin:" in store_bin

    # drop the env declaration — the profile must not go stale
    (repo / "modules" / "hello.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("zed", {
  config: { "configs/zed/a": trackedCopy("~/.config/zed/a") },
});
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    # no env contributions in this generation → no profile in it; the
    # generation-1 file still exists behind the old directory, but
    # current/ no longer resolves one
    assert not profile.exists()


def test_service_intent_runs_the_adapter_without_failing_apply(sandbox):
    """systemd-user adapter: no systemctl/user bus in the sandbox —
    the intent must degrade to a warning, never a failed apply
    (0001 §3.8: never roll back on post-activation failure)."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, service, trackedCopy } from "@gripsack/core";

export default module("daemon", {
  config: { "configs/daemon/a": trackedCopy("~/.config/daemon/a") },
  activate: [service("my-daemon.service")],
});
""",
    )
    (repo / "configs" / "daemon").mkdir(parents=True)
    (repo / "configs" / "daemon" / "a").write_text("a\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "my-daemon.service" in out.stdout


def test_crash_before_adapters_resumes_next_run(sandbox, monkeypatch):
    """0032: a kill between the flip and the adapters leaves a durable
    pending record; the next run (even a satisfied one) resumes the
    intents. The pending write is PRE-flip — the TLA+ mutant with a
    post-flip write violates NoSilentSkip."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("a\n")
    marker = sandbox / "hook-ran"
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ module, trackedCopy, customHook }} from "@gripsack/core";

export default module("demo", {{
  config: {{ "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") }},
  activate: [customHook("touch {marker}")],
}});
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
    assert "resumed activation" in out.stdout, out.stdout
    assert marker.exists(), "the resumed hook ran"
    assert not pending.exists(), "the record drained"


def test_stale_pending_record_is_discarded_never_run(sandbox):
    """0032: a pending record naming a generation that never became
    current (crash between pending-write and flip) is discarded by
    the next run — adapters never run for non-current state."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("a\n")
    marker = sandbox / "hook-ran"
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ module, trackedCopy, customHook }} from "@gripsack/core";

export default module("demo", {{
  config: {{ "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") }},
  activate: [customHook("touch {marker}")],
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert marker.exists()

    # hand-craft a pending record naming a future (never-committed)
    # generation — as if a run died between the pending write and
    # the flip
    marker.unlink()
    import json

    pending = sandbox / ".local/share/gripsack/activation.json"
    pending.write_text(
        json.dumps(
            {
                "generation": 99,
                "intents": [
                    {
                        "module": "demo",
                        "action": {"kind": "custom_shell", "script": f"touch {marker}"},
                    }
                ],
            }
        )
    )
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
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "demo": f"""
import {{ module, trackedCopy, customHook }} from "@gripsack/core";

export default module("demo", {{
  config: {{ "configs/demo/a.conf": trackedCopy("~/.config/demo/a.conf") }},
  activate: [customHook("touch {marker}", "on_remove")],
}});
"""
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not marker.exists(), "on_remove must not fire at install"

    remove_module(repo, "demo")
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
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "demo": f"""
import {{ module, trackedCopy, customHook }} from "@gripsack/core";

export default module("demo", {{
  config: {{ "configs/demo/a.conf": trackedCopy("~/.config/demo/a.conf") }},
  activate: [customHook("true")],
}});
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
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "demo": f"""
import {{ module, trackedCopy, customHook }} from "@gripsack/core";

export default module("demo", {{
  config: {{ "configs/demo/a.conf": trackedCopy("~/.config/demo/a.conf") }},
  activate: [customHook("echo activated >> {marker}")],
}});
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
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "demo": """
import { module } from "@gripsack/core";

export default module("demo", {});
""",
            "ephemeral": f"""
import {{ module, customHook }} from "@gripsack/core";

export default module("ephemeral", {{
  activate: [customHook("echo removed >> {marker}", "on_remove")],
}});
""",
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)  # gen 1: both modules
    assert out.returncode == 0, out.stderr
    remove_module(repo, "ephemeral")
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


def test_fonts_and_desktop_entry_adapters_run_once_per_apply(sandbox, monkeypatch):
    """PostLink intents: fonts() runs fc-cache, desktop_entry() runs
    update-desktop-database — deduped across modules, tolerating
    absence (0001 §3.8)."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    log = sandbox / "calls.log"
    for tool in ("fc-cache", "update-desktop-database"):
        (bindir / tool).write_text(f'#!/bin/sh\necho "{tool} $@" >> {log}\n')
        (bindir / tool).chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    confdir = sandbox / "myenv" / "configs" / "font"
    confdir.mkdir(parents=True)
    (confdir / "myfont.ttf").write_text("fake font\n")
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "font-a": """
import { desktopEntry, fonts, module, symlink } from "@gripsack/core";

export default module("font-a", {
  config: { "configs/font/myfont.ttf": symlink("~/.local/share/fonts/myfont.ttf") },
  activate: [fonts(), desktopEntry()],
});
""",
            "font-b": """
import { fonts, module, symlink } from "@gripsack/core";

export default module("font-b", {
  config: { "configs/font/myfont.ttf": symlink("~/.local/share/fonts/myfont-b.ttf") },
  activate: [fonts()],
});
""",
        },
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    calls = log.read_text().splitlines()
    assert calls.count("fc-cache -f") == 1, calls
    assert sum(1 for c in calls if "update-desktop-database" in c and "applications" in c) == 1, calls


def test_fonts_adapter_skips_cleanly_without_fc_cache(sandbox):
    """No fc-cache on PATH → a warning, never an apply error."""
    confdir = sandbox / "myenv" / "configs" / "font"
    confdir.mkdir(parents=True)
    (confdir / "myfont.ttf").write_text("fake font\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { fonts, module, symlink } from "@gripsack/core";

export default module("font", {
  config: { "configs/font/myfont.ttf": symlink("~/.local/share/fonts/myfont.ttf") },
  activate: [fonts()],
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_step_form_intents_run_through_adapters(sandbox):
    """Step-form intents (class-style) execute via the activation
    adapters — a custom hook's post-activate script really runs."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { customHook, module, symlink } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.txt": symlink("~/.config/demo/a.txt") },
  activate: [customHook("echo post-activate > ~/hook-ran")],
});
""",
    )
    confdir = repo / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / "hook-ran").read_text() == "post-activate\n"
