"""Real CLI release consumers follow their own locks without source edits or network plans."""
import io
import json
import tarfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

from conftest import grip, make_workspace_repo


PLATFORM_TEMPLATE = "{os}-{arch}-{arch.go}-{arch.x64}-{target}-{system}"
PLATFORM = "linux-x86_64-amd64-x64-x86_64-unknown-linux-musl-x86_64-linux"
TOOL_TEMPLATE = "flyline-{version}-" + PLATFORM_TEMPLATE


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def evidence(label, result):
    print(f"\n--- {label} ---\n{result.stdout.strip()}")


class ReleasePathsServer:
    """Local GHE-shaped API with independently versioned packages and immutable assets."""

    def __init__(self):
        self.tags = {}
        self.assets = {}
        self.names = {}
        self.requests = []
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                path = self.path.split("?", 1)[0]
                owner.requests.append(path)
                prefix = "/api/v3/repos/acme/"
                if path.startswith(prefix):
                    package, route = path[len(prefix):].split("/", 1)
                    if package not in owner.tags or route != "releases/latest":
                        self.send_error(404)
                        return
                    tag = owner.tags[package]
                    name = owner.names[package, tag]
                    data = json.dumps({"tag_name": tag, "assets": [{
                        "name": name,
                        "browser_download_url": f"{owner.base}/downloads/{name}",
                        "url": f"{owner.base}/downloads/{name}",
                    }]}).encode()
                    media = "application/json"
                elif path.startswith("/downloads/") and path[11:] in owner.assets:
                    data = owner.assets[path[11:]]
                    media = "application/octet-stream"
                else:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header("Content-Type", media)
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

    def publish(self, revision):
        # Different tags catch accidentally borrowing a sibling package's lock.
        tags = {"tool": f"v{revision}", "library": f"v{revision + 10}", "tree": f"v{revision + 20}"}
        tool = f"flyline-{tags['tool']}-{PLATFORM}"
        library = f"libflyline.so.{revision + 10}"
        tree = f"tree-{tags['tree']}.tar.gz"
        self.assets[tool] = f"#!/bin/sh\nprintf 'tool:{tags['tool']}\\n'\n".encode()
        self.assets[library] = f"library:{tags['library']}\n".encode()
        root = f"share/{tags['tree']}"
        files = {
            f"{root}/value": f"tree:{tags['tree']}\n".encode(),
            f"{root}/private-{revision + 20}/secret": b"must not deploy\n",
            f"{root}/private-{revision + 20}-extra/public": b"segment boundary retained\n",
            f"{root}-extra/outside": b"must not include prefix sibling\n",
        }
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            for name, data in files.items():
                info = tarfile.TarInfo(name)
                info.size, info.mode = len(data), 0o644
                archive.addfile(info, io.BytesIO(data))
        self.assets[tree] = buffer.getvalue()
        for package, name in (("tool", tool), ("library", library), ("tree", tree)):
            self.names[package, tags[package]] = name
        self.tags = tags

    def close(self):
        if self.thread.is_alive():
            self.server.shutdown()
            self.server.server_close()
            self.thread.join()


