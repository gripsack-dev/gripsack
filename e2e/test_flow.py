"""Flow tests (plan/0003 §5) against the real binary and the real
TypeScript frontend (plan/0013): fixture env repos are built by conftest
under the defineEnv contract — modules/<name>.ts default-exports its
module value, hosts/<host>.ts returns them from defineEnv."""
import os
import shutil
import stat
import sys
import subprocess

import pytest
from conftest import (
    GRIP,
    grip,
    make_env_repo,
    make_tarball,
    refresh_host,
    remove_module,
)

HELLO_MODULE = """
import { module, trackedCopy } from "@gripsack/core";

export default module("hello", {
  config: { "configs/demo/a": trackedCopy("~/.config/demo/a") },
});
"""


def test_binary_exists_and_runs():
    out = grip("--version")
    assert out.returncode == 0
    assert "grip" in out.stdout


def test_doctor_reports_environment(sandbox):
    out = grip("doctor")
    # exit code depends on the sandbox's runtime being acceptable to
    # doctor; the report itself is the contract.
    assert "deno:" in out.stdout
    assert "frontend:" in out.stdout
    assert "home:" in out.stdout
    assert str(sandbox) in out.stdout


def test_untrusted_repo_fails_closed_without_tty(sandbox, monkeypatch):
    """The trust gate (0013 D7): first eval of an untrusted repo, no TTY
    to prompt on → hard error with the escape hatch, never a silent
    eval of unreviewed repo code."""
    monkeypatch.delenv("GRIPSACK_TRUST_ALL", raising=False)
    repo = make_env_repo(sandbox / "myenv", HELLO_MODULE)
    (repo / "configs" / "demo").mkdir(parents=True)
    (repo / "configs" / "demo" / "a").write_text("a\n")
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "trust" in out.stderr.lower()
    assert "grip trust add" in out.stderr
    # nothing ran: eval never happened
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_trust_add_records_and_unblocks_eval(sandbox, monkeypatch):
    """`grip trust add <path>` records the canonical path; eval then
    proceeds without the CI bypass; remove re-arms the gate."""
    monkeypatch.delenv("GRIPSACK_TRUST_ALL", raising=False)
    repo = make_env_repo(sandbox / "myenv", HELLO_MODULE)
    (repo / "configs" / "demo").mkdir(parents=True)
    (repo / "configs" / "demo" / "a").write_text("a\n")

    out = grip("trust", "add", str(repo))
    assert out.returncode == 0, out.stderr
    trust = sandbox / ".local/share/gripsack" / "trust.toml"
    assert "[[repos]]" in trust.read_text()
    assert str(repo) in trust.read_text()

    listing = grip("trust", "list")
    assert listing.returncode == 0, listing.stderr
    assert str(repo) in listing.stdout

    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    out = grip("trust", "remove", str(repo))
    assert out.returncode == 0, out.stderr
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "trust" in out.stderr.lower()


