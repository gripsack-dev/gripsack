"""Crash recovery through the kernel's recover machine (plan/0056 F2 step 2).

The unrestorable-entry wedge (plan/0058 item 1): an apply is killed after it
created a file; the user then puts a directory where that file was. Recovery
refuses by name and changes nothing, `grip doctor` shows the blocking entry,
what recovery expected and found, and the repair; once the directory is moved
aside, the next apply recovers and deploys.
"""
import hashlib
import json
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


def diagnose_without_effects(sandbox, repo, monkeypatch):
    home = sandbox / ".local/share/gripsack"

    def snapshot():
        return {
            path.relative_to(home): (
                path.lstat().st_mode,
                path.readlink() if path.is_symlink() else
                path.read_bytes() if path.is_file() else None,
            )
            for path in home.rglob("*")
        }

    before = snapshot()
    trace = sandbox / "doctor-fs-effects"
    with monkeypatch.context() as env:
        env.setenv("GRIPSACK_FS_TRACE", str(trace))
        env.delenv("GRIPSACK_FS_INCLUDE_READS", raising=False)
        result = grip("doctor", cwd=repo, approve=False)
    assert snapshot() == before, "doctor changed names, modes or content"
    assert not trace.exists(), trace.read_text()  # includes file and directory syncs
    return result


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
def test_an_interrupted_apply_restores_prior_bytes_and_mode(sandbox, monkeypatch):
    repo = sandbox / "engine-env"
    declare(repo)
    destination = sandbox / ".config/wedge/a.conf"
    destination.parent.mkdir(parents=True)
    destination.write_text("mine before\n")
    destination.chmod(0o640)
    journal = crash_after_the_write(sandbox, repo, monkeypatch, "--take-over")
    # A later write in the same run can have a different immediate-before
    # file. Neither it nor the original prior is repair authority for doctor.
    record = next(p for p in journal.glob("*.json") if p.name != "run.json")
    entry = json.loads(record.read_text())
    directory = journal.parent / "prior"
    original = directory / entry["prior"]["hash"]
    middle = b"intermediate contents\n"
    digest = hashlib.sha256(middle).hexdigest()
    intermediate = directory / digest
    intermediate.write_bytes(middle)
    entry["before"] = {"kind": "file", "hash": digest, "mode": 0o640}
    record.write_text(json.dumps(entry))
    for blob in [original, intermediate]:
        blob.chmod(0o644)
    pending = diagnose_without_effects(sandbox, repo, monkeypatch)
    assert pending.returncode == 0, pending.stdout + pending.stderr
    assert f"{destination}: the prior will be restored" in pending.stdout, pending.stdout

    # recovery alone restores the prior, with its mode
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    recovered = grip("apply", cwd=repo)
    assert recovered.returncode == 0, recovered.stdout + recovered.stderr
    assert destination.read_text() == "mine before\n"
    assert destination.stat().st_mode & 0o7777 == 0o640
    assert not list(journal.glob("*.json"))
    assert all(blob.stat().st_mode & 0o777 == 0o600 for blob in [original, intermediate])


@pytest.mark.requires_fault_controls
@pytest.mark.parametrize("damaged", ["missing-prior", "corrupt-before", "unknown-entry", "broken-marker"])
def test_doctor_refuses_without_repairing_journal(sandbox, monkeypatch, damaged):
    repo = sandbox / "env"
    declare(repo)
    destination = sandbox / ".config/wedge/a.conf"
    destination.parent.mkdir(parents=True)
    destination.write_text("my prior\n")
    journal = crash_after_the_write(sandbox, repo, monkeypatch, "--take-over")
    record = next(p for p in journal.glob("*.json") if p.name != "run.json")
    entry = json.loads(record.read_text())
    directory = journal.parent / "prior"
    original = directory / entry["prior"]["hash"]
    original.chmod(0o644)
    if damaged == "missing-prior":
        original.unlink()
        reason = f"prior blob {original.name}"
    elif damaged == "corrupt-before":
        bad = directory / hashlib.sha256(b"before").hexdigest()
        bad.write_bytes(b"corrupt")
        entry["before"] = {"kind": "file", "hash": bad.name, "mode": 0o644}
        record.write_text(json.dumps(entry))
        reason = "prior blob content hash mismatch"
    elif damaged == "unknown-entry":
        entry["v"] = 250
        record.write_text(json.dumps(entry))
        reason = "wire version 250"
    else:
        (journal / "run.json").write_text("{")
        reason = "run marker"
    diagnosis = diagnose_without_effects(sandbox, repo, monkeypatch)
    assert diagnosis.returncode != 0, diagnosis.stdout
    assert "MISS  journal:" in diagnosis.stdout and reason in diagnosis.stdout
    assert "move " + str(destination) + " aside" not in diagnosis.stdout
    assert destination.read_text() == "deployed\n"
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    refused = grip("apply", cwd=repo)
    assert refused.returncode != 0, refused.stdout + refused.stderr
    assert destination.read_text() == "deployed\n"


@pytest.mark.requires_fault_controls
def test_doctor_admits_current_manifest_without_sealing(sandbox, monkeypatch):
    repo = sandbox / "env"
    declare(repo)
    applied = grip("apply", cwd=repo)
    assert applied.returncode == 0, applied.stderr
    home = sandbox / ".local/share/gripsack"
    current = (home / "current").readlink()
    generation = int(current.name)
    # Reconstitute the post-flip/pre-cleanup marker for the real published
    # selection. A pointer match alone is insufficient for committed cleanup.
    marker = home / "journal/run.json"
    marker.write_text(json.dumps({
        "version": 1, "previous_selection": None, "op": "apply",
        "target_selection": {
            "kind": "transaction", "generation": generation,
            "transaction": current.parent.name,
        },
    }))
    healthy = diagnose_without_effects(sandbox, repo, monkeypatch)
    assert healthy.returncode == 0 and "committed (the selection flipped)" in healthy.stdout
    manifest = home / current / "manifest.json"
    wire = json.loads(manifest.read_text())
    wire.get("generation", wire)["number"] = generation + 1
    manifest.write_text(json.dumps(wire))
    diagnosis = diagnose_without_effects(sandbox, repo, monkeypatch)
    assert diagnosis.returncode != 0 and "MISS  journal:" in diagnosis.stdout
    assert f"current generation {generation} manifest:" in diagnosis.stdout
    assert "directory and identity disagree" in diagnosis.stdout
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    refused = grip("apply", cwd=repo)
    assert refused.returncode != 0
    assert "directory and identity disagree" in refused.stderr + refused.stdout
    assert marker.is_file() and (home / "current").readlink() == current
