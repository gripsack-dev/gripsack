"""Complete native workspace pins, frozen reconstruction and release metadata.

Only update resolves external package sources. Profiles consume real artifacts;
local repository content is captured separately rather than merged into a pin.
All transports are fixture files, local git repositories or loopback HTTP.
"""

from __future__ import annotations

import gzip
import hashlib
import io
import json
import os
import re
import subprocess
import tarfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest
from conftest import grip, make_workspace_repo, make_tarball

LOCK = Path("gripsack.lock")
GIT_ENV = {
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@t",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@t",
    "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
}


def run(repo: Path, *args: str):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def pins(repo: Path) -> dict:
    lock = json.loads((repo / LOCK).read_text())
    return {
        pin["output"]: pin
        for resolution in lock["resolutions"].values()
        for pin in resolution["pins"]
    }


def hex64(value: object) -> str:
    assert isinstance(value, str) and len(value) == 64, value
    assert all(c in "0123456789abcdef" for c in value), value
    return value


def package_profile(name: str, fetch: str, source: str, destination: str,
                    *, runtime: tuple[str, ...] = ()) -> str:
    return f'''import {{pkg,provider,fileFetch,tarball,git,githubRelease,
  profile,file,artifactFile,identity,trackedCopyTo}} from "@gripsack/core";
export default [
  pkg({json.dumps(name)}, {{producer:provider({fetch}), commands:{{}},
    runtime:{json.dumps(runtime)}, target:{{os:"linux",arch:"x86_64"}},
    layout:{{kind:"relocatable"}}}}),
  profile({json.dumps(name + "-files")}, {{files:[file({{
    source:artifactFile({json.dumps(name)}, {json.dumps(source)}), content:identity(),
    destination:trackedCopyTo({json.dumps(destination)})
  }})]}}),
];'''


def tarball_bytes(files: dict[str, bytes]) -> bytes:
    """Identical payloads have identical compressed transport bytes."""
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as tar:
            for name, content in sorted(files.items()):
                info = tarfile.TarInfo(name)
                info.size = len(content)
                tar.addfile(info, io.BytesIO(content))
    return buf.getvalue()


class Releases:
    """Loopback releases API retaining every tag for frozen reconstruction."""

    TRIPLES = (
        "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin", "aarch64-apple-darwin",
    )

    def __init__(self) -> None:
        self.releases: dict[str, bytes] = {}
        self.latest = ""
        self.requests: list[str] = []
        self._lock = threading.Lock()
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self._server.server_port}"

    def serve(self, tag: str, files: dict[str, bytes]) -> None:
        with self._lock:
            self.releases[tag] = tarball_bytes(files)
            self.latest = tag

    def _release(self, tag: str) -> dict:
        versions = [tag]
        if (bare := tag.removeprefix("v")) != tag:
            versions.append(bare)
        return {
            "tag_name": tag,
            "assets": [
                {"name": f"tool-{version}-{target}.tar.gz",
                 "browser_download_url": f"{self.base}/dl/{tag}/tool.tar.gz",
                 "url": f"{self.base}/api/assets/{version}-{target}"}
                for version in versions for target in self.TRIPLES
            ],
        }

    def _handler(self):
        releases = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                with releases._lock:
                    releases.requests.append(self.path)
                    known = dict(releases.releases)
                    latest = releases.latest
                if self.path == "/api/v3/repos/acme/tool/releases/latest":
                    body = json.dumps(releases._release(latest)).encode()
                elif (match := re.fullmatch(
                    r"/api/v3/repos/acme/tool/releases/tags/(.+)", self.path
                )) and match[1] in known:
                    body = json.dumps(releases._release(match[1])).encode()
                elif (match := re.fullmatch(r"/dl/(.+)/tool.tar.gz", self.path)) \
                        and match[1] in known:
                    body = known[match[1]]
                else:
                    self.send_response(404)
                    self.end_headers()
                    return
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        return Handler

    def close(self):
        self._server.shutdown()
        self._server.server_close()
        self._thread.join()


