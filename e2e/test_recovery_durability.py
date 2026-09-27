"""Required restore/cleanup ordering through the real recovery-only CLI.

Injected errors and process death exercise actual filesystem calls. This is
not a physical power-loss test. The trace only chooses the fault location;
the oracle checks destination state and retained journal/prior evidence.
"""
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess

import pytest
from conftest import GRIP, make_env_repo


@pytest.mark.parametrize("prior_kind", ["absent", "file", "symlink"])
@pytest.mark.parametrize("already_restored", [False, True])
def test_recovery_sync_failure_retains_evidence_on_retry(sandbox, prior_kind, already_restored):
    repo = make_env_repo(sandbox / "env", '''
import { module } from "@gripsack/core";
export default module("demo", {});
''')
    home = sandbox / ".local/share/gripsack"
    destination = sandbox / "interrupted"
    trace = sandbox / "recovery-boundaries.tsv"
    env = dict(os.environ, GRIPSACK_FS_RECOVER_ONLY="1")

    def invoke(extra=None):
        return subprocess.run(
            [str(GRIP), "apply", "--host", "testhost"], cwd=repo,
            env={**env, **(extra or {})}, capture_output=True, text=True, timeout=30,
        )

    # Provision the ordinary frontend/lock paths before introducing interrupted
    # recovery metadata; no destination or generation is created by this seam.
    warm = invoke()
    assert warm.returncode == 0, warm.stdout + warm.stderr
    journal = home / "journal"
    journal.mkdir(exist_ok=True, mode=0o700)
    journal.chmod(0o700)
    entry_path = journal / "intent.json"
    marker = journal / "run.json"
    origin = b"original private bytes\n"
    blob = home / "prior" / hashlib.sha256(origin).hexdigest()
    if prior_kind == "file":
        blob.parent.mkdir(mode=0o700, exist_ok=True)
        blob.parent.chmod(0o700)
        blob.write_bytes(origin)
        blob.chmod(0o600)
        prior = {"kind": "file", "hash": blob.name, "mode": 0o600}
    elif prior_kind == "symlink":
        prior = {"kind": "symlink", "target": "original-link"}
    else:
        prior = {"kind": "absent"}
    entry = json.dumps({
        "v": 2, "dest": str(destination), "prior": prior, "before": prior,
        "after": {"kind": "link", "target": "installed"},
    }).encode()
    run = b'{"previous_generation":null,"target_generation":1,"op":"apply"}'

    def set_destination(restored):
        destination.unlink(missing_ok=True)
        if not restored:
            destination.symlink_to("installed")
        elif prior_kind == "file":
            destination.write_bytes(origin)
            destination.chmod(0o600)
        elif prior_kind == "symlink":
            destination.symlink_to("original-link")

    def seed(restored):
        set_destination(restored)
        entry_path.write_bytes(entry)
        entry_path.chmod(0o600)
        marker.write_bytes(run)
        # A real permission-admission sync names the marker read boundary in
        # the existing trace. It separates frontend setup from recovery work.
        marker.chmod(0o644)
        trace.unlink(missing_ok=True)

    def assert_prior():
        if prior_kind == "file":
            assert destination.read_bytes() == origin
            assert destination.stat().st_mode & 0o7777 == 0o600
            assert blob.read_bytes() == origin
        elif prior_kind == "symlink":
            assert destination.readlink() == Path("original-link")
        else:
            assert not destination.exists() and not destination.is_symlink()

    def recovery_sync_cut(restored):
        seed(restored)
        result = invoke({"GRIPSACK_FS_TRACE": str(trace)})
        assert result.returncode == 0, result.stdout + result.stderr
        assert_prior()
        rows = [line.split("\t", 3) for line in trace.read_text().splitlines()]
        admitted = next(i for i, row in enumerate(rows)
                        if row[1:] == ["After", "FileSync", '"run.json"'])
        cleanup = next(i for i, row in enumerate(rows)
                       if row[1:] == ["Before", "Unlink", '"intent.json"'])
        barriers = [row for row in rows[admitted + 1:cleanup]
                    if row[1:3] == ["Before", "DirSync"]]
        assert barriers, "recovery_cleanup_without_durable_prior: " + prior_kind
        assert not entry_path.exists() and not marker.exists()
        return int(barriers[-1][0])

    # Independently exercise a restore and a retry that merely OBSERVES the
    # prior. An earlier attempt may have landed its write but failed its sync.
    cut = recovery_sync_cut(already_restored)
    retry_cut = recovery_sync_cut(True)
    for fault in ["error", "kill"]:
        seed(already_restored)
        failed = invoke({"GRIPSACK_FS_TRACE": str(trace), "GRIPSACK_FS_CUT": str(cut),
                         "GRIPSACK_FS_FAULT": fault})
        assert failed.returncode != 0, failed.stdout + failed.stderr
        if fault == "kill":
            assert failed.returncode == -signal.SIGKILL
        assert_prior()
        assert entry_path.read_bytes() == entry
        assert marker.read_bytes() == run
        assert not (home / "current").exists()

        # A second failed attempt cannot erase the evidence merely because the
        # first attempt made the prior visible. It must seal durability again.
        marker.chmod(0o644)
        trace.unlink(missing_ok=True)
        retried = invoke({"GRIPSACK_FS_TRACE": str(trace), "GRIPSACK_FS_CUT": str(retry_cut),
                          "GRIPSACK_FS_FAULT": "error"})
        assert retried.returncode != 0, retried.stdout + retried.stderr
        assert entry_path.read_bytes() == entry
        assert marker.read_bytes() == run
        assert_prior()

        completed = invoke()
        assert completed.returncode == 0, completed.stdout + completed.stderr
        assert_prior()
        assert not entry_path.exists() and not marker.exists()


