"""0042: real worker ancestry and authoritative diagnostic-log selection."""
import json
import os
from pathlib import Path

from conftest import grip, latest_run_log, make_workspace_repo, make_tarball


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