def release_outputs(base: str, *, pinned: str | None = None,
                    token: str = "version") -> str:
    version = f', version:{json.dumps(pinned)}' if pinned else ""
    source = f'''githubRelease({{repo:"acme/tool",
      asset:"tool-{{{token}}}-{{target}}.tar.gz", base_url:{json.dumps(base)}{version}}})'''
    return package_profile("rel", source, "bin/tool", "~/.local/bin/tool")


def git_remote(root: Path, files: dict[str, bytes]) -> tuple[Path, str]:
    remote = root / "remote"
    remote.mkdir()
    subprocess.run(["git", "init", "-q"], cwd=remote,
                   env={**GIT_ENV, "HOME": str(root)}, check=True)
    return remote, git_commit(remote, root, files)


def git_commit(remote: Path, root: Path, files: dict[str, bytes]) -> str:
    env = {**GIT_ENV, "HOME": str(root)}
    for name, content in files.items():
        (remote / name).write_bytes(content)
    subprocess.run(["git", "add", "-A"], cwd=remote, env=env, check=True)
    subprocess.run(["git", "commit", "-qm", "fixture"], cwd=remote, env=env, check=True)
    return subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=remote, env=env, check=True,
        capture_output=True, text=True,
    ).stdout.strip()


@pytest.mark.parametrize("cold", [False, True])
def test_first_apply_and_repeat_update_leave_complete_lock_unchanged(sandbox, monkeypatch, cold):
    payload = make_tarball(sandbox / "payload.tar.gz", {"input": b"v1\n"})
    repo = make_workspace_repo(sandbox / "env", {
        "source": package_profile("source", f"fileFetch({json.dumps(str(payload))})",
                                  "input", "~/.pins/source"),
    })
    run(repo, "update")
    baseline = (repo / LOCK).read_bytes()
    pin = pins(repo)["source"]["resolved"]
    assert pin["sha256"] == hashlib.sha256(payload.read_bytes()).hexdigest()
    assert hex64(pin["tree256"])
    assert not (sandbox / ".pins/source").exists()
    home = sandbox / ".local/share/gripsack"
    assert any((home / "store").iterdir()), "update publishes an unrooted source cache"
    assert not (home / "current").exists()
    if cold:
        # A new sandboxed state root has no cache, without deleting historical roots.
        monkeypatch.setenv("GRIPSACK_HOME", str(sandbox / "cold-state"))
    run(repo, "apply")
    assert (sandbox / ".pins/source").read_text() == "v1\n"
    assert (repo / LOCK).read_bytes() == baseline
    run(repo, "apply")
    assert (repo / LOCK).read_bytes() == baseline
    run(repo, "update")
    assert (repo / LOCK).read_bytes() == baseline
    run(repo, "store-verify")


def test_repository_template_changes_do_not_move_external_package_pins(sandbox):
    payload = make_tarball(sandbox / "payload.tar.gz", {"payload": b"payload-v1\n"})
    repo = make_workspace_repo(sandbox / "env", {
        "pkg": package_profile("data", f"tarball({json.dumps(payload.as_uri())})",
                               "payload", "~/.pins/payload"),
        "config": '''import {profile,file,repoFile,templateText,trackedCopyTo} from "@gripsack/core";
export default profile("config", {files:[file({source:repoFile("app.conf"),
  content:templateText("setting={{ value }}\\n", {value:"one"}),
  destination:trackedCopyTo("~/.config/app/app.conf")})]});''',
    })
    (repo / "app.conf").write_text("setting={{ value }}\n")
    run(repo, "update")
    original_pin = pins(repo)["data"]["resolved"]
    assert original_pin["sha256"] == hashlib.sha256(payload.read_bytes()).hexdigest()
    assert "repo256" not in original_pin
    run(repo, "apply")
    destination = sandbox / ".config/app/app.conf"
    assert destination.read_text() == "setting=one\n"
    source = repo / "outputs/config.ts"
    source.write_text(source.read_text().replace('value:"one"', 'value:"two"'))
    (repo / "app.conf").write_text("new captured template binding\n")
    run(repo, "update")
    assert pins(repo)["data"]["resolved"] == original_pin
    locked = (repo / LOCK).read_bytes()
    run(repo, "apply")
    assert destination.read_text() == "setting=two\n"
    assert (sandbox / ".pins/payload").read_text() == "payload-v1\n"
    assert (repo / LOCK).read_bytes() == locked


