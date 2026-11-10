"""`grip apply` streams its progress and renders each check/hook as one block.

Progress (phases, units starting) goes to stderr as it happens; step lines and
verification blocks are the report on stdout. A passing verification collapses
to a summary unless `--verbose`; a failing one always shows its full output.
"""
import json
import os
import pty
import re
import select
import shlex
import subprocess
import time

from conftest import GRIP, grip, start_grip

IMPORTS = '''import { defineWorkspace, workspace, profile, file, literalText,
  trackedCopyTo, hook, check, exec, lit } from "@gripsack/core";
'''
SGR = re.compile(r"\x1b\[[0-9;]*m")


def sh(script):
    return 'exec({argv: [lit("/bin/sh"), lit("-c"), lit(' + json.dumps(script) + ')]})'


def declare(repo, hooks=(), checks=(), content="value\n"):
    """A profile with post_activate hooks and pre_flip rendered-file checks."""
    repo.mkdir(exist_ok=True)
    outputs = [f'hook({json.dumps(name)}, {{trigger: "post_activate", run: {sh(script)}}})'
               for name, script in hooks]
    outputs += [f'check({json.dumps(name)}, {{subject: "personal", run: {sh(script)}}})'
                for name, script in checks]
    file_checks = ",".join(f'{{check: {json.dumps(name)}, subject: "rendered", stage: "pre_flip"}}'
                           for name, _ in checks)
    outputs.append(
        'profile("personal", {files: [file({content: literalText(' + json.dumps(content) + '), '
        'destination: trackedCopyTo("~/.output-fixture"), checks: [' + file_checks + ']})], '
        'hooks: ' + json.dumps([name for name, _ in hooks]) + '})')
    (repo / "gripsack.ts").write_text(IMPORTS + "export default defineWorkspace(() => workspace({outputs: [\n  "
                                      + ",\n  ".join(outputs) + "\n]}));\n")
    return repo


def block(stdout, header):
    """The contiguous lines of one verification block, header through status."""
    lines = stdout.splitlines()
    start = next(index for index, line in enumerate(lines) if header in line)
    end = next(index for index in range(start + 1, len(lines))
               if re.search(r"(✓ passed|✗ FAILED)", lines[index]))
    return lines[start:end + 1]