def test_apply_creates_generation_and_symlinks(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    home = sandbox / ".local/share/gripsack"
    assert (home / "generations/1").is_dir()
    assert (home / "current").is_symlink()
    assert (sandbox / ".local/bin/hello").is_symlink()


def test_tree_entries_and_prune_on_undeclare(sandbox):
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "settings.json").write_text('{"theme": "mocha"}\n')
    (confdir / "keymap.json").write_text("[]\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("zed", {
  config: { ...tree("configs/zed", "~/.config/zed") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".config" / "zed"
    assert (deployed / "settings.json").exists()
    assert (deployed / "keymap.json").exists()

    # drop a file from the tree -> pruned on next apply
    (confdir / "keymap.json").unlink()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (deployed / "settings.json").exists()
    assert not (deployed / "keymap.json").exists()


def test_check_passes_valid_repo_and_has_no_side_effects(sandbox):
    """grip check (0011 §9): eval + sema + lint, zero side effects."""
    repo = make_lint_repo(sandbox, "good = true\n")
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "check: ok" in out.stdout
    # zero side effects: no generations, no store, nothing deployed
    assert not (sandbox / ".local/share/gripsack/generations").exists()
    assert not (sandbox / ".local/share/gripsack/store").exists()
    assert not (sandbox / ".config/demo").exists()


def test_check_fails_on_lint_error(sandbox):
    repo = make_lint_repo(sandbox, "BAD_KEY = 1\n")
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "griplint-demo/A01" in out.stderr
    assert not (sandbox / ".local/share/gripsack/generations").exists()
    assert not (sandbox / ".config/demo").exists()


def test_owned_prune_on_undeclare(sandbox):
    """Regression: prune-on-undeclare must work for owned symlinks too —
    the recorded hash is the source content, so the tracked_copy hash
    check can never match a symlink (gripsack-exec apply.rs)."""
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    (confdir / "b.txt").write_text("b\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("zed", {
  config: { ...tree("configs/zed", "~/.config/zed", "owned") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".config" / "zed"
    assert (deployed / "a.txt").is_symlink()
    assert (deployed / "b.txt").is_symlink()

    (confdir / "b.txt").unlink()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (deployed / "a.txt").is_symlink()
    assert not (deployed / "b.txt").exists()

    # a user file replacing our symlink is never pruned
    (deployed / "a.txt").unlink()
    (deployed / "a.txt").write_text("user edit\n")
    (confdir / "a.txt").unlink()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (deployed / "a.txt").read_text() == "user edit\n"


LINT_FIXTURE = """#!/usr/bin/env python3
import json, sys
req = json.loads(sys.stdin.readline())
for p in req["paths"]:
    for n, line in enumerate(open(p).read().splitlines(), 1):
        if "BAD_KEY" in line:
            print(json.dumps({"type": "diagnostic", "diagnostic": {
                "code": "griplint-demo/A01", "severity": "error",
                "message": "unknown key BAD_KEY",
                "labels": [{"span": {"file": p, "line": n}, "note": "not a real key"}],
                "help": "remove it"}}))
        if "WARN_KEY" in line:
            print(json.dumps({"type": "diagnostic", "diagnostic": {
                "code": "griplint-demo/W01", "severity": "warning",
                "message": "WARN_KEY is deprecated",
                "labels": []}}))
print(json.dumps({"type": "response", "id": 1, "result": {"linted": len(req["paths"])}}))
"""


def make_lint_repo(sandbox, config_text, lint_decl='lint: "demo"'):
    """A repo with one linted config module and the fixture linter on
    a path registration (offline — 0010 §3's path form)."""
    import stat

    repo = sandbox / "myenv"
    confdir = repo / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "demo.toml").write_text(config_text)
    exe = sandbox / "griplint-demo"
    exe.write_text(LINT_FIXTURE)
    exe.chmod(exe.stat().st_mode | stat.S_IXUSR)
    make_env_repo(
        repo,
        f"""
import {{ module, trackedCopy }} from "@gripsack/core";

export default module("demo", {{
  config: {{ "configs/demo/demo.toml": trackedCopy("~/.config/demo/demo.toml") }},
  {lint_decl},
}});
""",
    )
    (repo / "env.toml").write_text(
        f'[env]\nname = "fixture"\n\n[linters.demo]\npath = "{exe}"\n'
    )
    return repo


def test_lint_error_fails_apply_before_staging(sandbox):
    repo = make_lint_repo(sandbox, "BAD_KEY = 1\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "griplint-demo/A01" in out.stderr
    assert "unknown key BAD_KEY" in out.stderr
    # nothing staged or deployed
    assert not (sandbox / ".config/demo/demo.toml").exists()
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_lint_warning_flows_but_applies(sandbox):
    repo = make_lint_repo(sandbox, "WARN_KEY = 1\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "griplint-demo/W01" in out.stderr
    assert (sandbox / ".config/demo/demo.toml").exists()


def test_lint_clean_config_applies(sandbox):
    repo = make_lint_repo(sandbox, "good = true\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "griplint-demo" not in out.stderr


def test_lint_unregistered_name_is_a_hard_eval_error(sandbox):
    repo = make_lint_repo(sandbox, "good = true\n", lint_decl='lint: "ghost"')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E501" in out.stderr
    assert "ghost" in out.stderr


def test_owned_deploy_refuses_foreign_paths_unless_take_over(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    foreign = sandbox / ".local" / "bin"
    foreign.mkdir(parents=True)
    (foreign / "hello").write_text("system binary\n")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "not deployed by gripsack" in out.stderr

    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (foreign / "hello").is_symlink()


def test_apply_repo_from_elsewhere(sandbox):
    """The bootstrap story: apply a repo that isn't the cwd."""
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    elsewhere = sandbox / "elsewhere"
    elsewhere.mkdir()
    out = grip("apply", "--host", "testhost", "--repo", str(repo), cwd=elsewhere)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/hello").is_symlink()

    # git URL form (a local path is a valid clone source)
    git_env = {
        "GIT_AUTHOR_NAME": "t",
        "GIT_AUTHOR_EMAIL": "t@t",
        "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@t",
        "PATH": os.environ["PATH"],
    }
    subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True, env=git_env)
    subprocess.run(["git", "add", "-A"], cwd=repo, check=True, env=git_env)
    subprocess.run(["git", "commit", "--quiet", "-m", "init"], cwd=repo, check=True, env=git_env)
    bare = sandbox / "myenv-remote"
    subprocess.run(["git", "clone", "--quiet", str(repo), str(bare)], check=True, env=git_env)
    out = grip("apply", "--host", "testhost", "--repo", str(bare), cwd=elsewhere)
    assert out.returncode == 0, out.stderr


def test_explicit_steps_module_is_satisfied_on_reapply(sandbox):
    """Class/explicit-steps modules keep fetch specs in steps, not
    module.fetch — their store path must still be stable (canary-caught)."""
    payload = sandbox / "hello.tar.gz"
    make_tarball(payload, {"bin/x": b"#!/bin/sh\necho x\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fetchStep, module, shellStep, tarball }} from "@gripsack/core";

export default module("stepped", {{
  steps: [
    fetchStep(tarball("file://{payload}")),
    shellStep("true", "noop", {{ needs: ["fetch"] }}),
  ],
}});
""",
    )
    first = grip("apply", "--host", "testhost", cwd=repo)
    assert first.returncode == 0, first.stderr
    second = grip("apply", "--host", "testhost", cwd=repo)
    assert second.returncode == 0, second.stderr
    assert "already satisfied" in second.stdout


def test_update_rewrites_lockfile_then_apply_deploys(sandbox):
    """The flake cycle: update moves the lockfile, apply executes it."""
    payload = sandbox / "hello.tar.gz"
    make_tarball(payload, {"bin/hello": b"#!/bin/sh\necho v1\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    lock = repo / "locks" / "testhost.lock"
    assert lock.exists()
    first_pin = lock.read_text()

    # no movement -> unchanged
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "unchanged" in out.stdout
    assert lock.read_text() == first_pin

    # payload changes -> update bumps the pin, apply deploys it
    make_tarball(payload, {"bin/hello": b"#!/bin/sh\necho v2\n"})
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "bumped" in out.stdout
    assert lock.read_text() != first_pin

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "applied" in out.stdout


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
    profile = sandbox / ".local/share/gripsack/env/profile.sh"
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


def test_gc_collects_unreferenced_store_paths_and_why_owns(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho v1\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("why-owns", "~/.local/bin/hello", cwd=repo)
    assert out.returncode == 0
    assert "hello" in out.stdout
    store = sandbox / ".local/share/gripsack/store"
    before = {p.name for p in store.iterdir()}
    assert len(before) == 1

    # drop the module entirely; deployment pruned on the next apply
    remove_module(repo, "hello")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not (sandbox / ".local/bin/hello").exists()

    # gen 1 still references the store path — gc alone must keep it
    # (rollback!). keep_generations = 1 is what releases it.
    out = grip("gc", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert set({p.name for p in store.iterdir()}) == before

    user_conf = sandbox / ".config/gripsack"
    user_conf.mkdir(parents=True)
    (user_conf / "config.toml").write_text("[settings]\nkeep_generations = 1\n")
    out = grip("gc", cwd=repo)
    assert out.returncode == 0, out.stderr
    after = set(store.iterdir()) if store.exists() else set()
    assert not after, f"gc left {after}"
    assert not (sandbox / ".local/share/gripsack/generations/1").exists()
    assert (sandbox / ".local/share/gripsack/generations/2").exists()
    out = grip("why-owns", "~/.local/bin/hello", cwd=repo)
    assert out.returncode != 0


def test_duplicate_destination_is_a_check_time_error(sandbox):
    """E111 (N2): two modules may not declare the same destination —
    a deploy race in parallel and a lie for why-owns."""
    confdir = sandbox / "myenv" / "configs" / "x"
    confdir.mkdir(parents=True)
    (confdir / "same.conf").write_text("x\n")
    repo = make_env_repo(
        sandbox / "myenv",
        {
            "one": """
import { module, trackedCopy } from "@gripsack/core";

export default module("one", {
  config: { "configs/x/same.conf": trackedCopy("~/.out/same.conf") },
});
""",
            "two": """
import { module, trackedCopy } from "@gripsack/core";

export default module("two", {
  config: { "configs/x/same.conf": trackedCopy("~/.out/same.conf") },
});
""",
        },
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E111" in out.stderr
    assert "same.conf" in out.stderr


def test_jobs_one_forces_serial_execution(sandbox):
    """--jobs bounds the scheduler (N3): the 2x2s parallel proof
    inverted — with --jobs 1 it must take serial time."""
    import time

    repo = make_env_repo(
        sandbox / "myenv",
        {
            name: f"""
import {{ module }} from "@gripsack/core";

export default module("{name}", {{
  build: {{ kind: "custom_shell", script: "sleep 2" }},
}});
"""
            for name in ("slow-a", "slow-b")
        },
    )
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", "--jobs", "1", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed >= 3.5, f"--jobs 1 not respected: {elapsed:.1f}s"


def test_gc_dry_run_previews_without_deleting(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho v1\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    remove_module(repo, "hello")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    user_conf = sandbox / ".config/gripsack"
    user_conf.mkdir(parents=True)
    (user_conf / "config.toml").write_text("[settings]\nkeep_generations = 1\n")
    store = sandbox / ".local/share/gripsack/store"
    before = set(store.iterdir())
    out = grip("gc", "--dry-run", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "dry run" in out.stdout
    assert "collected" in out.stdout or "pruned" in out.stdout
    assert set(store.iterdir()) == before  # nothing deleted
    assert (sandbox / ".local/share/gripsack/generations/1").exists()


def test_eval_env_reaches_build_steps(sandbox):
    """[eval] env (build-time exported env): env.toml declares it,
    a build step's shell inherits it."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, shellStep } from "@gripsack/core";

export default module("probe", {
  steps: [shellStep('test "$MY_CERT_PATH" = "/etc/ssl/company.pem"', "probe")],
});
""",
    )
    (repo / "env.toml").write_text(
        '[env]\nname = "fixture"\n\n[eval]\nenv = { MY_CERT_PATH = "/etc/ssl/company.pem" }\n'
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_independent_modules_run_in_parallel(sandbox):
    """Two independent 2s builds finish in ~2s, not 4s (0007 §5 —
    the ready-queue scheduler runs N = cores)."""
    import time

    repo = make_env_repo(
        sandbox / "myenv",
        {
            name: f"""
import {{ module }} from "@gripsack/core";

export default module("{name}", {{
  build: {{ kind: "custom_shell", script: "sleep 2" }},
}});
"""
            for name in ("slow-a", "slow-b")
        },
    )
    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed < 3.5, f"serial execution suspected: {elapsed:.1f}s"


def test_check_fails_statically_on_missing_config_source(sandbox):
    """E110 (review finding E2): a fetch-less module's source must be
    a repo file — missing is a check-time error, not mid-deploy."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("aaa", {
  config: { "configs/aaa/MISSING.conf": trackedCopy("~/.out/a.conf") },
});
""",
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E110" in out.stderr
    assert "MISSING.conf" in out.stderr
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_owned_deploy_refuses_foreign_symlinks(sandbox):
    """Review finding E4: a stow-style foreign symlink is exactly the
    path the guard is for — refuse unless --take-over."""
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    foreign = sandbox / ".local/bin"
    foreign.mkdir(parents=True)
    stow_target = sandbox / "elsewhere/real-hello"
    stow_target.parent.mkdir(parents=True)
    stow_target.write_text("#!/bin/sh\necho stow\n")
    (foreign / "hello").symlink_to(stow_target)

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "not deployed by gripsack" in out.stderr
    assert (foreign / "hello").readlink() == stow_target  # untouched

    out = grip("apply", "--host", "testhost", "--take-over", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert str(foreign / "hello").endswith("hello")


def test_failed_apply_rolls_back_this_runs_deployments(sandbox):
    """0001 §9 / review finding E1: a mid-graph failure must leave no
    half-applied deployment — the flip never happens, and every
    destination the failed run touched returns to the previous
    generation's state."""
    confdir = sandbox / "myenv" / "configs" / "aaa"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("v1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("aaa", {
  config: { "configs/aaa/a.conf": trackedCopy("~/.out/a.conf") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".out/a.conf").read_text() == "v1\n"

    # v2 of aaa + a module that fails at deploy (payload lacks the
    # entry — a deploy-time failure E110 can't catch)
    (confdir / "a.conf").write_text("v2\n")
    payload = make_tarball(sandbox / "b.tar.gz", {"bin/b": b"#!/bin/sh\n"})
    (repo / "modules" / "bbb.ts").write_text(
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("bbb", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/MISSING": symlink("~/.out/b") }},
}});
"""
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    # the flip never happened…
    generations = sandbox / ".local/share/gripsack/generations"
    assert [p.name for p in generations.iterdir()] == ["1"]
    # …and this run's deployments are rolled back exactly
    assert (sandbox / ".out/a.conf").read_text() == "v1\n"
    assert not (sandbox / ".out/b").exists()


def test_concurrent_applies_serialize_and_lose_nothing(sandbox):
    """Finding A: two applies over disjoint subsets must not lose a
    manifest update — the lifecycle holds apply.flock."""
    repo = sandbox / "myenv"
    for name in ("amod", "bmod"):
        confdir = repo / "configs" / name
        confdir.mkdir(parents=True)
        (confdir / f"{name}.conf").write_text(f"{name}\n")
    make_env_repo(
        repo,
        {
            name: f"""
import {{ module, trackedCopy }} from "@gripsack/core";

export default module("{name}", {{
  config: {{ "configs/{name}/{name}.conf": trackedCopy("~/.out/{name}.conf") }},
}});
"""
            for name in ("amod", "bmod")
        },
    )
    grip_bin = str(GRIP.resolve())
    p1 = subprocess.Popen(
        [grip_bin, "apply", "--host", "testhost", "amod"],
        cwd=repo,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    p2 = subprocess.Popen(
        [grip_bin, "apply", "--host", "testhost", "bmod"],
        cwd=repo,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    o1, e1 = p1.communicate(timeout=60)
    o2, e2 = p2.communicate(timeout=60)
    assert p1.returncode == 0, e1
    assert p2.returncode == 0, e2
    out = grip("why-owns", "~/.out/amod.conf", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "amod" in out.stdout
    out = grip("why-owns", "~/.out/bmod.conf", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "bmod" in out.stdout


def test_jobs_zero_is_rejected(sandbox):
    """Finding B: --jobs 0 / GRIPSACK_JOBS=0 must fail loudly, never
    silently unmanage the environment."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("a", {
  config: { "configs/a/a": trackedCopy("~/.out/a") },
});
""",
    )
    (repo / "configs" / "a").mkdir(parents=True)
    (repo / "configs" / "a" / "a").write_text("a\n")
    out = grip("apply", "--host", "testhost", "--jobs", "0", cwd=repo)
    assert out.returncode != 0
    assert "--jobs 0" in out.stderr
    assert not (sandbox / ".local/share/gripsack/generations").exists()


def test_deferred_identity_is_stable_across_applies(sandbox):
    """Finding C: fetch kinds whose payload hash isn't knowable up
    front (git/pixi/plugin) must produce ONE store path and satisfy
    on the second apply, not a spurious second generation."""
    import subprocess as sp

    remote = sandbox / "remote"
    remote.mkdir()
    env = dict(
        GIT_AUTHOR_NAME="t",
        GIT_AUTHOR_EMAIL="t@t",
        GIT_COMMITTER_NAME="t",
        GIT_COMMITTER_EMAIL="t@t",
        PATH="/usr/bin:/bin:/usr/local/bin",
        HOME=str(sandbox),
    )
    sp.run(["git", "init", "--quiet"], cwd=remote, env=env, check=True)
    (remote / "bin.txt").write_text("payload\n")
    sp.run(["git", "add", "."], cwd=remote, env=env, check=True)
    sp.run(["git", "commit", "--quiet", "-m", "init"], cwd=remote, env=env, check=True)
    rev = sp.run(
        ["git", "rev-parse", "HEAD"], cwd=remote, env=env, check=True, capture_output=True, text=True
    ).stdout.strip()

    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ git, module, symlink }} from "@gripsack/core";

export default module("tool", {{
  fetch: git("file://{remote}", "{rev}"),
  install: {{ "bin.txt": symlink("~/.local/bin/tool") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "applied" in out.stdout
    store = sandbox / ".local/share/gripsack/store"
    paths_after_first = sorted(p.name for p in store.iterdir())
    assert len(paths_after_first) == 1

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "already satisfied" in out.stdout, out.stdout
    assert sorted(p.name for p in store.iterdir()) == paths_after_first


def test_rollback_restores_previous_generation(sandbox):
    payload = make_tarball(
        sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"}
    )
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    first = grip("apply", "--host", "testhost", cwd=repo)
    assert first.returncode == 0, first.stderr

    # a no-op apply creates no generation (0008 §3)
    second = grip("apply", "--host", "testhost", cwd=repo)
    assert second.returncode == 0, second.stderr
    assert "already satisfied" in second.stdout
    assert not (sandbox / ".local/share/gripsack/generations/2").exists()

    # a changed module produces generation 2; rollback restores 1
    (repo / "modules" / "extra.ts").write_text(
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("extra", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/extra") }},
}});
"""
    )
    refresh_host(repo)
    third = grip("apply", "--host", "testhost", cwd=repo)
    assert third.returncode == 0, third.stderr
    assert (sandbox / ".local/bin/extra").is_symlink()

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    current = sandbox / ".local/share/gripsack/current"
    assert current.resolve().name == "1"
    # the extra module's destination is gone after rollback
    assert not (sandbox / ".local/bin/extra").exists()


def test_merge_mode_owns_one_block_in_a_foreign_file(sandbox):
    """merge: gripsack owns exactly one delimited block; everything
    outside the markers is never touched (0001 §3.7)."""
    confdir = sandbox / "myenv" / "configs" / "shell"
    confdir.mkdir(parents=True)
    (confdir / "block.sh").write_text('export PATH="$HOME/.local/bin:$PATH"\n')
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { merge, module } from "@gripsack/core";

export default module("shell", {
  config: { "configs/shell/block.sh": merge("~/.bashrc") },
});
""",
    )
    # foreign file with pre-existing content
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("# user stuff\nexport EDITOR=hx\n")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert content.startswith("# user stuff\nexport EDITOR=hx\n")
    assert "# >>> gripsack module=shell >>>" in content
    assert 'export PATH="$HOME/.local/bin:$PATH"' in content
    assert "# <<< gripsack <<<" in content

    # re-apply is satisfied; user drift INSIDE the block self-heals,
    # user content outside is untouched
    healed = content.replace("export PATH", "# user edited\nexport PATH")
    bashrc.write_text(healed + "\n# more user stuff\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert content.count("# >>> gripsack module=shell >>>") == 1
    assert "# user edited" not in content
    assert content.endswith("# more user stuff\n")

    # undeclare prunes only the block; the foreign file stays
    remove_module(repo, "hello")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    content = bashrc.read_text()
    assert "gripsack" not in content
    assert "# user stuff" in content
    assert "# more user stuff" in content


def test_template_mode_renders_vars_at_deploy(sandbox):
    """template: {{ name }} placeholders render from entry vars at
    deploy time; undefined variables fail loudly (0001 §3.7)."""
    confdir = sandbox / "myenv" / "configs" / "git"
    confdir.mkdir(parents=True)
    (confdir / "id.toml").write_text(
        'email = "{{ email }}"\nname = "{{ name }}"\nliteral = "{{{{ keep }}"\n'
    )
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, template } from "@gripsack/core";

export default module("git", {
  config: {
    "configs/git/id.toml": template("~/.config/git/id.toml", {
      email: "a@b.c",
      name: "T",
    }),
  },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".config" / "git" / "id.toml"
    assert deployed.read_text() == 'email = "a@b.c"\nname = "T"\nliteral = "{{ keep }}"\n'

    # changing a var updates the rendered dest on the next apply
    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, template } from "@gripsack/core";

export default module("git", {
  config: {
    "configs/git/id.toml": template("~/.config/git/id.toml", {
      email: "x@y.z",
      name: "T",
    }),
  },
});
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert 'email = "x@y.z"' in deployed.read_text()

    # an undefined variable fails at apply, never silently empty
    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, template } from "@gripsack/core";

export default module("git", {
  config: {
    "configs/git/id.toml": template("~/.config/git/id.toml", {
      email: "x@y.z",
    }),
  },
});
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "undefined variable" in out.stderr


def test_init_scaffolds_a_working_env_repo(sandbox):
    """grip init: embedded template (offline, version-matched), never
    clobbers an existing env repo."""
    repo = sandbox / "fresh"
    repo.mkdir()
    out = grip("init", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / "env.toml").exists()
    assert (repo / "modules" / "hello.ts").exists()
    assert (repo / "modules" / "examples.ts").exists()
    assert (repo / "configs" / "hello" / "hello.toml").exists()
    assert (repo / "package.json").exists()
    assert '"@gripsack/core"' in (repo / "package.json").read_text()
    assert (repo / "tsconfig.json").exists()
    assert "node_modules/" in (repo / ".gitignore").read_text()
    hosts = list((repo / "hosts").glob("*.ts"))
    assert len(hosts) == 1
    assert (repo / ".git").is_dir()

    # the scaffold is a working repo: check and apply succeed offline
    out = grip("check", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config" / "hello" / "hello.toml").is_symlink()

    # init never clobbers an existing env repo
    out = grip("init", cwd=repo)
    assert out.returncode != 0
    assert "already looks like an env repo" in out.stderr


FETCH_FIXTURE = """#!/usr/bin/env python3
import json, os, sys
req = json.loads(sys.stdin.readline())
if req["op"] == "capabilities":
    print(json.dumps({"type": "response", "result": {
        "capabilities": {"throttle": {"demo.local": "1/s"}}}}))
elif req["op"] == "fetch":
    dest = req["dest_dir"]
    os.makedirs(os.path.join(dest, "bin"), exist_ok=True)
    with open(os.path.join(dest, "bin", "demo"), "w") as f:
        f.write("#!/bin/sh\\necho demo\\n")
    print(json.dumps({"type": "response", "result": {}}))
sys.stdout.flush()
"""

PLUGIN_MODULES = {
    name: """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("%s", {
  fetch: pluginFetch("demo"),
  install: { "bin/demo": symlink("~/.local/bin/demo-%s") },
});
"""
    % (name, suffix)
    for name, suffix in (("a", "a"), ("b", "b"))
}


def test_throttle_token_bucket_serializes_plugin_fetches(sandbox, monkeypatch):
    """[throttle] (0002): a fetcher declares its rate budget via the
    capabilities op; the core's token bucket enforces it across
    concurrent modules — two fetches at 1/s take >= ~1s wall."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    repo = make_env_repo(sandbox / "myenv", PLUGIN_MODULES)
    import time

    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed >= 0.9, f"two 1/s fetches finished in {elapsed:.2f}s — bucket not enforced"
    assert (sandbox / ".local/bin/demo-a").is_symlink()
    assert (sandbox / ".local/bin/demo-b").is_symlink()


def test_throttle_user_override_beats_plugin_budget(sandbox, monkeypatch):
    """env.toml [throttle] outranks the fetcher's own declaration."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    repo = make_env_repo(sandbox / "myenv", PLUGIN_MODULES)
    with open(repo / "env.toml", "a") as f:
        f.write('\n[throttle]\n"demo.local" = "100/s"\n')
    import time

    start = time.monotonic()
    out = grip("apply", "--host", "testhost", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed < 0.9, f"override not applied — took {elapsed:.2f}s"


CRASHY_LINTER = """#!/usr/bin/env python3
import json, sys
req = json.loads(sys.stdin.readline())
print(json.dumps({"type": "diagnostic", "diagnostic": {
    "code": "griplint-demo/E99", "severity": "error",
    "message": "linter crashed: boom", "labels": []}}))
print(json.dumps({"type": "response", "id": 1, "result": {"linted": 0}}))
"""


def test_crash_class_lint_codes_are_warnings_core_side(sandbox):
    """0012: a linter's self-reported severity for crash-class codes
    (E99/E02) is not evidence — the CORE host classifies by code,
    always warning, so lint= never becomes an availability dependency
    (review finding E, enforcement round)."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    linter = bindir / "griplint-demo"
    linter.write_text(CRASHY_LINTER)
    linter.chmod(0o755)
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "demo.toml").write_text("good = true\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/demo.toml": trackedCopy("~/.config/demo/demo.toml") },
  lint: "demo",
});
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write(f'\n[linters.demo]\npath = "{linter}"\n')
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "griplint-demo/E99" in out.stderr
    assert "warning" in out.stderr.lower()