def committed_fixture(sandbox):
    repo = make_env_repo(sandbox / "env", '''
import { module } from "@gripsack/core";
export default module("demo", {});
''')
    # Keep frontend provisioning out of the recorded recovery fault ordinals.
    warm = subprocess.run(
        [str(GRIP), "apply", "--host", "testhost"], cwd=repo,
        env=dict(os.environ, GRIPSACK_FS_RECOVER_ONLY="1"),
        capture_output=True, text=True, timeout=30,
    )
    assert warm.returncode == 0, warm.stdout + warm.stderr
    home = sandbox / ".local/share/gripsack"
    journal = home / "journal"
    journal.mkdir(parents=True, mode=0o700)
    journal.chmod(0o700)
    generation = home / "generations/1"
    generation.mkdir(parents=True)
    manifest = generation / "manifest.json"
    manifest.write_text('{"number":1,"modules":{}}')
    (home / "current").symlink_to("generations/1")
    destination = sandbox / "committed-link"
    destination.symlink_to("installed")
    marker = journal / "run.json"
    marker.write_text('{"previous_generation":null,"target_generation":1,"op":"apply"}')
    marker.chmod(0o644)
    entry = journal / "intent.json"
    entry.write_text(json.dumps({
        "v": 2, "dest": str(destination), "prior": {"kind": "absent"},
        "before": {"kind": "absent"}, "after": {"kind": "link", "target": "installed"},
    }))
    entry.chmod(0o600)
    return repo, home, destination, marker, entry, manifest


def test_observed_commit_is_synced_before_recovery_cleanup(sandbox):
    repo, home, destination, marker, entry, _ = committed_fixture(sandbox)
    trace = sandbox / "committed-boundaries.tsv"
    env = dict(os.environ, GRIPSACK_FS_RECOVER_ONLY="1", GRIPSACK_FS_TRACE=str(trace))

    def invoke(extra=None):
        return subprocess.run(
            [str(GRIP), "apply", "--host", "testhost"], cwd=repo,
            env={**env, **(extra or {})}, capture_output=True, text=True, timeout=30,
        )

    marker_bytes, entry_bytes = marker.read_bytes(), entry.read_bytes()
    observed = invoke()
    assert observed.returncode == 0, observed.stdout + observed.stderr
    rows = [line.split("\t", 3) for line in trace.read_text().splitlines()]
    admitted = next(i for i, row in enumerate(rows)
                    if row[1:] == ["After", "FileSync", '"run.json"'])
    cleanup = next(i for i, row in enumerate(rows)
                   if row[1:] == ["Before", "Unlink", '"intent.json"'])
    seals = [row for row in rows[admitted + 1:cleanup]
             if row[1:3] == ["Before", "DirSync"]]
    assert seals, "committed_cleanup_without_durable_current"
    cut = int(seals[-1][0])
    marker.write_bytes(marker_bytes)
    marker.chmod(0o644)
    entry.write_bytes(entry_bytes)
    entry.chmod(0o600)
    trace.unlink()
    refused = invoke({"GRIPSACK_FS_CUT": str(cut), "GRIPSACK_FS_FAULT": "error"})
    assert refused.returncode != 0, refused.stdout + refused.stderr
    assert marker.read_bytes() == marker_bytes and entry.read_bytes() == entry_bytes
    assert destination.readlink() == Path("installed")
    assert (home / "current").readlink() == Path("generations/1")
    completed = invoke()
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert not marker.exists() and not entry.exists()
    assert destination.readlink() == Path("installed")


@pytest.mark.parametrize("corruption", ["missing", "malformed"])
def test_corrupt_current_generation_retains_recovery_evidence(sandbox, corruption):
    repo, _, destination, marker, entry, manifest = committed_fixture(sandbox)
    marker_bytes, entry_bytes = marker.read_bytes(), entry.read_bytes()
    if corruption == "missing":
        manifest.unlink()
    else:
        manifest.write_bytes(b"{torn")
    result = subprocess.run(
        [str(GRIP), "apply", "--host", "testhost"], cwd=repo,
        env=dict(os.environ, GRIPSACK_FS_RECOVER_ONLY="1"),
        capture_output=True, text=True, timeout=30,
    )
    assert result.returncode != 0, "corrupt current gained commit authority"
    assert marker.read_bytes() == marker_bytes and entry.read_bytes() == entry_bytes
    assert destination.readlink() == Path("installed")