def test_progress_streams_while_a_hook_is_still_running(sandbox):
    """Every line before the blocked hook arrives while grip is still alive."""
    started, release = sandbox / "hook-started", sandbox / "hook-release"
    repo = declare(sandbox / "env", hooks=[(
        "gate", f"touch {shlex.quote(str(started))}; "
                f"while [ ! -e {shlex.quote(str(release))} ]; do sleep 0.05; done; echo released")])
    process = start_grip([str(GRIP), "apply"], cwd=repo,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    seen = {"out": b"", "err": b""}
    streams = {process.stdout.fileno(): "out", process.stderr.fileno(): "err"}
    try:
        deadline = time.monotonic() + 60
        while not started.exists() and time.monotonic() < deadline and process.poll() is None:
            ready, _, _ = select.select(list(streams), [], [], 0.05)
            for descriptor in ready:
                seen[streams[descriptor]] += os.read(descriptor, 65536)
        assert started.exists(), seen
        # drain what grip wrote before the hook blocked, without waiting for exit
        settle = time.monotonic() + 2
        while time.monotonic() < settle:
            ready, _, _ = select.select(list(streams), [], [], 0.05)
            for descriptor in ready:
                seen[streams[descriptor]] += os.read(descriptor, 65536)
        assert process.poll() is None, "the hook gate must still hold grip open"
        err, out = seen["err"].decode(), seen["out"].decode()
        for phase in ["preparing packages and profiles…", "deploying profiles…",
                      "activating generation…"]:
            assert phase in err, err
        assert "personal … running hook gate" in err, err
        assert "personal ✓ profile content published" in out, out
        assert "copied personal → ~/.output-fixture" in out, out
        assert "▸ hook gate" not in out, "a block is written only once its hook completes"
        assert "applied —" not in out
    finally:
        release.write_text("go")
        rest_out, rest_err = process.communicate(timeout=60)
    out = (seen["out"] + rest_out).decode()
    assert process.returncode == 0, out + (seen["err"] + rest_err).decode()
    gate = block(out, "▸ hook gate")
    assert gate[-1].strip().startswith("✓ passed in") and "released" in gate[-1], gate
    assert out.rstrip().splitlines()[-1].startswith("applied — generation 1 active"), out
    assert not SGR.search(out + (seen["err"] + rest_err).decode()), "piped output is plain"


def test_success_collapses_and_failure_expands_each_block(sandbox):
    repo = declare(sandbox / "env", hooks=[
        ("ok", "echo ok-first; echo ok-second; echo ok-third"),
        ("bad", "echo bad-out; echo bad-err >&2; exit 4"),
        ("quiet", "true"),
    ])
    applied = grip("apply", cwd=repo)
    # A failed post-activation hook never rolls back (0001 §3.8).
    assert applied.returncode == 0, applied.stdout + applied.stderr
    out = applied.stdout
    ok = block(out, "▸ hook ok")
    assert ok[0].strip() == "personal ▸ hook ok $ /bin/sh -c 'echo ok-first; echo ok-second; echo ok-third'", ok
    assert len(ok) == 2, ok
    assert "✓ passed in" in ok[1] and "ok-first (+2 more lines, --verbose shows all)" in ok[1], ok
    assert not any("ok-second" in line for line in out.splitlines() if "▸" not in line), out
    bad = block(out, "▸ hook bad")
    assert [line.strip() for line in bad[1:-1]] == ["│ bad-out", "┆ bad-err"], bad
    assert "✗ FAILED after" in bad[-1] and "exit 4" in bad[-1], bad
    assert "no automatic retry" in bad[-1], bad
    quiet = block(out, "▸ hook quiet")
    assert len(quiet) == 2 and quiet[1].rstrip().endswith("· no output"), quiet
    # raw tool output never escapes its block
    assert all(line.startswith(" ") or line.startswith("applied")
               for line in out.splitlines()), out
    # the run log keeps every line the terminal collapsed
    log = (sandbox / ".local/share/gripsack/runs/latest").read_text()
    records = [json.loads(line)["fields"] for line in log.splitlines()
               if '"verification output"' in line]
    recorded = {record["name"]: record for record in records}
    assert recorded["ok"]["stdout"] == "ok-first\nok-second\nok-third\n"
    assert recorded["bad"]["passed"] is False and recorded["bad"]["stderr"] == "bad-err\n"


def test_verbose_shows_passing_output_in_full(sandbox):
    repo = declare(sandbox / "env", hooks=[("ok", "echo ok-first; echo ok-second")])
    applied = grip("apply", "--verbose", cwd=repo)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    ok = block(applied.stdout, "▸ hook ok")
    assert [line.strip() for line in ok[1:-1]] == ["│ ok-first", "│ ok-second"], ok
    assert ok[-1].strip().startswith("✓ passed in") and "more line" not in ok[-1], ok


def test_failed_check_block_precedes_its_diagnostic(sandbox):
    destination = sandbox / ".output-fixture"
    repo = declare(sandbox / "env", checks=[
        ("validate", "echo checking; echo 'value is invalid' >&2; exit 5")])
    applied = grip("apply", cwd=repo)
    assert applied.returncode != 0
    validate = block(applied.stdout, "▸ check validate")
    assert validate[0].strip() == "personal ▸ check validate $ /bin/sh -c 'echo checking; echo '\\''value is invalid'\\'' >&2; exit 5'", validate
    assert [line.strip() for line in validate[1:-1]] == ["│ checking", "┆ value is invalid"]
    assert "✗ FAILED after" in validate[-1] and "(exit 5)" in validate[-1], validate
    assert "file check failed" in applied.stderr
    assert not destination.exists(), "a failed pre-flip check never deploys"


def test_terminal_output_respects_no_color(sandbox):
    repo = declare(sandbox / "env", hooks=[("ok", "echo ok-first")])

    def run_on_terminal(extra):
        leader, follower = pty.openpty()
        environment = {key: value for key, value in os.environ.items() if key != "NO_COLOR"}
        environment.update(extra)
        process = start_grip([str(GRIP), "apply"], cwd=repo, env=environment,
                             stdin=follower, stdout=follower, stderr=follower)
        os.close(follower)
        captured = b""
        while True:
            try:
                chunk = os.read(leader, 65536)
            except OSError:
                break
            if not chunk:
                break
            captured += chunk
        assert process.wait(timeout=60) == 0, captured
        os.close(leader)
        return captured.decode(errors="replace")

    plain = run_on_terminal({"NO_COLOR": "1"})
    assert "▸ hook ok" in plain and "applied —" in plain, plain
    assert not SGR.search(plain), "NO_COLOR forbids every colour sequence"
    (repo / "gripsack.ts").write_text((repo / "gripsack.ts").read_text().replace("ok-first", "ok-again"))
    coloured = run_on_terminal({})
    assert SGR.search(coloured), coloured
    assert "▸ hook ok" in SGR.sub("", coloured), coloured