CHATTY_LINTER = """#!/usr/bin/env python3
import json, sys
req = json.loads(sys.stdin.readline())
sys.stderr.write("noise\\n" * 20000)  # >64KB — fills an undrained pipe
sys.stderr.flush()
print(json.dumps({"type": "response", "id": 1, "result": {"linted": len(req["paths"])}}))
"""


def test_chatty_linter_does_not_deadlock_the_exchange(sandbox):
    """0012 host hardening: a linter writing >64KB to stderr fills the
    pipe; without a concurrent drain the child blocks before its
    response and the exchange burns the 120s deadline into a false
    'linter is broken' warning (the fetch host's F1 rule, inherited)."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    linter = bindir / "griplint-demo"
    linter.write_text(CHATTY_LINTER)
    linter.chmod(0o755)
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "demo.toml").write_text("good = true\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/demo.toml": trackedCopy("~/.config/demo/demo.toml") },
  lint: "demo",
});
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write(f'\n[linters.demo]\npath = "{linter}"\n')
    import time

    start = time.monotonic()
    out = grip("check", "--host", "testhost", cwd=repo)
    elapsed = time.monotonic() - start
    assert out.returncode == 0, out.stderr
    assert elapsed < 30, f"chatty linter took {elapsed:.0f}s — stderr not drained concurrently"


def test_unmatched_host_errors_instead_of_empty_tags(sandbox):
    """A hosts/ dir with no matching entrypoint must not silently
    yield empty tags — every gated module would drop and the run would
    report success (enterprise review finding)."""
    repo = make_env_repo(sandbox / "myenv", HELLO_MODULE)
    (sandbox / "myenv" / "configs" / "demo").mkdir(parents=True)
    (sandbox / "myenv" / "configs" / "demo" / "a").write_text("a\n")
    out = grip("check", "--host", "nosuchhost", cwd=repo)
    assert out.returncode != 0
    assert "nosuchhost" in out.stderr


def test_default_host_resolves_role_named_entrypoint(sandbox):
    """[env] default_host: containers with random hostnames still get a
    deterministic host entrypoint (enterprise review finding)."""
    repo = make_env_repo(sandbox / "myenv", HELLO_MODULE)
    (sandbox / "myenv" / "configs" / "demo").mkdir(parents=True)
    (sandbox / "myenv" / "configs" / "demo" / "a").write_text("a\n")
    (sandbox / "myenv" / "hosts" / "testhost.ts").unlink()
    (sandbox / "myenv" / "hosts" / "role.ts").write_text(
        'import { defineEnv } from "@gripsack/core";\n'
        "import hello from \"../modules/hello.ts\";\n"
        "\n"
        "export default defineEnv((ctx) => ({\n"
        '  tags: ["container"],\n'
        "  modules: [hello],\n"
        "}));\n"
    )
    env_toml = repo / "env.toml"
    env_toml.write_text(env_toml.read_text().replace(
        '[env]\nname = "fixture"', '[env]\nname = "fixture"\ndefault_host = "role"'))
    out = grip("check", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "container" in out.stdout


def _seed_plugin_store(sandbox, exe, fixture, tag="1.0"):
    """Pre-seed the managed plugin store as if a prior provision ran."""
    home = sandbox / ".local/share/gripsack"
    bindir = home / "plugins" / exe / tag
    bindir.mkdir(parents=True)
    (bindir / exe).write_text(fixture)
    (bindir / exe).chmod(0o755)
    (home / "plugins" / exe / "current").symlink_to(f"{tag}/")
    (home / "plugins" / "receipts").mkdir(parents=True)
    (home / "plugins" / "receipts" / f"{exe}.toml").write_text(
        f'source = "acme/{exe}"\ntag = "{tag}"\nsha256 = "ab"\n'
    )


def test_fetcher_package_ref_resolves_from_the_plugin_store(sandbox):
    """0012 move 2: [fetchers.x] package = "owner/repo@tag" — with the
    receipt already recording the tag, provisioning is a satisfied
    no-op (no network) and the fetch runs the store's binary, not PATH."""
    _seed_plugin_store(sandbox, "gripfetch-demo", FETCH_FIXTURE)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("a", {
  fetch: pluginFetch("demo"),
  install: { "bin/demo": symlink("~/.local/bin/demo-a") },
});
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()


