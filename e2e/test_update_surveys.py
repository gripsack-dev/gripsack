"""Native update surveys are complete, read-only and failure-dominant.

Release asset templates belong to acquisition; profile artifact selectors are
explicit paths. Native check/apply preflight never runs a package's contents.
"""
import io
import json
import tarfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest
from conftest import grip, make_workspace_repo, make_tarball


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def store_snapshot(sandbox):
    root = sandbox / ".local/share/gripsack/store"
    return {
        str(path.relative_to(root)): (
            path.lstat().st_mode,
            str(path.readlink()) if path.is_symlink()
            else path.read_bytes() if path.is_file() else None,
        )
        for path in root.rglob("*")
    }


def package_outputs(name, archive):
    return f'''import {{pkg,provider,fileFetch,profile,file,artifactFile,identity,trackedCopyTo}}
  from "@gripsack/core";
export default [
  pkg({json.dumps(name)}, {{producer:provider(fileFetch({json.dumps(str(archive))})),
    commands:{{}}, target:{{os:"linux",arch:"x86_64"}}, layout:{{kind:"relocatable"}}}}),
  profile({json.dumps(name + "-files")}, {{files:[file({{
    source:artifactFile({json.dumps(name)}, "payload"), content:identity(),
    destination:trackedCopyTo({json.dumps("~/." + name)})
  }})]}}),
];'''


def test_unchanged_update_preserves_exact_lock_bytes(sandbox):
    archive = make_tarball(sandbox / "stable.tar.gz", {"payload": b"stable"})
    repo = make_workspace_repo(sandbox / "stable-env", package_outputs("stable", archive))
    run(repo, "update")
    lock = repo / "gripsack.lock"
    # Formatting is not pin identity: do not normalize an unchanged lock.
    lock.write_text(json.dumps(json.loads(lock.read_text()), indent=7) + "\n")
    before = lock.read_bytes()
    before_store = store_snapshot(sandbox)
    run(repo, "update", "--check")
    assert store_snapshot(sandbox) == before_store
    run(repo, "update")
    assert lock.read_bytes() == before
    assert not (sandbox / ".stable").exists()


@pytest.mark.parametrize("failed_position", [0, 1, 2])
def test_survey_keeps_all_results_and_error_dominates_changes(sandbox, failed_position):
    names = ["alpha", "middle", "omega"]
    failed = names.pop(failed_position)
    stable, moving = names
    archive = make_tarball(sandbox / "stable.tar.gz", {"payload": b"stable"})
    changing = make_tarball(sandbox / "moving.tar.gz", {"payload": b"one"})
    paths = {failed: sandbox / "missing.tar.gz", stable: archive, moving: changing}
    repo = make_workspace_repo(sandbox / "env", {
        name: package_outputs(name, path) for name, path in paths.items()
    })
    run(repo, "update", stable + "-files", moving + "-files")
    lock = repo / "gripsack.lock"
    before_lock, before_store = lock.read_bytes(), store_snapshot(sandbox)
    make_tarball(changing, {"payload": b"two"})
    result = grip("update", "--check", cwd=repo)
    assert result.returncode == 2, result.stdout + result.stderr
    lines = result.stdout.splitlines()
    assert any(stable in line and "unchanged" in line for line in lines)
    assert any(moving in line and "would bump" in line for line in lines)
    assert any(failed in line and "failed" in line for line in lines)
    assert "incomplete" in result.stdout
    assert lock.read_bytes() == before_lock
    assert store_snapshot(sandbox) == before_store
    assert not any((sandbox / f".{name}").exists() for name in paths)
    assert not (sandbox / ".local/share/gripsack/current").exists()
    changed_only = grip("update", "--check", moving + "-files", cwd=repo)
    assert changed_only.returncode == 1, changed_only.stdout + changed_only.stderr
    run(repo, "update", "--check", stable + "-files")
    unknown = grip("update", "--check", "not-selected", cwd=repo)
    assert unknown.returncode == 2, unknown.stdout + unknown.stderr
    assert lock.read_bytes() == before_lock
    assert store_snapshot(sandbox) == before_store
    # A publishing update retains the all-or-nothing lock contract.
    assert grip("update", cwd=repo).returncode == 1
    assert lock.read_bytes() == before_lock


def test_survey_setup_failure_is_not_an_update_available_exit(sandbox):
    archive = make_tarball(sandbox / "stable.tar.gz", {"payload": b"stable"})
    repo = make_workspace_repo(sandbox / "env", package_outputs("stable", archive))
    run(repo, "update")
    lock = repo / "gripsack.lock"
    lock.write_text("{broken")
    before_store = store_snapshot(sandbox)
    failed = grip("update", "--check", cwd=repo)
    assert failed.returncode == 2, failed.stdout + failed.stderr
    assert lock.read_text() == "{broken"
    assert store_snapshot(sandbox) == before_store


