"""Stable literal selectors across lock-only archive updates and retained replay."""
import json
from pathlib import Path

from conftest import grip, make_toolchain_tarball


def test_single_directory_lock_update_and_retained_consumers(sandbox):
    archive = sandbox / "ripgrep.tar.gz"
    repo = sandbox / "archive-roots"
    repo.mkdir()
    source = '''import {
      defineWorkspace, workspace, pkg, provider, tarball, profile, file,
      artifactFile, artifactTree, identity, trackedCopyTo, environment, task, exec, lit,
    } from "@gripsack/core";
    export default defineWorkspace(() => workspace({outputs:[
      pkg("rg", {producer:provider(tarball(URL)), archiveRoot:{kind:"single_directory"},
        commands:{rg:"rg"},target:{os:"linux",arch:"x86_64"},layout:{kind:"relocatable"}}),
      environment("tools",{packages:["rg"],target:{os:"linux",arch:"x86_64"}}),
      task("probe",{environment:"tools",steps:[exec({argv:[lit("rg")]})]}),
      profile("files",{files:[
        file({source:artifactFile("rg","share/value"),content:identity(),destination:trackedCopyTo("~/root-value")}),
        file({source:artifactTree("rg",{include:["share"]}),content:identity(),destination:trackedCopyTo("~/root-tree")}),
      ]}),
    ]}));
    '''.replace("URL", json.dumps(archive.as_uri()))
    (repo / "gripsack.ts").write_text(source)

    def run(*args):
        result = grip(*args, cwd=repo)
        assert result.returncode == 0, result.stdout + result.stderr
        return result

    identities = []
    for version in ("v1", "v2"):
        make_toolchain_tarball(archive, {
            f"ripgrep-{version}-x86_64/rg": f"#!/bin/sh\nprintf {version}\\n\n".encode(),
            f"ripgrep-{version}-x86_64/share/value": version.encode(),
        })
        run("update")
        built = json.loads(run("build", "rg", "--json").stdout)
        selected = Path(built["outputs"][0]["path"])
        assert selected.name == f"ripgrep-{version}-x86_64"
        identities.append(str(selected))
        run("apply", "files")
        assert (sandbox / "root-value").read_text() == version
        assert (sandbox / "root-tree/share/value").read_text() == version
        assert version in run("task", "run", "probe").stdout
        assert (repo / "gripsack.ts").read_text() == source
    assert identities[0] != identities[1]
    archive.unlink()
    state = sandbox / ".local/share/gripsack"
    # Native acquisition retains immutable source objects, not builder caches.
    assert not (state / "buildkit").exists()
    assert "v2" in run("task", "run", "probe").stdout
    run("apply", "files")
    assert (sandbox / "root-value").read_text() == "v2"
    assert (sandbox / "root-tree/share/value").read_text() == "v2"

    receipts = list((state / "store").glob("*workspace-package*/package.json"))
    assert receipts
    receipt_path = next(path for path in receipts if json.loads(path.read_text()).get("selected_root") == "ripgrep-v2-x86_64")
    receipt = json.loads(receipt_path.read_text())
    assert receipt["version"] == 3
    for changes in (
        {"selected_root": "../forged"},
        {"archive_root": None},
        {"version": 2},
    ):
        receipt_path.chmod(0o644)
        receipt_path.write_text(json.dumps({**receipt, **changes}))
        refused = grip("build", "rg", "--json", cwd=repo)
        assert refused.returncode != 0
