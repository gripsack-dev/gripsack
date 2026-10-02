"""Real lifecycle composition across repeated recovery interruptions.

The snapshot is only for locating each next syscall cut. The campaign itself
keeps the previous interrupted attempt's state; it never restarts the journal.
SIGKILL exercises process death, not physical power loss.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import stat

from conftest import GRIP, make_env_repo, run_grip


def test_repeated_recovery_preserves_destination_and_collection_authority(sandbox):
    home = sandbox / "campaign-home"
    destinations = home / "destinations"
    destinations.mkdir(parents=True)
    paths = [destinations / str(index) for index in range(6)]
    for index, mode in enumerate([0o600, 0o640, 0o755]):
        paths[index].write_text(f"original-{index}\n")
        paths[index].chmod(mode)
    paths[3].symlink_to("original-three")
    paths[4].symlink_to("original-four")

    def state(path):
        if path.is_symlink():
            return "link", path.readlink()
        if path.is_file():
            return "file", path.read_bytes(), stat.S_IMODE(path.stat().st_mode)
        assert not path.exists(), str(path)
        return ("absent",)

    original = [state(path) for path in paths]
    repo = make_env_repo(sandbox / "env", '''
import { module } from "@gripsack/core";
export default module("demo", {env: {RECOVERY_VERSION: "one"}});
''')
    grip_home = home / ".local/share/gripsack"
    environment = dict(os.environ, HOME=str(home), GRIPSACK_HOME=str(grip_home))
    apply = ["apply", "--host", "testhost", "--jobs", "1"]
    trace = sandbox / "repeated-recovery.tsv"

    def invoke(arguments, extra=None):
        return run_grip([str(GRIP), *arguments], cwd=repo, env={**environment, **(extra or {})},
        capture_output=True, text=True, timeout=60,)

    def succeeds(arguments, extra=None):
        result = invoke(arguments, extra)
        assert result.returncode == 0, result.stdout + result.stderr
        return result

    succeeds(apply)
    config = ",\n".join(
        f'"payload-{index}": {"trackedCopy" if index < 3 else "symlink"}("~/destinations/{index}")'
        for index in range(6)
    )
    (repo / "modules/hello.ts").write_text(f'''
import {{ module, trackedCopy, symlink }} from "@gripsack/core";
export default module("demo", {{env: {{RECOVERY_VERSION: "two"}}, config: {{{config}}}}});
''')
    for index in range(6):
        (repo / f"payload-{index}").write_text(f"deployed-{index}\n")
    succeeds([*apply, "--take-over"])
    committed = [state(path) for path in paths]
    current = grip_home / "current"
    committed_selection = current.readlink()
    assert current.resolve() == grip_home / "generations/2"

    interrupted = invoke(["rollback", "1"], {"GRIPSACK_CRASH_AFTER": "after-rollback-restore"})
    assert interrupted.returncode == -signal.SIGABRT, interrupted.stdout + interrupted.stderr
    assert [state(path) for path in paths] == original
    assert current.readlink() == committed_selection
    journal = grip_home / "journal"
    marker = journal / "run.json"
    marker_bytes = marker.read_bytes()
    initial_entries = {path.name: path.read_bytes() for path in journal.glob("*.json") if path != marker}
    assert {json.loads(value)["dest"] for value in initial_entries.values()} == {str(path) for path in paths}

    # A durable external edit is not owned by the interrupted rollback. Every
    # later attempt must preserve it, even while other cells make progress.
    paths[1].write_text("independent-user-edit\n")
    paths[1].chmod(0o640)
    expected = list(committed)
    expected[1] = state(paths[1])
    recovery = {"GRIPSACK_FS_RECOVER_ONLY": "1"}

    def recovery_roots():
        return {
            "store": {path.name for path in (grip_home / "store").iterdir()},
            "prior": {path.name: path.read_bytes() for path in (grip_home / "prior").iterdir()},
            "generations": {path.name for path in (grip_home / "generations").iterdir() if path.name.isdecimal()},
        }

    cuts = [
        ("restore-barrier", "error"),
        ("before-entry-removal", "kill"),
        ("after-entry-removal", "kill"),
        ("after-entry-removal", "kill"),
        ("before-entry-removal", "error"),
        ("after-entry-removal", "kill"),
    ]
    for attempt, (boundary, fault) in enumerate(cuts):
        # Locate a cut in this attempt's actual input state, then restore that
        # exact private sandbox. No serialized journal field is manufactured.
        saved = sandbox / f"before-recovery-{attempt}"
        shutil.copytree(home, saved, symlinks=True)
        trace.unlink(missing_ok=True)
        succeeds(apply, {**recovery, "GRIPSACK_FS_TRACE": str(trace)})
        assert [state(path) for path in paths] == expected
        rows = [line.split("\t", 3) for line in trace.read_text().splitlines()]
        remaining = {path.name: path.read_bytes() for path in (saved / ".local/share/gripsack/journal").glob("*.json")
                     if path.name != "run.json"}
        eligible = {name for name, content in remaining.items()
                    if boundary != "restore-barrier" or json.loads(content)["dest"] != str(paths[1])}
        before = next(index for index, row in enumerate(rows)
                      if row[1:3] == ["Before", "Unlink"] and json.loads(row[3]) in eligible)
        if boundary == "restore-barrier":
            selected = next(row for row in reversed(rows[:before]) if row[1:3] == ["Before", "DirSync"])
        elif boundary == "before-entry-removal":
            selected = rows[before]
        else:
            selected = next(row for row in rows[before + 1:]
                            if row[1:] == ["After", "Unlink", rows[before][3]])
        shutil.rmtree(home)
        shutil.copytree(saved, home, symlinks=True)
        shutil.rmtree(saved)
        trace.unlink()
        roots = recovery_roots()
        failed = invoke(apply, {**recovery, "GRIPSACK_FS_TRACE": str(trace),
                               "GRIPSACK_FS_CUT": selected[0], "GRIPSACK_FS_FAULT": fault})
        assert failed.returncode != 0, f"attempt={attempt}: {failed.stdout}{failed.stderr}"
        if fault == "kill":
            assert failed.returncode == -signal.SIGKILL
        emitted = [line.split("\t", 3) for line in trace.read_text().splitlines()]
        assert emitted[int(selected[0]) - 1] == selected, f"attempt={attempt}: fault hit a different syscall"
        assert marker.read_bytes() == marker_bytes
        assert current.readlink() == committed_selection
        assert state(paths[1]) == expected[1]
        retained = {path.name: path.read_bytes() for path in journal.glob("*.json") if path != marker}
        for name, content in retained.items():
            assert content == initial_entries[name]
        for name in remaining.keys() - retained.keys():
            destination = Path(json.loads(remaining[name])["dest"])
            assert state(destination) == expected[paths.index(destination)]
        assert recovery_roots() == roots
        for arguments in (["gc"], ["gc", "--dry-run"]):
            refused = invoke(arguments)
            assert refused.returncode != 0, f"attempt={attempt}: pending recovery admitted collection"
            assert recovery_roots() == roots
            assert marker.read_bytes() == marker_bytes
            assert {path.name: path.read_bytes() for path in journal.glob("*.json") if path != marker} == retained

    succeeds(apply, recovery)
    assert [state(path) for path in paths] == expected
    assert current.readlink() == committed_selection
    assert not list(journal.glob("*.json"))
    succeeds(apply, recovery)
    assert [state(path) for path in paths] == expected
    assert current.readlink() == committed_selection

    # Subsequent lifecycles select an older generation and then reactivate the
    # newer one using fresh transaction identities, not numeric commit tests.
    succeeds(["rollback", "1"])
    older_selection = current.readlink()
    assert older_selection != committed_selection
    assert current.resolve() == grip_home / "generations/1"
    older = list(original)
    older[1] = expected[1]
    assert [state(path) for path in paths] == older
    # These restored originals are foreign to generation one, which declares
    # no destinations. Explicitly relinquish the fixture originals before
    # requesting reactivation; the independently edited file stays protected.
    for index in (0, 2, 3, 4):
        paths[index].unlink()
    succeeds(["rollback", "2"])
    assert current.resolve() == grip_home / "generations/2"
    assert current.readlink() not in {committed_selection, older_selection}
    assert [state(path) for path in paths] == expected
    succeeds(["gc"])
    assert [state(path) for path in paths] == expected
    print("6 destinations; 6 recovery interruptions including 4 process kills; collection refused at every pending cut")
