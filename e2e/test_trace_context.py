"""0042: real worker ancestry and authoritative diagnostic-log selection."""
import json
import os
from pathlib import Path

from conftest import grip, latest_run_log, make_env_repo, make_tarball


def test_parallel_fetch_events_keep_their_module_and_run(sandbox):
    modules = {}
    for name in ("alpha", "beta"):
        payload = make_tarball(sandbox / f"{name}.tar.gz", {"data": name.encode()})
        modules[name] = f'''
import {{ module, fileFetch, fetchStep, installStep, symlink }} from "@gripsack/core";
export default module("{name}", {{ steps: [
  fetchStep(fileFetch({json.dumps(str(payload))}), "fetch-{name}"),
  installStep({{ data: symlink("~/.trace/{name}") }}, "install-{name}", {{ needs: ["fetch-{name}"] }}),
] }});
'''
    repo = make_env_repo(sandbox / "env", modules)
    result = grip("apply", "--host", "testhost", "--jobs", "2", cwd=repo)
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
        assert names.index("run") < names.index("module") < names.index("step")
        module = next(span["module"] for span in spans if span["name"] == "module")
        step = next(span["step"] for span in spans if span["name"] == "step")
        assert step == event["fields"]["step"] == f"fetch-{module}"
        assert next(span["run_id"] for span in spans if span["name"] == "run") == log.stem
        fetched.append(module)
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
