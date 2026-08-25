"""Lint contract tests (0011): registry resolution, request shape, and
diagnostics in the core's shape — no IR surface."""

import json
import os
import stat

import pytest

import gripsack
from gripsack import clear_graph, module, tracked_copy, tree
from gripsack.lint import run_lints


def setup_function():
    clear_graph()


def _write_linter(tmp_path, body):
    exe = tmp_path / "griplint-demo"
    exe.write_text(body)
    exe.chmod(exe.stat().st_mode | stat.S_IXUSR)
    return exe


LINTER = """#!/usr/bin/env python3
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
print(json.dumps({"type": "response", "id": 1, "result": {"linted": len(req["paths"])}}))
"""


def _repo(tmp_path, env_toml, config_text="good = true\n"):
    (tmp_path / "configs" / "demo").mkdir(parents=True)
    (tmp_path / "configs" / "demo" / "demo.toml").write_text(config_text)
    (tmp_path / "env.toml").write_text(env_toml)
    return tmp_path


def test_lint_kwarg_stays_out_of_ir(tmp_path):
    module("demo", config={"configs/demo/demo.toml": tracked_copy("~/x")}, lint="demo")
    payload = json.loads(gripsack.emit_ir())
    assert "lint" not in payload["modules"]["demo"]


def test_unregistered_linter_is_a_hard_error(tmp_path):
    repo = _repo(tmp_path, '[env]\nname = "x"\n')
    module("demo", config={"configs/demo/demo.toml": tracked_copy("~/x")}, lint="ghost")
    diagnostics = run_lints(repo, "testhost", gripsack.graph.registered_modules())
    assert len(diagnostics) == 1
    assert diagnostics[0].code == "E501"
    assert diagnostics[0].severity == "error"
    assert diagnostics[0].labels[0]["span"]["file"].endswith("test_lint.py")


def test_lint_receives_files_and_version(tmp_path, monkeypatch):
    repo = _repo(
        tmp_path,
        '[env]\nname = "x"\n\n[linters.demo]\npackage = "griplint-demo==1.0"\n',
        config_text="BAD_KEY = 1\n",
    )
    exe = _write_linter(tmp_path, LINTER)
    # package form resolves griplint-<name> next to the running python —
    # point the venv bin at our fixture instead
    import gripsack.lint as lint_mod

    monkeypatch.chdir(repo)  # tree() expands relative to cwd at eval
    module("demo", config={**tree("configs/demo", "~/.config/demo")}, lint="demo")
    monkeypatch.setattr(lint_mod, "_resolve_exe", lambda name, reg: (str(exe), None))
    diagnostics = run_lints(repo, "testhost", gripsack.graph.registered_modules())
    assert len(diagnostics) == 1
    d = diagnostics[0].to_dict()
    assert d["code"] == "griplint-demo/A01"
    assert d["severity"] == "error"
    assert d["labels"][0]["span"]["line"] == 1
    assert d["help"] == "remove it"


def test_linter_death_is_not_silent(tmp_path, monkeypatch):
    repo = _repo(tmp_path, '[env]\nname = "x"\n\n[linters.demo]\npackage = "griplint-demo==1.0"\n')
    exe = _write_linter(tmp_path, "#!/usr/bin/env python3\nimport sys; sys.exit(2)\n")
    module("demo", config={"configs/demo/demo.toml": tracked_copy("~/x")}, lint="demo")
    import gripsack.lint as lint_mod

    monkeypatch.setattr(lint_mod, "_resolve_exe", lambda name, reg: (str(exe), None))
    diagnostics = run_lints(repo, "testhost", gripsack.graph.registered_modules())
    assert len(diagnostics) == 1
    assert diagnostics[0].code == "griplint-demo/E02"
    # crash class (review finding E): a broken linter is evidence
    # about the linter, never about the config — warning, not a blocker
    assert diagnostics[0].severity == "warning"