@pytest.mark.parametrize("token", ["version", "version.bare"])
def test_release_version_pins_and_asset_substitution(sandbox, monkeypatch, token):
    server = Releases()
    try:
        server.serve("v1.0.0", {"bin/tool": b"tool v1.0.0\n"})
        repo = make_workspace_repo(sandbox / "env", {"rel": release_outputs(server.base, token=token)})
        run(repo, "update")
        first = pins(repo)["rel"]["resolved"]
        assert first["version"] == "v1.0.0"
        assert hex64(first["sha256"]) and hex64(first["tree256"])
        locked = (repo / LOCK).read_bytes()
        run(repo, "apply")
        assert (sandbox / ".local/bin/tool").read_text() == "tool v1.0.0\n"
        assert (repo / LOCK).read_bytes() == locked
        server.serve("v1.1.0", {"bin/tool": b"tool v1.1.0\n"})
        before = len(server.requests)
        monkeypatch.setenv("GRIPSACK_HOME", str(sandbox / "cold-state"))
        run(repo, "apply")
        assert (sandbox / ".local/bin/tool").read_text() == "tool v1.0.0\n"
        assert (repo / LOCK).read_bytes() == locked
        requests = server.requests[before:]
        assert "/dl/v1.0.0/tool.tar.gz" in requests
        assert not any("/releases/" in request for request in requests)
        run(repo, "update")
        second = pins(repo)["rel"]["resolved"]
        assert second["version"] == "v1.1.0"
        assert second["sha256"] != first["sha256"]
        assert second["tree256"] != first["tree256"]
        locked = (repo / LOCK).read_bytes()
        run(repo, "apply")
        assert (sandbox / ".local/bin/tool").read_text() == "tool v1.1.0\n"
        assert (repo / LOCK).read_bytes() == locked
        run(repo, "update")
        assert (repo / LOCK).read_bytes() == locked
    finally:
        server.close()


def test_pinned_release_declaration_is_not_upgraded(sandbox):
    server = Releases()
    try:
        server.serve("v1.0.0", {"bin/tool": b"tool v1.0.0\n"})
        repo = make_workspace_repo(sandbox / "env", {
            "rel": release_outputs(server.base, pinned="v1.0.0"),
        })
        run(repo, "update")
        locked = (repo / LOCK).read_bytes()
        assert pins(repo)["rel"]["resolved"]["version"] == "v1.0.0"
        server.serve("v1.1.0", {"bin/tool": b"tool v1.1.0\n"})
        run(repo, "update")
        assert (repo / LOCK).read_bytes() == locked
        assert any("releases/tags/v1.0.0" in request for request in server.requests)
        assert not any("releases/latest" in request for request in server.requests)
    finally:
        server.close()


def test_release_tag_metadata_moves_without_payload_change(sandbox):
    server = Releases()
    try:
        files = {"bin/tool": b"identical payload\n"}
        server.serve("v1.0.0", files)
        repo = make_workspace_repo(sandbox / "env", {"rel": release_outputs(server.base)})
        run(repo, "update")
        run(repo, "apply")
        first = pins(repo)["rel"]["resolved"]
        server.serve("1.0.0", files)
        run(repo, "update")
        second = pins(repo)["rel"]["resolved"]
        assert second["version"] == "1.0.0"
        assert second["sha256"] == first["sha256"]
        assert second["tree256"] == first["tree256"]
        locked = (repo / LOCK).read_bytes()
        requests = len(server.requests)
        run(repo, "apply")
        assert len(server.requests) == requests, "metadata-only change must reuse the source cache"
        assert (sandbox / ".local/bin/tool").read_text() == "identical payload\n"
        assert (repo / LOCK).read_bytes() == locked
    finally:
        server.close()