def test_linter_repo_ref_resolves_from_the_plugin_store(sandbox):
    """0012 move 2: a repo-ref [linters.x] package resolves the
    provisioned store binary (the wheel meaning stays for bare names)."""
    _seed_plugin_store(sandbox, "griplint-demo", LINT_FIXTURE)
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "demo.toml").write_text("BAD_KEY = 1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/demo.toml": trackedCopy("~/.config/demo/demo.toml") },
  lint: "demo",
});
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[linters.demo]\npackage = "acme/griplint-demo@1.0"\n')
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "griplint-demo/A01" in out.stderr


def test_builtin_pack_lints_in_process_without_registration(sandbox):
    """0012 move 3: lint="helix" with NO [linters] entry runs the
    embedded pack in-process — no venv, no provisioning, no plugin."""
    confdir = sandbox / "myenv" / "configs" / "helix"
    confdir.mkdir(parents=True)
    (confdir / "config.toml").write_text('[editor]\nscrollof = 5\n')
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("helix", {
  config: { "configs/helix/config.toml": trackedCopy("~/.config/helix/config.toml") },
  lint: "helix",
});
""",
    )
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "A01" in out.stderr
    assert "scrolloff" in out.stderr
    # and the satisfied path: the fix goes green
    (confdir / "config.toml").write_text('[editor]\nscrolloff = 5\n')
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_fetcher_path_registers_an_offline_executable(sandbox):
    """[fetchers.x] path = ... registers the executable directly —
    the offline/air-gapped route, symmetric with [linters.x] path
    (enterprise review). No PATH lookup, no provisioning."""
    bindir = sandbox / "bin"
    bindir.mkdir()
    fetcher = bindir / "gripfetch-demo"
    fetcher.write_text(FETCH_FIXTURE)
    fetcher.chmod(0o755)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("a", {
  fetch: pluginFetch("demo"),
  install: { "bin/demo": symlink("~/.local/bin/demo-a") },
});
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write(f'\n[fetchers.demo]\npath = "{fetcher}"\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo-a").is_symlink()


def test_changed_fetch_spec_re_resolves_instead_of_mirror_blame(sandbox):
    """A fetch-spec edit must not fail as 'the mirror changed' — the
    args are the declaration and the pin follows them (enterprise
    review's stale-plugin-lock papercut)."""
    _seed_plugin_store(sandbox, "gripfetch-demo", FETCH_FIXTURE)
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("a", {
  fetch: pluginFetch("demo"),
  install: { "bin/demo": symlink("~/.local/bin/demo-a") },
});
""",
    )
    with open(repo / "env.toml", "a") as f:
        f.write('\n[fetchers.demo]\npackage = "acme/gripfetch-demo@1.0"\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    # same module, now pinned — the lock entry for the OLD args must
    # not compare against the NEW spec
    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, pluginFetch, symlink } from "@gripsack/core";

export default module("a", {
  fetch: pluginFetch("demo", { package: "hello", version: "1.0" }),
  install: { "bin/demo": symlink("~/.local/bin/demo-a") },
});
"""
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "mirror" not in out.stderr


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


def test_tracked_copy_drift_is_kept_never_clobbered(sandbox):
    """The killer drift policy (0001 §3.7): a user edit inside a
    tracked_copy destination is detected and KEPT — gripsack never
    silently overwrites it (review finding G: this path had zero
    coverage)."""
    confdir = sandbox / "myenv" / "configs" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "settings.json").write_text('{"theme": "mocha"}\n')
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("zed", {
  config: { "configs/zed/settings.json": trackedCopy("~/.config/zed/settings.json") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = sandbox / ".config" / "zed" / "settings.json"

    # user edits the deployed file (zed rewrites its own config) — the
    # next apply detects drift and KEEPS it
    dest.write_text('{"theme": "nord", "user": true}\n')
    (confdir / "settings.json").write_text('{"theme": "latte"}\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == '{"theme": "nord", "user": true}\n'

    # drift resolved by hand (dest back to the pinned content): gripsack
    # can't tell a restore from a new drift, so it keeps once — and the
    # next apply converges and updates (bounded, no lockfile surgery)
    dest.write_text('{"theme": "mocha"}\n')
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == '{"theme": "latte"}\n'


def test_plan_diffs_against_the_current_generation(sandbox):
    """plan shows what apply WOULD do: new/update/satisfied/prune, plus
    the take-over warning for foreign paths (0004 pass 5)."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.toml").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
""",
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "+ configs/demo/a.toml" in out.stdout

    grip("apply", "--host", "testhost", cwd=repo)
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "= ~/.config/demo/a.toml (satisfied)" in out.stdout

    (confdir / "a.toml").write_text("b\n")
    (confdir / "b.toml").write_text("b\n")
    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: {
    "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml"),
    "configs/demo/b.toml": trackedCopy("~/.config/demo/b.toml"),
  },
});
"""
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "~ configs/demo/a.toml" in out.stdout
    assert "+ configs/demo/b.toml" in out.stdout

    # apply the two-entry module, then drop b → the next plan prunes it
    grip("apply", "--host", "testhost", cwd=repo)
    (sandbox / "myenv" / "modules" / "hello.ts").write_text(
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("demo", {
  config: { "configs/demo/a.toml": trackedCopy("~/.config/demo/a.toml") },
});
"""
    )
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert "- ~/.config/demo/b.toml (prune)" in out.stdout