class ReleaseServer:
    def __init__(self):
        self.tag = "v1"
        self.archives = {}
        self.requests = []
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                owner.requests.append(self.path)
                if self.path.startswith("/api/v3/repos/acme/tool/releases"):
                    name = f"pkg-{owner.tag[1:]}.tar.gz"
                    data = json.dumps({"tag_name": owner.tag, "assets": [{
                        "name": name, "browser_download_url": f"{owner.base}/downloads/{name}",
                        "url": f"{owner.base}/api/v3/assets/{name}",
                    }]}).encode()
                elif self.path.rsplit("/", 1)[-1] in owner.archives:
                    data = owner.archives[self.path.rsplit("/", 1)[-1]]
                else:
                    self.send_response(404)
                    self.end_headers()
                    return
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self):
        return f"http://127.0.0.1:{self.server.server_port}"

    def release(self, tag, marker):
        self.tag = tag
        buffer = io.BytesIO()
        script = f'#!/bin/sh\nprintf {tag} > "{marker}"\n'.encode()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            info = tarfile.TarInfo(f"pkg-{tag[1:]}/tool")
            info.mode, info.size = 0o755, len(script)
            archive.addfile(info, io.BytesIO(script))
        self.archives[f"pkg-{tag[1:]}.tar.gz"] = buffer.getvalue()
        return script

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def release_outputs(base, path):
    return f'''import {{pkg,provider,githubRelease,profile,file,artifactFile,identity,symlinkTo}}
  from "@gripsack/core";
export default [
  pkg("tool", {{producer:provider(githubRelease({{repo:"acme/tool",
    asset:"pkg-{{version.bare}}.tar.gz",base_url:{json.dumps(base)}}})),
    commands:{{}},target:{{os:"linux",arch:"x86_64"}},layout:{{kind:"relocatable"}}}}),
  profile("tool-files", {{files:[file({{source:artifactFile("tool", {json.dumps(path)}),
    content:identity(),destination:symlinkTo("~/.tool")}})]}}),
];'''


def test_bare_release_asset_and_explicit_historical_paths_agree(sandbox):
    server = ReleaseServer()
    marker = sandbox / "package-was-executed"
    first = server.release("v1", marker)
    try:
        repo = make_workspace_repo(sandbox / "env", {
            "tool": release_outputs(server.base, "pkg-1/tool"),
        })
        survey = grip("update", "--check", cwd=repo)
        assert survey.returncode == 1, survey.stdout + survey.stderr
        assert not (repo / "gripsack.lock").exists()
        assert not store_snapshot(sandbox)
        assert not marker.exists()
        run(repo, "update")
        lock = repo / "gripsack.lock"
        pins = [pin for resolution in json.loads(lock.read_text())["resolutions"].values()
                for pin in resolution["pins"]]
        assert len(pins) == 1 and pins[0]["resolved"]["version"] == "v1"
        assert not marker.exists()
        run(repo, "check")
        run(repo, "apply")
        assert (sandbox / ".tool").read_bytes() == first
        generation = json.loads((sandbox / ".local/share/gripsack/current/manifest.json").read_text())["number"]
        second = server.release("v2", marker)
        (repo / "outputs/tool.ts").write_text(release_outputs(server.base, "pkg-2/tool"))
        run(repo, "update")
        run(repo, "apply")
        assert (sandbox / ".tool").read_bytes() == second
        run(repo, "rollback", str(generation))
        assert (sandbox / ".tool").read_bytes() == first
        # Check inspects a retained artifact without acquiring or executing it.
        (repo / "outputs/tool.ts").write_text(release_outputs(server.base, "missing/tool"))
        request_count = len(server.requests)
        before_lock, before_store = lock.read_bytes(), store_snapshot(sandbox)
        failed = grip("check", cwd=repo)
        assert failed.returncode != 0, failed.stdout + failed.stderr
        assert len(server.requests) == request_count
        assert lock.read_bytes() == before_lock
        assert store_snapshot(sandbox) == before_store
        assert (sandbox / ".tool").read_bytes() == first
        assert not marker.exists(), "survey, preflight and file activation never execute payloads"
    finally:
        server.close()


def test_missing_artifact_preflight_precedes_any_profile_activation(sandbox):
    archive = make_tarball(sandbox / "payload.tar.gz", {"present": b"must not deploy"})
    repo = make_workspace_repo(sandbox / "env", f'''import {{pkg,provider,tarball,profile,file,
  artifactFile,identity,trackedCopyTo}} from "@gripsack/core";
export default [
  pkg("data", {{producer:provider(tarball({json.dumps(archive.as_uri())})),commands:{{}},
    target:{{os:"linux",arch:"x86_64"}},layout:{{kind:"relocatable"}}}}),
  profile("files", {{files:[
    file({{source:artifactFile("data","present"),content:identity(),destination:trackedCopyTo("~/.first")}}),
    file({{source:artifactFile("data","absent"),content:identity(),destination:trackedCopyTo("~/.second")}}),
  ]}}),
];''')
    run(repo, "update")
    # Provider realization is native acquisition, not a shell/build recipe.
    run(repo, "build", "data")
    before_lock = (repo / "gripsack.lock").read_bytes()
    before_store = store_snapshot(sandbox)
    checked = grip("check", cwd=repo)
    assert checked.returncode != 0, checked.stdout + checked.stderr
    assert store_snapshot(sandbox) == before_store
    applied = grip("apply", cwd=repo)
    assert applied.returncode != 0, applied.stdout + applied.stderr
    assert not (sandbox / ".first").exists()
    assert not (sandbox / ".second").exists()
    assert not (sandbox / ".local/share/gripsack/current").exists()
    assert (repo / "gripsack.lock").read_bytes() == before_lock