@pytest.fixture
def releases(monkeypatch):
    for name in ("GH_TOKEN", "GITHUB_TOKEN", "GH_HOST", "GITHUB_HOST",
                 "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN",
                 "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("NO_PROXY", "127.0.0.1,localhost")
    server = ReleasePathsServer()
    server.publish(1)
    try:
        yield server
    finally:
        server.close()


def release_workspace(sandbox, server):
    return make_workspace_repo(sandbox / "release-paths", {"release": '''import {
  pkg, provider, githubRelease, profile, file, artifactFile, artifactTree,
  identity, trackedCopyTo, environment, task, exec, lit,
} from "@gripsack/core";
const target = {os:"linux", arch:"x86_64"} as const;
const layout = {kind:"relocatable"} as const;
const source = (repo: string, asset: string) => provider(githubRelease({
  repo: "acme/" + repo, asset, base_url: BASE,
}));
export default [
  pkg("tool", {producer:source("tool", TOOL), commands:{flyline:TOOL}, target, layout}),
  pkg("library", {producer:source("library", "libflyline.so.{version.bare}"), commands:{}, target, layout}),
  pkg("tree", {producer:source("tree", "tree-{version}.tar.gz"), commands:{}, target, layout}),
  environment("tools", {packages:["tool"], target}),
  task("probe", {environment:"tools", steps:[exec({argv:[lit("flyline")]})]}),
  profile("files", {files:[
    file({source:artifactFile("tool", TOOL), content:identity(), destination:trackedCopyTo("~/release-tool")}),
    file({source:artifactFile("library", "libflyline.so.{version.bare}"), content:identity(), destination:trackedCopyTo("~/release-library")}),
    file({source:artifactTree("tree", {include:["share/{version}"], exclude:["share/{version}/private-{version.bare}"]}),
      content:identity(), destination:trackedCopyTo("~/release-tree")}),
  ]}),
];'''.replace("BASE", json.dumps(server.base)).replace("TOOL", json.dumps(TOOL_TEMPLATE))})


def assert_deployed(sandbox, server, revision):
    tool = f"flyline-v{revision}-{PLATFORM}"
    assert (sandbox / "release-tool").read_bytes() == server.assets[tool]
    assert (sandbox / "release-library").read_bytes() == f"library:v{revision + 10}\n".encode()
    root = sandbox / f"release-tree/share/v{revision + 20}"
    assert (root / "value").read_bytes() == f"tree:v{revision + 20}\n".encode()
    assert not (root / f"private-{revision + 20}").exists()
    assert (root / f"private-{revision + 20}-extra/public").read_bytes() == b"segment boundary retained\n"
    assert not root.with_name(root.name + "-extra").exists()


def assert_plan(repo, server, revision):
    before = list(server.requests)
    result = run(repo, "plan")
    assert server.requests == before, "planning must not resolve latest or fetch payloads"
    for selected in (f"flyline-v{revision}-{PLATFORM}", f"libflyline.so.{revision + 10}",
                     f"share/v{revision + 20}", f"share/v{revision + 20}/private-{revision + 20}"):
        assert selected in result.stdout, result.stdout
    structured = json.loads(run(repo, "plan", "--json").stdout)["resolved_paths"]
    command = next(path for path in structured if path["kind"] == "command")
    assert command["path"] == f"flyline-v{revision}-{PLATFORM}"
    library = next(path for path in structured if path["kind"] == "artifact_file" and path["output"] == "library")
    assert library["selector"] == f"libflyline.so.{revision + 10}"
    tree = next(path for path in structured if path["kind"] == "artifact_tree")
    assert tree["include"] == [f"share/v{revision + 20}"]
    assert tree["exclude"] == [f"share/v{revision + 20}/private-{revision + 20}"]
    assert server.requests == before
    assert "{version}" not in result.stdout and "{version.bare}" not in result.stdout
    evidence(f"plan (locked tool v{revision})", result)


def test_lock_only_release_update_moves_all_consumers_and_replays_offline(sandbox, releases):
    repo = release_workspace(sandbox, releases)
    sources = {path.relative_to(repo): path.read_bytes() for path in repo.rglob("*.ts")}
    state = sandbox / ".local/share/gripsack"
    lock = repo / "gripsack.lock"
    evidence("update v1", run(repo, "update"))
    first_lock = lock.read_bytes()
    # Update may retain source objects, but no package realization or activation exists.
    assert not list((state / "store").glob("*workspace-package*/package.json"))
    assert not (state / "current").exists()
    assert_plan(repo, releases, 1)
    first_build = json.loads(run(repo, "build", "tool", "--json").stdout)
    first_path = Path(first_build["outputs"][0]["path"])
    run(repo, "apply", "files")
    assert_deployed(sandbox, releases, 1)
    probe = run(repo, "task", "run", "probe")
    assert "tool:v1\n" in probe.stdout
    evidence("task v1", probe)

    releases.publish(2)
    before = list(releases.requests)
    assert_plan(repo, releases, 1)
    assert json.loads(run(repo, "build", "tool", "--json").stdout) == first_build
    generation = (state / "current").readlink()
    satisfied = run(repo, "apply", "files")
    assert "already satisfied" in satisfied.stdout
    assert (state / "current").readlink() == generation
    frozen = run(repo, "task", "run", "probe")
    assert "tool:v1\n" in frozen.stdout and "tool:v2\n" not in frozen.stdout
    evidence("task while upstream is v2, lock remains v1", frozen)
    assert_deployed(sandbox, releases, 1)
    assert releases.requests == before
    assert lock.read_bytes() == first_lock

    evidence("update v2 (unchanged workspace)", run(repo, "update"))
    assert lock.read_bytes() != first_lock
    assert {path.relative_to(repo): path.read_bytes() for path in repo.rglob("*.ts")} == sources
    assert_plan(repo, releases, 2)
    second_build = json.loads(run(repo, "build", "tool", "--json").stdout)
    assert second_build["outputs"][0]["path"] != str(first_path)
    assert (first_path / f"flyline-v1-{PLATFORM}").read_bytes() == releases.assets[f"flyline-v1-{PLATFORM}"]
    run(repo, "apply", "files")
    assert (state / "current").readlink() != generation
    assert_deployed(sandbox, releases, 2)
    probe = run(repo, "task", "run", "probe")
    assert "tool:v2\n" in probe.stdout and "tool:v1\n" not in probe.stdout
    evidence("task v2", probe)

    releases.close()
    before = list(releases.requests)
    second_lock = lock.read_bytes()
    generation = (state / "current").readlink()
    assert_plan(repo, releases, 2)
    assert json.loads(run(repo, "build", "tool", "--json").stdout) == second_build
    replay = run(repo, "task", "run", "probe")
    assert "tool:v2\n" in replay.stdout
    evidence("offline retained task", replay)
    satisfied = run(repo, "apply", "files")
    assert "already satisfied" in satisfied.stdout
    assert (state / "current").readlink() == generation
    assert_deployed(sandbox, releases, 2)
    assert releases.requests == before
    assert lock.read_bytes() == second_lock
    assert {path.relative_to(repo): path.read_bytes() for path in repo.rglob("*.ts")} == sources


@pytest.mark.parametrize(("consumer", "pattern", "reason"), [
    ("command", "bin/{version}", "version"),
    ("file", "lib/{unsupported}", "unsupported"),
    ("include", "share/{version.bare}", "version"),
    ("exclude", "share/{unsupported}", "unsupported"),
])
def test_invalid_release_selectors_refuse_with_source_provenance(sandbox, consumer, pattern, reason):
    commands = {"probe": pattern} if consumer == "command" else {}
    selector = f'artifactFile("data", {json.dumps(pattern)})' if consumer == "file" else (
        f'artifactTree("data", {json.dumps({"include": ["share"], consumer: [pattern]})})'
        if consumer in ("include", "exclude") else 'artifactFile("data", "payload")'
    )
    repo = make_workspace_repo(sandbox / "invalid-paths", {"invalid": f'''import {{
  pkg, provider, fileFetch, profile, file, artifactFile, artifactTree, identity, trackedCopyTo,
}} from "@gripsack/core";
export default [
  pkg("data", {{producer:provider(fileFetch("payload")), commands:{json.dumps(commands)},
    target:{{os:"linux",arch:"x86_64"}},layout:{{kind:"relocatable"}}}}),
  profile("files", {{files:[file({{source:{selector}, content:identity(), destination:trackedCopyTo("~/must-not-deploy")}})]}}),
];'''})
    (repo / "payload").mkdir()
    (repo / "payload/payload").write_bytes(b"not a release; has no version\n")
    result = grip("plan", cwd=repo)
    assert result.returncode != 0, result.stdout + result.stderr
    diagnostic = result.stdout + result.stderr
    assert reason in diagnostic.lower(), diagnostic
    assert "invalid.ts" in diagnostic, diagnostic
    assert not (sandbox / "must-not-deploy").exists()
    assert not (sandbox / ".local/share/gripsack/current").exists()