def test_git_transports_pin_fixed_and_floating_refs(sandbox, monkeypatch):
    remote, rev1 = git_remote(sandbox, {"doc": b"v1\n"})
    subprocess.run(["git", "branch", "moving", rev1], cwd=remote, env=GIT_ENV, check=True)
    outputs = {}
    for name, revision in [("fixed", rev1), ("float", None), ("named", "moving")]:
        suffix = f", {json.dumps(revision)}" if revision else ""
        outputs[name] = package_profile(name, f"git({json.dumps(str(remote))}{suffix})",
                                        "doc", f"~/.pins/{name}")
    repo = make_workspace_repo(sandbox / "env", outputs)
    run(repo, "update")
    first = pins(repo)
    assert first["float"]["resolved"]["version"] == rev1
    assert first["fixed"]["resolved"]["version"] == rev1
    run(repo, "apply")
    rev2 = git_commit(remote, sandbox, {"doc": b"v2\n"})
    subprocess.run(["git", "branch", "-f", "moving", rev2], cwd=remote, env=GIT_ENV, check=True)
    locked = (repo / LOCK).read_bytes()
    monkeypatch.setenv("GRIPSACK_HOME", str(sandbox / "cold-state"))
    run(repo, "apply")
    assert (repo / LOCK).read_bytes() == locked
    assert all((sandbox / f".pins/{name}").read_text() == "v1\n" for name in outputs)
    run(repo, "update")
    second = pins(repo)
    assert second["float"]["resolved"]["version"] == rev2
    assert second["fixed"]["resolved"] == first["fixed"]["resolved"]
    run(repo, "apply")
    assert (sandbox / ".pins/float").read_text() == "v2\n"
    assert (sandbox / ".pins/named").read_text() == "v2\n"
    assert (sandbox / ".pins/fixed").read_text() == "v1\n"


def test_failed_update_preserves_the_lock_byte_for_byte(sandbox):
    archives = {name: make_tarball(sandbox / f"{name}.tar.gz", {"input": f"{name}-v1\n".encode()})
                for name in ("alpha", "beta")}
    repo = make_workspace_repo(sandbox / "env", {
        name: package_profile(name, f"fileFetch({json.dumps(str(archive))})",
                              "input", f"~/.pins/{name}")
        for name, archive in archives.items()
    })
    run(repo, "update")
    locked = (repo / LOCK).read_bytes()
    before = pins(repo)
    tree = sorted(str(path.relative_to(repo)) for path in repo.rglob("*"))
    make_tarball(archives["alpha"], {"input": b"alpha-v2\n"})
    saved = archives["beta"].read_bytes()
    archives["beta"].unlink()
    failed = grip("update", cwd=repo)
    assert failed.returncode != 0, failed.stdout + failed.stderr
    assert (repo / LOCK).read_bytes() == locked
    assert sorted(str(path.relative_to(repo)) for path in repo.rglob("*")) == tree
    assert not (sandbox / ".pins").exists()
    archives["beta"].write_bytes(saved)
    run(repo, "update")
    after = pins(repo)
    assert after["alpha"]["resolved"]["sha256"] != before["alpha"]["resolved"]["sha256"]
    assert after["beta"]["resolved"] == before["beta"]["resolved"]


def test_update_profile_subset_scopes_to_transitive_package_dependencies(sandbox):
    archives = {name: make_tarball(sandbox / f"{name}.tar.gz", {"input": f"{name}-v1\n".encode()})
                for name in ("lib", "app", "solo")}
    repo = make_workspace_repo(sandbox / "env", {
        name: package_profile(name, f"tarball({json.dumps(archive.as_uri())})",
                              "input", f"~/.pins/{name}",
                              runtime=("lib",) if name == "app" else ())
        for name, archive in archives.items()
    })
    run(repo, "update", "app-files")
    assert set(pins(repo)) == {"app", "lib"}
    run(repo, "update")
    assert set(pins(repo)) == {"app", "lib", "solo"}
    locked = (repo / LOCK).read_bytes()
    before = pins(repo)
    make_tarball(archives["lib"], {"input": b"lib-v2\n"})
    run(repo, "update", "solo-files")
    assert (repo / LOCK).read_bytes() == locked
    run(repo, "update", "app-files")
    refreshed = pins(repo)
    assert refreshed["lib"]["resolved"]["sha256"] != before["lib"]["resolved"]["sha256"]
    assert refreshed["app"]["resolved"] == before["app"]["resolved"]
    assert refreshed["solo"]["resolved"] == before["solo"]["resolved"]
    run(repo, "apply")
    assert (sandbox / ".pins/lib").read_text() == "lib-v2\n"
