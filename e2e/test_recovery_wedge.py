"""Crash recovery through the kernel's recover machine (plan/0056 F2 step 2).

The unrestorable-entry wedge (plan/0058 item 1): an apply is killed after it
created a file; the user then puts a directory where that file was. Recovery
refuses by name and changes nothing, `grip doctor` shows the blocking entry,
what recovery expected and found, and the repair; once the directory is moved
aside, the next apply recovers and deploys.

The legacy engine stays behind the debug-only GRIPSACK_ENGINE switch for one
release; an ordinary interrupted apply recovers identically on both.
"""
import signal

import pytest

from conftest import grip

IMPORTS = '''import {
  defineWorkspace, workspace, profile, file, literalText, trackedCopyTo,
} from "@gripsack/core";
'''


def declare(repo):
    repo.mkdir(exist_ok=True)
    (repo / "gripsack.ts").write_text(
        IMPORTS + "export default defineWorkspace(() => workspace({ outputs: [\n"
        'profile("wedge", { files: [file({ content: literalText("deployed\\n"), '
        'destination: trackedCopyTo("~/.config/wedge/a.conf") })] })'
        "\n] }));\n"
    )


def crash_after_the_write(sandbox, repo, monkeypatch, *flags):
    """Kill an apply right after its one file landed; return its journal."""
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "workspace-file:1")
    crashed = grip("apply", *flags, cwd=repo)
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    assert crashed.returncode == -signal.SIGABRT, crashed.stdout + crashed.stderr
    assert (sandbox / ".config/wedge/a.conf").read_text() == "deployed\n"
    assert not (sandbox / ".local/share/gripsack/current").exists()
    journal = sandbox / ".local/share/gripsack/journal"
    assert len(list(journal.glob("*.json"))) == 2, "the run marker and one entry"
    return journal


@pytest.mark.requires_fault_controls
def test_a_directory_at_an_interrupted_entry_is_refused_by_name_until_moved(
    sandbox, monkeypatch
):
    repo = sandbox / "wedge-env"
    declare(repo)
    destination = sandbox / ".config/wedge/a.conf"
    journal = crash_after_the_write(sandbox, repo, monkeypatch)
    records = {path.name: path.read_bytes() for path in journal.glob("*.json")}

    # the user replaces the half-deployed file with a directory of their own
    destination.unlink()
    destination.mkdir()
    (destination / "mine").write_text("user data\n")

    refused = grip("apply", cwd=repo)
    output = refused.stdout + refused.stderr
    assert refused.returncode != 0, output
    for needle in [str(destination), "a directory", "Move", "aside", "grip doctor"]:
        assert needle in output, f"{needle!r} missing from: {output}"
    assert {path.name: path.read_bytes() for path in journal.glob("*.json")} == records
    assert (destination / "mine").read_text() == "user data\n"
    assert not (sandbox / ".local/share/gripsack/current").exists()

    doctor = grip("doctor", cwd=repo)
    assert doctor.returncode != 0, doctor.stdout + doctor.stderr
    assert "journal: an interrupted run left recovery pending" in doctor.stdout
    assert f"{destination}: BLOCKED: found a directory" in doctor.stdout
    assert "expected: the interrupted run left the file it wrote" in doctor.stdout
    assert f"move {destination} aside" in doctor.stdout

    # the repair: move it aside, run again
    aside = destination.with_name("a.conf.mine")
    destination.rename(aside)
    repaired = grip("apply", cwd=repo)
    assert repaired.returncode == 0, repaired.stdout + repaired.stderr
    assert destination.read_text() == "deployed\n"
    assert (aside / "mine").read_text() == "user data\n"
    assert not list(journal.glob("*.json"))
    assert "journal: nothing pending" in grip("doctor", cwd=repo).stdout


@pytest.mark.requires_fault_controls
@pytest.mark.parametrize("engine", ["kernel", "legacy"])
def test_an_interrupted_apply_recovers_on_both_engines(sandbox, monkeypatch, engine):
    repo = sandbox / "engine-env"
    declare(repo)
    destination = sandbox / ".config/wedge/a.conf"
    destination.parent.mkdir(parents=True)
    destination.write_text("mine before\n")
    destination.chmod(0o640)
    journal = crash_after_the_write(sandbox, repo, monkeypatch, "--take-over")
    pending = grip("doctor", cwd=repo)
    assert f"{destination}: the prior will be restored" in pending.stdout, pending.stdout

    # recovery alone restores the prior, with its mode
    monkeypatch.setenv("GRIPSACK_ENGINE", engine)
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    recovered = grip("apply", cwd=repo)
    assert recovered.returncode == 0, recovered.stdout + recovered.stderr
    assert destination.read_text() == "mine before\n"
    assert destination.stat().st_mode & 0o7777 == 0o640
    assert not list(journal.glob("*.json"))