def test_rollback_restores_template_rendered_and_merge_block(sandbox):
    """rollback through the ONE engine (0001 §3.5, review verification):
    template destinations get the previous generation's RENDERED bytes
    (re-rendered with recorded vars), and merge entries re-upsert only
    the block — the foreign file's other content survives."""
    confdir = sandbox / "myenv" / "configs" / "app"
    confdir.mkdir(parents=True)
    (confdir / "id.toml").write_text('email = "{{ email }}"\n')
    (confdir / "block.sh").write_text("export A=1\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { merge, module, template } from "@gripsack/core";

export default module("app", {
  config: {
    "configs/app/id.toml": template("~/.config/app/id.toml", { email: "a@b.c" }),
    "configs/app/block.sh": merge("~/.bashrc"),
  },
});
""",
    )
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("# user stuff\nexport EDITOR=hx\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    # generation 2: new template content and new block content
    (confdir / "id.toml").write_text('email = "rendered-v2"\nname = "{{ email }}"\n')
    (confdir / "block.sh").write_text("export A=2\n")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "rendered-v2" in (sandbox / ".config/app/id.toml").read_text()
    assert "export A=2" in bashrc.read_text()

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    # template: back to the previous generation's rendered bytes
    assert (sandbox / ".config/app/id.toml").read_text() == 'email = "a@b.c"\n'
    # merge: the block reverted, the foreign content untouched
    content = bashrc.read_text()
    assert content.startswith("# user stuff\nexport EDITOR=hx\n")
    assert "export A=1" in content
    assert "export A=2" not in content


def test_store_verify_detects_and_repairs_corruption(sandbox):
    """0008 §3 shipped: the re-hash walk catches a tampered payload and
    --repair removes it; the next apply re-fetches (publish_dir's
    refusal becomes a republish)."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("demo", {
  config: { ...tree("configs/demo", "~/.config/demo", "owned") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stderr

    # tamper with a store path — verify must catch it, repair must
    # remove it, and the next apply re-fetches
    store = sandbox / ".local/share/gripsack/store"
    target = next(store.iterdir())
    for f in target.rglob("*"):
        if f.is_file() and not f.is_symlink():
            f.chmod(0o644)  # store payloads are read-only (0016 §D3)
            f.write_text("tampered\n")
    out = grip("store-verify", cwd=repo)
    assert out.returncode != 0
    assert "corrupt" in out.stdout
    out = grip("store-verify", "--repair", cwd=repo)
    assert "removed corrupt" in out.stdout
    assert not target.exists()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert target.exists()


def only_store_path(sandbox):
    store = sandbox / ".local/share/gripsack/store"
    entries = [p.name for p in store.iterdir()]
    assert len(entries) == 1, f"expected one store path, found {entries}"
    return entries[0]


def test_mirror_swap_proves_then_dedups(sandbox):
    """0014 §3: the recipe left the store path — a changed fetch spec
    must re-fetch once to PROVE byte identity, then dedup to the same
    content path: no second store entry, no redeploy."""
    payload_a = make_tarball(sandbox / "a.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    payload_b = make_tarball(sandbox / "b.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    assert payload_a != payload_b
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload_a}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    before = only_store_path(sandbox)

    # the mirror swap: different URL, identical bytes
    module_ts = repo / "modules" / "hello.ts"
    module_ts.write_text(module_ts.read_text().replace(str(payload_a), str(payload_b)))
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert only_store_path(sandbox) == before
    # one fetch to prove identity, then nothing moved: no redeploy, no
    # new generation
    assert "fetched" in out.stdout
    assert "linked" not in out.stdout
    assert "already satisfied" in out.stdout


def test_install_mapping_edit_does_not_refetch(sandbox):
    """0014 §3: install destinations are deploy concerns, not content —
    editing the mapping keeps the content path and skips the fetch."""
    payload = make_tarball(sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    before = only_store_path(sandbox)

    module_ts = repo / "modules" / "hello.ts"
    module_ts.write_text(
        module_ts.read_text().replace("~/.local/bin/hello", "~/.local/bin/hello2")
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert only_store_path(sandbox) == before
    assert "content already in store" in out.stdout
    assert "fetched" not in out.stdout
    assert (sandbox / ".local/bin/hello2").is_symlink()


def test_store_verify_covers_fetched_modules(sandbox):
    """0014 §1a: verify used to compare the store TREE hash against the
    lock's TRANSPORT hash (never matches) under a hostname-keyed lock
    lookup (usually skipped). The manifest's tree256 is the expectation
    now — a tampered fetched payload fails verify, hostname-free."""
    payload = make_tarball(sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("hello", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/hello": symlink("~/.local/bin/hello") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr

    store = sandbox / ".local/share/gripsack/store"
    target = next(store.iterdir())
    for f in target.rglob("*"):
        if f.is_file() and not f.is_symlink():
            f.chmod(0o644)  # store payloads are read-only (0016 §D3)
            f.write_text("tampered\n")
    out = grip("store-verify", cwd=repo)
    assert out.returncode != 0
    assert "corrupt" in out.stdout
    out = grip("store-verify", "--repair", cwd=repo)
    assert "removed corrupt" in out.stdout
    # repair removed it; the next apply republishes the SAME path —
    # the lock's tree256 names the content, and the bytes haven't moved
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert only_store_path(sandbox) == target.name


def test_exec_failures_render_with_codes_and_spans(sandbox):
    """0004 §3 across the stack: a fetch failure at apply is a coded,
    span-labeled diagnostic pointing at the module — never a bare
    `error:` line. (The placeholder E114 path is unit-tested in
    gripsack-ir; this covers the apply renderer.)"""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tarball } from "@gripsack/core";

export default module("demo", {
  fetch: tarball("http://127.0.0.1:1/never-there.tar.gz"),
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E301" in out.stderr
    assert "modules/hello.ts" in out.stderr  # make_env_repo's default filename
    assert "raised here" in out.stderr


def test_fetch_placeholders_expand_from_host_facts(sandbox):
    """0016 §D1: {system}/{target}/{arch}/{arch.go}/{os} in a fetch URL
    expand from the machine's facts — one spec serves every platform."""
    import hashlib
    import io
    import tarfile
    import threading
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    payload = make_tarball(sandbox / "pkg.tar.gz", {"bin/demo": b"#!/bin/sh\necho demo\n"})
    blob = payload.read_bytes()
    sha = hashlib.sha256(blob).hexdigest()

    # {system} on this host, per 0016 §D1's table
    machine = {"x86_64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}[os.uname().machine]
    system = f"{machine}-{'darwin' if sys.platform == 'darwin' else 'linux'}"

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == f"/pkg-{system}.tar.gz":
                self.send_response(200)
                self.send_header("Content-Length", str(len(blob)))
                self.end_headers()
                self.wfile.write(blob)
            else:
                self.send_response(404)
                self.end_headers()

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]

    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ module, symlink, tarball }} from "@gripsack/core";

export default module("demo", {{
  fetch: tarball("http://127.0.0.1:{port}/pkg-{{system}}.tar.gz"),
  install: {{ "bin/demo": symlink("~/.local/bin/demo") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    server.shutdown()
    # the server only answers the EXPANDED name — a 200 proves the
    # placeholder expanded from host facts
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/demo").is_symlink()
    # the lock records the spec verbatim and pins the CONTENT (hash);
    # per-host locks (0001 §5) expand per machine at fetch time
    lock = (repo / "locks/testhost.lock").read_text()
    assert sha in lock


def test_git_fetch_floats_and_the_lock_pins_head(sandbox):
    """0016 §D2: git(url) without a rev floats to the remote's HEAD —
    pinned into the lockfile at first apply; a new upstream commit does
    NOT move an apply; `grip update` moves it deliberately."""
    git_env = {
        "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t",
        "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t",
    }
    remote = sandbox / "remote"
    remote.mkdir()
    for args in (["init", "-q"], ["add", "-A"]):
        subprocess.run(["git", *args], cwd=remote, env=git_env, check=True)
    (remote / "file.txt").write_text("v1\n")
    subprocess.run(["git", "add", "-A"], cwd=remote, env=git_env, check=True)
    subprocess.run(["git", "commit", "-qm", "v1"], cwd=remote, env=git_env, check=True)

    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ git, module, symlink }} from "@gripsack/core";

export default module("tool", {{
  fetch: git("{remote}"),
  install: {{ "file.txt": symlink("~/.local/share/tool/file.txt") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed = sandbox / ".local/share/tool/file.txt"
    assert deployed.read_text() == "v1\n"

    # upstream moves; an apply must NOT follow (the lock pins HEAD)
    (remote / "file.txt").write_text("v2\n")
    subprocess.run(["git", "commit", "-qam", "v2"], cwd=remote, env=git_env, check=True)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert deployed.read_text() == "v1\n"

    # update re-resolves HEAD and moves the pin
    out = grip("update", "tool", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert deployed.read_text() == "v2\n"


def test_store_payloads_are_read_only(sandbox):
    """0016 §D3: payload files land a-w at publish — an app writing
    through an owned symlink gets EACCES, the store stays verifiable,
    and repair/gc (which unlink via writable parent dirs) still work."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, tree } from "@gripsack/core";

export default module("demo", {
  config: { ...tree("configs/demo", "~/.config/demo", "owned") },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr

    store = sandbox / ".local/share/gripsack/store"
    payload = next(store.iterdir()) / "configs/demo/a.txt"
    assert payload.exists()
    # mode bits are the assertion that works everywhere (e2e's docker
    # stage runs as root, and root's CAP_DAC_OVERRIDE bypasses them)
    assert stat.S_IMODE(payload.stat().st_mode) & 0o222 == 0, "payload file must be read-only"

    # the app-write-through-the-symlink path: EACCES, not corruption
    if os.geteuid() != 0:
        deployed = sandbox / ".config/demo/a.txt"
        try:
            deployed.write_text("corrupt\n")
            raise AssertionError("write through an owned symlink must fail")
        except PermissionError:
            pass

    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr

    # repair still collects (unlink needs a writable parent, not file)
    for f in next(store.iterdir()).rglob("*"):
        if f.is_file() and not f.is_symlink():
            os.chmod(f, 0o644)
            f.chmod(0o644)  # store payloads are read-only (0016 §D3)
            f.write_text("tampered\n")
    out = grip("store-verify", "--repair", cwd=repo)
    assert "removed corrupt" in out.stdout


def test_store_verify_merge_and_template_hashes(sandbox):
    """Merge and template entries record DEPLOY-OUTPUT hashes (trimmed
    block, rendered bytes) — verify must recompute the same value, not
    compare the raw store file (a raw hash can never match)."""
    confdir = sandbox / "myenv" / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "block.sh").write_text("export FROB=1\n")
    (confdir / "id.toml").write_text('name = "{{ name }}"\n')
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { merge, module, template } from "@gripsack/core";

export default module("demo", {
  config: {
    "configs/demo/block.sh": merge("~/.bashrc", "#"),
    "configs/demo/id.toml": template("~/.config/demo/id.toml", { name: "t" }),
  },
});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr


def test_adopt_end_to_end_restores_originals(sandbox):
    """0015 §6: adopt generates the module, manages the destination,
    and rollback to the baseline generation restores the ORIGINAL
    real files — bytes and permission bits."""
    confdir = sandbox / ".config" / "helix"
    confdir.mkdir(parents=True)
    original = confdir / "config.toml"
    original.write_text('theme = "gruvbox"\n')
    original_mode = stat.S_IMODE(original.stat().st_mode)
    (confdir / "languages.toml").write_text("[editor]\n")
    repo = make_env_repo(sandbox / "myenv", {})

    out = grip(
        "adopt", "~/.config/helix", "--mode", "owned",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    assert "owned" in out.stdout
    assert (repo / "configs/helix/config.toml").read_text() == 'theme = "gruvbox"\n'
    assert "tree(" in (repo / "modules/helix.ts").read_text()
    assert "helix" in (repo / "hosts/testhost.ts").read_text()
    assert original.is_symlink()  # managed now

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not original.is_symlink()
    assert original.read_text() == 'theme = "gruvbox"\n'
    assert stat.S_IMODE(original.stat().st_mode) == original_mode


def test_adopt_non_interactive_takes_the_safe_default(sandbox):
    """0015 §7 S1: no tables, no guessing — with no TTY to ask, adopt
    takes tracked_copy and SAYS it chose a default."""
    confdir = sandbox / ".config" / "zed"
    confdir.mkdir(parents=True)
    (confdir / "settings.json").write_text("{}\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "~/.config/zed", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "tracked_copy" in out.stderr or "tracked_copy" in out.stdout
    assert "safe default" in out.stderr
    assert '"tracked_copy"' in (repo / "modules/zed.ts").read_text()


def test_adopt_menu_selects_on_a_tty(sandbox):
    """The interactive menu (0015 §7 S1): bare enter takes the
    highlighted safe default (tracked_copy)."""
    if not shutil.which("script"):
        pytest.skip("script(1) not available")
    confdir = sandbox / ".config" / "helix"
    confdir.mkdir(parents=True)
    (confdir / "config.toml").write_text("theme = \"x\"\n")
    repo = make_env_repo(sandbox / "myenv", {})
    grip_bin = GRIP.resolve()
    env = dict(os.environ)
    env.update({
        "HOME": str(sandbox),
        "GRIPSACK_HOME": str(sandbox / ".local/share/gripsack"),
        "GRIPSACK_TRUST_ALL": "1",
        "PATH": f"{grip_bin.parent}:{os.environ['PATH']}",
    })
    # bare enter on the menu, 'y' at the apply confirm
    out = subprocess.run(
        ["script", "-qec", "grip adopt ~/.config/helix --host testhost", "/dev/null"],
        input=b"\ny\n", capture_output=True, env=env, cwd=repo, timeout=90,
    )
    transcript = out.stdout.decode(errors="replace") + out.stderr.decode(errors="replace")
    assert "how should gripsack own these files?" in transcript
    assert "tracked_copy" in transcript
    assert '"tracked_copy"' in (repo / "modules/helix.ts").read_text()


def test_adopt_refuses_path_outside_home(sandbox):
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "/etc/hostname", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode != 0
    assert "outside your home" in out.stderr


def test_adopt_refuses_to_clobber_the_repo(sandbox):
    """0015 §7 S4: the never-clobber rule covers the repo too."""
    confdir = sandbox / ".config" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_env_repo(sandbox / "myenv", {})
    (repo / "modules/demo.ts").write_text("// hand-written, do not touch\n")
    out = grip("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode != 0
    assert "refusing to overwrite" in out.stderr
    assert (repo / "modules/demo.ts").read_text() == "// hand-written, do not touch\n"


def test_adopt_does_not_follow_directory_symlinks(sandbox):
    """0015 §7 S2: a dir symlink inside the adopted tree must not pull
    an arbitrary tree into the repo — it's skipped and reported."""
    confdir = sandbox / ".config" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    elsewhere = sandbox / "elsewhere"
    elsewhere.mkdir()
    (elsewhere / "big.txt").write_text("x" * 1000)
    (confdir / "cache").symlink_to(elsewhere, target_is_directory=True)
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "~/.config/demo", "--mode", "owned",
               "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "not followed" in out.stdout
    assert not (repo / "configs/demo/cache/big.txt").exists()
    assert (repo / "configs/demo/a.txt").read_text() == "a\n"


def test_adopt_merge_mode_manages_one_block(sandbox):
    """merge mode: adopt takes one managed block, and rollback strips
    exactly that block, leaving the original bytes."""
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("export EDITOR=hx\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.bashrc", "--mode", "merge",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    assert "merge" in out.stdout
    assert "managed block" in out.stdout
    assert "EDITOR=hx" in bashrc.read_text()  # content preserved
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert bashrc.read_text() == "export EDITOR=hx\n"


def test_adopt_refuses_an_already_managed_path(sandbox):
    confdir = sandbox / ".config" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    out = grip("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode != 0
    assert 'already managed by module "demo"' in out.stderr


def test_adopt_take_over_is_scoped(sandbox):
    """0015 §3: the adopt apply may absorb exactly the adopted
    destinations — unrelated drift is never clobbered."""
    drifted = sandbox / ".config" / "demo"
    drifted.mkdir(parents=True)
    (drifted / "a.txt").write_text("a\n")
    other = sandbox / ".config" / "other"
    other.mkdir(parents=True)
    (other / "b.txt").write_text("b\n")
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip(
        "adopt", "~/.config/other", "--mode", "tracked_copy",
        "--host", "testhost", "--yes", cwd=repo,
    )
    assert out.returncode == 0, out.stderr
    # drift the managed copy — with a global --take-over this would be
    # clobbered; adopt's scoped set contains only the NEW destinations
    drift_target = sandbox / ".config/other/b.txt"
    drift_target.write_text("user edits\n")
    out = grip("adopt", "~/.config/demo", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert drift_target.read_text() == "user edits\n"  # drift preserved


def test_adopt_rollback_keeps_post_adopt_user_edits(sandbox):
    """0015 §4's drift guard: a destination the user changed after
    adopting is theirs — rollback keeps it, prior or not."""
    confdir = sandbox / ".config" / "helix"
    confdir.mkdir(parents=True)
    (confdir / "config.toml").write_text('theme = "gruvbox"\n')
    repo = make_env_repo(sandbox / "myenv", {})
    out = grip("adopt", "~/.config/helix", "--host", "testhost", "--yes", cwd=repo)
    assert out.returncode == 0, out.stderr
    dest = confdir / "config.toml"
    dest.unlink()
    dest.write_text('theme = "mine now"\n')
    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert dest.read_text() == 'theme = "mine now"\n'


def test_run_steps_execute_with_declared_outputs(sandbox):
    """run steps (0007 §3 rung 2): structured argv, no shell — declared
    outputs are the contract and a missing one is a step error."""
    payload = make_tarball(sandbox / "hello.tar.gz", {"bin/hello": b"#!/bin/sh\necho hello\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fetchStep, installStep, module, runStep, symlink, tarball }} from "@gripsack/core";

export default module("hello", {{
  steps: [
    fetchStep(tarball("file://{payload}")),
    runStep(["cp", "bin/hello", "bin/hello-copy"], "copy", {{ outputs: ["bin/hello-copy"] }}),
    installStep({{ "bin/hello-copy": symlink("~/.local/bin/hello-copy") }}),
  ],
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".local/bin/hello-copy").is_symlink()


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


def test_third_party_npm_deps_resolve_in_module_code(sandbox):
    """Repo npm dependencies work in module code: the eval sandbox is
    read-only within the repo (node_modules included), and the pin map
    is applied via --import-map (not deno.json discovery) so BYONM
    still engages. Repos without package.json keep the embedded copy."""
    repo = make_env_repo(sandbox / "myenv", {})
    pkg = repo / "node_modules/tinypkg"
    pkg.mkdir(parents=True)
    (pkg / "package.json").write_text(
        '{"name": "tinypkg", "version": "1.0.0", "type": "module", "main": "index.js"}'
    )
    (pkg / "index.js").write_text('export const answer = 42;\n')
    (repo / "package.json").write_text(
        '{"name": "fixture", "private": true, "type": "module",'
        ' "dependencies": {"tinypkg": "1.0.0"}}'
    )
    (repo / "modules" / "usesdep.ts").write_text(
        'import { answer } from "tinypkg";\n'
        'import { module, tree } from "@gripsack/core";\n'
        'export default module("usesdep", {\n'
        '  config: tree("configs/demo", "~/.config/demo", answer === 42 ? "owned" : "merge"),\n'
        '});\n'
    )
    confdir = repo / "configs" / "demo"
    confdir.mkdir(parents=True)
    (confdir / "a.txt").write_text("a\n")
    # rewrite the host to include the new module
    host = repo / "hosts" / "testhost.ts"
    src = host.read_text().replace("modules: []", "modules: [usesdep]")
    src = src.replace(
        'import { defineEnv } from "@gripsack/core";',
        'import { defineEnv } from "@gripsack/core";\nimport usesdep from "../modules/usesdep.ts";',
    )
    host.write_text(src)
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr


def test_embedded_frontend_serves_without_node_modules(sandbox):
    """The frontend source ships in the binary (0013 D3): a repo with
    NO node_modules evals offline against the embedded copy materialized
    under $GRIPSACK_HOME/frontend/ts-<version>/. Only the runtime is
    provisioned — and the sandbox inherits deno from PATH, so nothing
    downloads (e2e is offline)."""
    repo = make_env_repo(
        sandbox / "env",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("c", {
  config: { "x.toml": trackedCopy("~/.config/x.toml") },
});
""",
    )
    (repo / "x.toml").write_text("embedded = true\n")
    assert not (repo / "node_modules").exists()
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config/x.toml").read_text() == "embedded = true\n"

    # the embedded copy was materialized at the core's version…
    gs = sandbox / ".local/share/gripsack"
    versions = sorted((gs / "frontend").glob("ts-*"))
    assert versions, "embedded frontend not materialized"
    assert (versions[-1] / "src" / "cli.ts").exists()
    # …and no runtime was fetched when deno is external (PATH or
    # GRIPSACK_DENO — the gate image always provides it; the offline
    # proof only means anything there)
    deno_external = os.environ.get("GRIPSACK_DENO") or shutil.which("deno")
    tools = gs / "tools"
    if deno_external:
        assert not tools.exists() or not list(tools.iterdir()), \
            "deno was external but a tool was downloaded anyway"

    # re-apply is satisfied — the embedded path is stable, not a one-shot
    again = grip("apply", "--host", "testhost", cwd=repo)
    assert again.returncode == 0, again.stderr
    assert "already satisfied" in again.stdout


def _core_version() -> str:
    out = subprocess.run(
        [str(GRIP.resolve()), "--version"], capture_output=True, text=True
    )
    return out.stdout.strip().split()[-1]


def _fake_release_server(sandbox, latest: str):
    """A loopback GitHub releases API for self-update tests: one core-v
    release, the platform tarball (a fake `grip`), and its sha256
    sidecar — served for all four release triples so any host matches."""
    import hashlib
    import io
    import json
    import tarfile
    import threading
    from http.server import BaseHTTPRequestHandler, HTTPServer

    fake = b"#!/bin/sh\necho fake-grip-" + latest.encode() + b"\n"
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        # the real release layout nests: gripsack-<v>-<triple>/grip
        info = tarfile.TarInfo(f"gripsack-{latest}-x86_64-unknown-linux-musl/grip")
        info.size = len(fake)
        info.mode = 0o755
        tar.addfile(info, io.BytesIO(fake))
    tarball = buf.getvalue()
    sha = hashlib.sha256(tarball).hexdigest()

    triples = [
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
    ]
    base = "http://127.0.0.1:PORT"
    assets = [
        {
            "name": f"gripsack-{latest}-{t}.tar.gz",
            "browser_download_url": f"{base}/dl/t.tar.gz",
            "url": f"{base}/api/t.tar.gz",
        }
        for t in triples
    ] + [
        {
            "name": f"gripsack-{latest}-{t}.tar.gz.sha256",
            "browser_download_url": f"{base}/dl/t.sha256",
            "url": f"{base}/api/t.sha256",
        }
        for t in triples
    ]
    listing = json.dumps([
        {"tag_name": "ts-v99.0.0", "assets": []},
        {"tag_name": f"core-v{latest}", "assets": assets},
    ]).encode()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path.startswith("/repos/"):
                body = listing
            elif self.path.endswith("t.tar.gz"):
                body = tarball
            elif self.path.endswith("t.sha256"):
                body = f"{sha}  gripsack.tar.gz\n".encode()
            else:
                self.send_response(404)
                self.end_headers()
                return
            self.send_response(200)
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *a):
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    listing = listing.replace(b"PORT", str(server.server_port).encode())
    return server, f"http://127.0.0.1:{server.server_port}"


def test_self_update_check_and_swap(sandbox, monkeypatch):
    """self-update is the package manager dogfood: resolve the newest
    core-v release (ts-v tags don't fool it), verify the sha256 sidecar,
    stage, and atomically rename over the running binary."""
    import shutil

    server, api = _fake_release_server(sandbox, "9.9.9")
    monkeypatch.setenv("GRIPSACK_UPDATE_API", api)
    fake_bin = sandbox / "bin"
    fake_bin.mkdir()
    exe = fake_bin / "grip"
    shutil.copy(GRIP.resolve(), exe)
    exe.chmod(0o755)

    def self_update(*args):
        return subprocess.run(
            [str(exe), "self-update", *args], capture_output=True, text=True, timeout=60
        )

    out = self_update("--check")
    assert out.returncode == 0, out.stderr
    assert "9.9.9" in out.stdout and "available" in out.stdout
    before = exe.read_bytes()
    out = self_update()
    assert out.returncode == 0, out.stderr
    assert "updated" in out.stdout and "9.9.9" in out.stdout
    assert exe.read_bytes() != before
    swapped = subprocess.run([str(exe)], capture_output=True, text=True)
    assert swapped.stdout.strip() == "fake-grip-9.9.9"
    server.shutdown()


def test_self_update_already_current(sandbox, monkeypatch):
    server, api = _fake_release_server(sandbox, _core_version())
    monkeypatch.setenv("GRIPSACK_UPDATE_API", api)
    out = subprocess.run(
        [str(GRIP.resolve()), "self-update", "--check"],
        capture_output=True, text=True, timeout=60,
    )
    assert out.returncode == 0, out.stderr
    assert "is current" in out.stdout
    server.shutdown()


def test_config_tree_gain_repins_and_deploys_from_store(sandbox):
    """A config tree that gains a file under an unmoved transport pin:
    `grip update` moves the pin, a warm-store apply deploys the new
    file FROM THE STORE (never a link into the repo checkout), and a
    cold store re-pins instead of dying on the stale tree hash."""
    payload = make_tarball(sandbox / "tool.tar.gz", {"bin/tool": b"#!/bin/sh\n"})
    confdir = sandbox / "myenv" / "configs" / "tool"
    confdir.mkdir(parents=True)
    (confdir / "a.conf").write_text("a\n")
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink, tree }} from "@gripsack/core";

export default module("tool", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/tool": symlink("~/.local/bin/tool") }},
  config: {{ ...tree("configs/tool", "~/.config/tool", "owned") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    store = sandbox / ".local/share/gripsack/store"
    deployed_a = sandbox / ".config" / "tool" / "a.conf"
    assert deployed_a.is_symlink()
    assert str(deployed_a.readlink()).startswith(str(store))

    # the tree gains a file: update reports the move, apply deploys it
    (confdir / "b.conf").write_text("b\n")
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "bumped" in out.stdout, out.stdout
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    deployed_b = sandbox / ".config" / "tool" / "b.conf"
    assert deployed_b.is_symlink()
    target = str(deployed_b.readlink())
    assert target.startswith(str(store)), f"new file deployed from the repo checkout: {target}"

    # cold store: the pin moved, so this re-pins instead of failing
    shutil.rmtree(store)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".config" / "tool" / "b.conf").exists()


def test_deploy_refuses_destination_resolving_into_repo(sandbox):
    """A destination whose ancestor is a symlink into the env repo
    turns a deploy into a delete of the module's own source — refuse."""
    repo = make_env_repo(
        sandbox / "myenv",
        """
import { module, trackedCopy } from "@gripsack/core";

export default module("scripts", {
  config: { ".claude/scripts/deploy.sh": trackedCopy("~/.claude-config/scripts/deploy.sh") },
});
""",
    )
    source = repo / ".claude" / "scripts"
    source.mkdir(parents=True)
    (source / "deploy.sh").write_text("#!/bin/sh\necho real\n")
    (sandbox / ".claude-config").symlink_to(repo / ".claude")

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, out.stdout
    assert "resolves inside the env repo" in out.stderr
    # the module's source survived untouched
    assert (source / "deploy.sh").read_text() == "#!/bin/sh\necho real\n"


def test_pinned_git_lock_survives_apply_and_update_says_pinned(sandbox):
    """A git rev IS the pin: apply records it as the lock's version
    (and keeps it across re-applies), and `grip update` says so."""
    remote = sandbox / "remote"
    remote.mkdir()
    env = dict(
        GIT_AUTHOR_NAME="t",
        GIT_AUTHOR_EMAIL="t@t",
        GIT_COMMITTER_NAME="t",
        GIT_COMMITTER_EMAIL="t@t",
        PATH="/usr/bin:/bin:/usr/local/bin",
        HOME=str(sandbox),
    )
    subprocess.run(["git", "init", "--quiet"], cwd=remote, env=env, check=True)
    (remote / "bin.txt").write_text("payload\n")
    subprocess.run(["git", "add", "."], cwd=remote, env=env, check=True)
    subprocess.run(["git", "commit", "--quiet", "-m", "init"], cwd=remote, env=env, check=True)
    rev = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=remote, env=env, check=True, capture_output=True, text=True
    ).stdout.strip()

    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ git, module, symlink }} from "@gripsack/core";

export default module("tpm", {{
  fetch: git("file://{remote}", "{rev}"),
  install: {{ "bin.txt": symlink("~/.local/bin/tpm") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    lock = repo / "locks" / "testhost.lock"
    assert f'"version": "{rev}"' in lock.read_text()
    # a warm re-apply must not drop the pin
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert f'"version": "{rev}"' in lock.read_text()

    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert "pinned by rev" in out.stdout
    assert "not supported" not in out.stdout


def test_failed_apply_rollback_leaves_no_placeholder_links(sandbox):
    """A mid-graph failure rolls this run's deploys back to the
    previous generation — restored links must be the EXPANDED paths
    the generation actually deployed, never placeholder-literal."""
    payload = make_tarball(sandbox / "a.tar.gz", {"linux/a.txt": b"a\n"})
    repo = make_env_repo(
        sandbox / "myenv",
        f"""
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("aa", {{
  fetch: fileFetch("{payload}"),
  install: {{ "{{os}}/a.txt": symlink("~/.local/bin/aa") }},
}});
""",
    )
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    link = sandbox / ".local/bin/aa"
    assert "{" not in str(link.readlink())

    # add a module whose fetch fails -> the apply aborts mid-graph and
    # rolls back aa's redeploy to the previous generation
    (repo / "modules" / "zz.ts").write_text(
        """
import { fileFetch, module, symlink } from "@gripsack/core";

export default module("zz", {
  fetch: fileFetch("%s/does-not-exist.tar.gz"),
  install: { "x": symlink("~/.local/bin/zz") },
});
"""
        % sandbox
    )
    refresh_host(repo)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, out.stdout
    target = str(link.readlink())
    assert "{" not in target, f"rollback wrote a placeholder-literal link: {target}"
    assert link.exists(), f"rollback left a dangling link: {target}"
