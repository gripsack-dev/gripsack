"""Real run ancestry, private log publication, selection and retention."""
import json
import os
import signal
import stat
import time
from contextlib import contextmanager
from pathlib import Path

import pytest

from conftest import (
    GRIP, grip, latest_run_log, make_workspace_repo, make_tarball, run_grip, start_grip,
)


def test_fetch_events_keep_their_output_and_run(sandbox):
    outputs = {}
    for name in ("alpha", "beta"):
        payload = make_tarball(sandbox / f"{name}.tar.gz", {"data": name.encode()})
        outputs[name] = f'''
import {{ pkg, provider, fileFetch, profile, file, artifactFile, identity, symlinkTo }} from "@gripsack/core";
export default [
  pkg("{name}", {{producer:provider(fileFetch({json.dumps(str(payload))})),
    commands:{{}},target:{{os:"linux",arch:"x86_64"}},layout:{{kind:"relocatable"}}}}),
  profile("{name}-profile", {{files:[file({{source:artifactFile("{name}","data"),
    content:identity(),destination:symlinkTo("~/.trace/{name}")}})]}}),
];
'''
    repo = make_workspace_repo(sandbox / "env", outputs)
    result = grip("update", "--host", "testhost", cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    log = latest_run_log(sandbox / ".local/share/gripsack/runs")
    assert log is not None
    fetched = []
    for line in log.read_text().splitlines():
        event = json.loads(line)
        if event.get("fields", {}).get("message") != "fetched":
            continue
        spans = event["spans"]
        names = [span["name"] for span in spans]
        assert names.index("run") < names.index("output")
        output = next(span["output"] for span in spans if span["name"] == "output")
        assert output == event["fields"]["output"]
        assert next(span["run_id"] for span in spans if span["name"] == "run") == log.stem
        fetched.append(output)
    assert sorted(fetched) == ["alpha", "beta"]


def test_log_pointer_wins_and_invalid_targets_fall_back(tmp_path: Path):
    runs = tmp_path / "runs"
    runs.mkdir()
    first = runs / "100-zzzzzz.jsonl"
    second = runs / "100-aaaaaa.jsonl"
    first.write_text("older")
    second.write_text("newer")
    os.utime(first, ns=(1_000_000, 1_000_000))
    os.utime(second, ns=(2_000_000, 2_000_000))
    latest = runs / "latest"
    latest.symlink_to(first.name)
    assert latest_run_log(runs) == first
    latest.unlink()
    outside = tmp_path / "secret.jsonl"
    outside.write_text("not a run")
    latest.symlink_to(outside)
    assert latest_run_log(runs) == second
    latest.unlink()
    latest.symlink_to("latest")
    assert latest_run_log(runs) == second


def logged_generations(sandbox, *, umask=-1):
    result = run_grip(
        [GRIP, "generations"], cwd=sandbox, umask=umask,
        capture_output=True, text=True, timeout=15,
    )
    assert result.returncode == 0, result.stderr
    runs = Path(os.environ["GRIPSACK_HOME"]) / "runs"
    target = (runs / "latest").readlink()
    assert target.parent == Path(".")
    log = runs / target
    opening = json.loads(log.read_bytes().splitlines()[0])
    assert opening["fields"]["run_id"] == log.stem
    return log


@pytest.mark.parametrize("umask", [0, 0o777])
def test_new_runs_and_logs_are_private_regardless_of_umask(sandbox, umask):
    Path(os.environ["GRIPSACK_HOME"]).mkdir(parents=True)
    log = logged_generations(sandbox, umask=umask)
    assert stat.S_IMODE(log.parent.stat().st_mode) == 0o700
    assert stat.S_IMODE(log.stat().st_mode) == 0o600


def test_refuses_alias_directory_and_foreign_pointer(sandbox):
    home = Path(os.environ["GRIPSACK_HOME"])
    home.mkdir(parents=True)
    runs = home / "runs"
    outside = sandbox / "elsewhere"
    outside.mkdir()
    outside.chmod(0o755)
    runs.symlink_to(outside)
    assert grip("generations", cwd=sandbox).returncode == 0
    assert not list(outside.iterdir())
    assert stat.S_IMODE(outside.stat().st_mode) == 0o755

    runs.unlink()
    runs.mkdir()
    (runs / "latest").write_text("not a pointer")
    assert grip("generations", cwd=sandbox).returncode == 0
    assert (runs / "latest").read_text() == "not a pointer"
    assert not list(runs.glob("*.jsonl"))


@contextmanager
def active_generations(sandbox, runs, *, crash=False):
    # Hold a real command at its stdout write, after trace initialization.
    # Draining one pipe page lets the real command return normally.
    previous = (runs / "latest").readlink()
    rfd, wfd = os.pipe()
    with os.fdopen(rfd, "rb", buffering=0) as reader, os.fdopen(wfd, "wb", buffering=0) as writer:
        os.set_blocking(wfd, False)
        with pytest.raises(BlockingIOError):
            while True:
                os.write(wfd, bytes(4096))
        os.set_blocking(wfd, True)
        with start_grip([GRIP, "generations"], cwd=sandbox, stdout=writer) as proc:
            try:
                end = time.monotonic() + 15
                while True:
                    target = (runs / "latest").readlink()
                    if target != previous and (runs / target).stat().st_size > 0:
                        break
                    assert proc.poll() is None and time.monotonic() < end
                    time.sleep(0.01)
                yield runs / target
            finally:
                if crash:
                    proc.kill()
                    assert proc.wait(timeout=15) == -signal.SIGKILL
                else:
                    reader.read(4096)
                    assert proc.wait(timeout=15) == 0


def test_prunes_only_completed_recognized_logs_and_protects_latest(sandbox):
    runs = Path(os.environ["GRIPSACK_HOME"]) / "runs"
    runs.mkdir(parents=True)
    runs.chmod(0o777)  # Upgrade from the previous umask-dependent writer.
    legacy = runs / "1700000000-abcdef.jsonl"
    legacy.write_text("unlocked older writer\n")
    (runs / "latest").symlink_to(legacy)
    hard = logged_generations(sandbox)
    assert stat.S_IMODE(runs.stat().st_mode) == 0o700
    os.link(hard, sandbox / "hard-copy")
    alias = logged_generations(sandbox)
    alias.rename(sandbox / "moved")
    alias.symlink_to(sandbox / "moved")
    foreign = runs / f"run-{'0'*32}-abcd1234.jsonl"
    foreign.write_bytes(hard.read_bytes())  # Genuine opening event, wrong ID.
    fifo = runs / f"run-{'0'*32}-abcd1235.jsonl"
    os.mkfifo(fifo)

    with active_generations(sandbox, runs, crash=True) as interrupted:
        assert interrupted.is_file()
    assert interrupted.exists()
    with active_generations(sandbox, runs) as first:
        with active_generations(sandbox, runs) as second:
            completed = [logged_generations(sandbox) for _ in range(102)]
            assert not any(path.exists() for path in completed[:2])
            assert not interrupted.exists()
            assert all(path.exists() for path in completed[2:])
            assert first.exists() and second.exists()
            assert (runs / "latest").readlink() == Path(completed[-1].name)
            # Protect latest even when it is outside the newest hundred.
            (runs / "latest").unlink()
            (runs / "latest").symlink_to(first.name)
        assert not second.exists()  # Its normal return made it eligible.
    assert (runs / "latest").resolve(strict=True) == first
    logged_generations(sandbox)
    assert not first.exists()
    assert not completed[2].exists()
    assert all(path.exists() for path in completed[3:])
    assert hard.samefile(sandbox / "hard-copy")
    assert alias.is_symlink() and alias.samefile(sandbox / "moved")
    assert foreign.read_bytes() == hard.read_bytes()
    assert fifo.is_fifo()
    assert legacy.read_text() == "unlocked older writer\n"
