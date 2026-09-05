"""Linter e2e: path/package/builtin-pack registrations, the crashy and
chatty fixture linters, severity policy — split from test_flow.py;
fixture repos come from conftest."""



from conftest import (
    _seed_plugin_store,
    grip,
    make_env_repo,
)



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


def test_plan_fails_on_lint_error(sandbox):
    """0033 R5: plan runs the same validation pipeline as check and
    apply — a lint error is a plan failure, not a surprise at apply."""
    repo = make_lint_repo(sandbox, "BAD_KEY = 1\n")
    out = grip("plan", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, out.stdout + out.stderr
    assert "griplint-demo/A01" in out.stderr


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
