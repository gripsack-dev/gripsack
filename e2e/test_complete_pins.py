"""0042 C: complete update-time pins, without deployment.

`grip update` resolves, acquires and finalizes every selected module's
pin in one pass and writes the lock once, after all of them succeed —
so the first warm AND cold apply, repeated updates, and failures all
leave the lock byte-identical. Source-only modules additionally pin the
merged tree (repo overlay included) as an unrooted cache; recipe
modules pin source identity only and their recipes NEVER run here.

Offline: file tarballs, local git remotes, and a loopback releases API
for the github_release transport. Real CLI, real frontend, no mocks.
"""

from __future__ import annotations

import gzip
import hashlib
import io
import json
import os
import re
import shutil
import stat
import subprocess
import tarfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from conftest import grip, make_env_repo, make_tarball

LOCK = Path("locks/testhost.lock")
GIT_ENV = {
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@t",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@t",
    "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
}


def pins(repo: Path) -> dict:
    return json.loads((repo / LOCK).read_text())["modules"]


def hex64(value: object) -> str:
    assert isinstance(value, str) and len(value) == 64, value
    assert all(c in "0123456789abcdef" for c in value), value
    return value


def tarball_bytes(files: dict[str, bytes]) -> bytes:
    """Deterministic fixture tarballs: zeroed gzip mtime, so identical
    file sets always produce identical transport bytes (a re-tagged
    release with the same payload must not move the transport pin)."""
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as tar:
            for name, content in sorted(files.items()):
                info = tarfile.TarInfo(name)
                info.size = len(content)
                tar.addfile(info, io.BytesIO(content))
    return buf.getvalue()


def _make_removable(path: Path) -> None:
    for entry in [path, *path.rglob("*")]:
        if not entry.is_symlink():
            entry.chmod(
                entry.stat().st_mode | stat.S_IWUSR | stat.S_IRUSR
                | (stat.S_IXUSR if entry.is_dir() else 0)
            )


def remove_unrooted_store(home: Path) -> None:
    """Whole-store eviction is legal only BEFORE the first apply: the
    update-time source cache is unrooted then (0042 C). Once a
    generation exists, every rooted path must survive — see
    evict_generation_paths for the scoped cold reconstruct."""
    store = home / ".local/share/gripsack/store"
    if store.exists():
        _make_removable(store)
        shutil.rmtree(store)


def evict_generation_paths(sandbox: Path) -> None:
    """Cold reconstruct with roots preserved: remove only the current
    generation's module artifacts — exactly what this pinned apply
    rebuilds. Distinct historical roots stay on disk, so a strict
    `store-verify` stays green afterwards."""
    home = sandbox / ".local/share/gripsack"
    manifest = json.loads((home / "current/manifest.json").read_bytes())
    for state in manifest["modules"].values():
        path = Path(state["store_path"])
        if path.exists():
            _make_removable(path)
            shutil.rmtree(path)


def repo_tree(repo: Path) -> list[str]:
    return sorted(str(p.relative_to(repo)) for p in repo.rglob("*"))


# --------------------------------------------------------------------------
# loopback github_release transport


class Releases:
    """A loopback GitHub-releases API for github_release fetch specs
    (0002 §8). Every served release stays resolvable by its tag after a
    newer one becomes latest — pinned declarations resolve through
    /releases/tags/<tag>, exactly like the real registry. Asset names
    cover both tag spellings and every release triple, so the
    {version}/{target} pattern expands against any host."""

    TRIPLES = (
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
    )

    def __init__(self) -> None:
        self.releases: dict[str, bytes] = {}
        self.latest = ""
        self.requests: list[str] = []
        self._lock = threading.Lock()
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        threading.Thread(target=self._server.serve_forever, daemon=True).start()

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
        names = [f"tool-{v}-{t}.tar.gz" for v in versions for t in self.TRIPLES]
        return {
            "tag_name": tag,
            "assets": [
                {
                    "name": name,
                    "browser_download_url": f"{self.base}/dl/{tag}/tool.tar.gz",
                    "url": f"{self.base}/api/assets/{name}",
                }
                for name in names
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
                elif (m := re.fullmatch(
                    r"/api/v3/repos/acme/tool/releases/tags/(.+)", self.path
                )) and m[1] in known:
                    body = json.dumps(releases._release(m[1])).encode()
                elif (m := re.fullmatch(r"/dl/(.+)/tool.tar.gz", self.path)) \
                        and m[1] in known:
                    body = known[m[1]]
                else:
                    self.send_response(404)
                    self.end_headers()
                    return
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        return Handler


def release_module(base: str, *, pinned: str | None = None) -> str:
    version = f', version: {json.dumps(pinned)}' if pinned else ""
    return f"""import {{ githubRelease, module, trackedCopy }} from "@gripsack/core";
export default module("rel", {{
  fetch: githubRelease({{
    repo: "acme/tool",
    asset: "tool-{{version}}-{{target}}.tar.gz",
    base_url: {json.dumps(base)}{version},
  }}),
  install: {{ "tool-{{version}}/bin/tool": trackedCopy("~/.local/bin/tool") }},
}});"""


# --------------------------------------------------------------------------
# local git remotes


def git_remote(root: Path, files: dict[str, bytes]) -> tuple[Path, str]:
    remote = root / "remote"
    remote.mkdir()
    subprocess.run(["git", "init", "-q"], cwd=remote, env={**GIT_ENV, "HOME": str(root)},
                   check=True)
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


def git_module(name: str, remote: Path, dest: str, rev: str | None = None) -> str:
    rev_arg = f", {json.dumps(rev)}" if rev else ""
    return f"""import {{ git, module, trackedCopy }} from "@gripsack/core";
export default module({json.dumps(name)}, {{
  fetch: git({json.dumps(str(remote))}{rev_arg}),
  install: {{ doc: trackedCopy({json.dumps(dest)}) }},
}});"""


# --------------------------------------------------------------------------
# 0042 C acceptance


def test_first_warm_and_cold_apply_and_repeat_update_leave_lock_unchanged(sandbox):
    """The core acceptance: after update, the first WARM apply (cache
    present), the first COLD apply (unrooted cache evicted) and a
    repeated update all leave lock bytes untouched."""
    payload = make_tarball(sandbox / "payload.tar.gz", {"input": b"v1\n"})
    repo = make_env_repo(sandbox / "env", {
        "source": f"""import {{ fileFetch, module, trackedCopy }} from "@gripsack/core";
export default module("source", {{
  fetch: fileFetch({json.dumps(str(payload))}),
  install: {{ input: trackedCopy("~/.pins/source") }},
}});""",
        "recipe": f"""import {{ fetchStep, fileFetch, installStep, module,
  runStep, trackedCopy }} from "@gripsack/core";
export default module("recipe", {{ steps: [
  fetchStep(fileFetch({json.dumps(str(payload))})),
  runStep(["cp", "input", "built"], "build", {{ needs: ["fetch"], outputs: ["built"] }}),
  installStep({{ built: trackedCopy("~/.pins/recipe") }}, "install", {{ needs: ["build"] }}),
] }});""",
    })
    dests = (sandbox / ".pins/source", sandbox / ".pins/recipe")

    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    baseline = (repo / LOCK).read_bytes()
    sha = hashlib.sha256(payload.read_bytes()).hexdigest()
    first = pins(repo)
    # source-only: the merged tree is pinned; recipes: source identity only
    assert first["source"]["resolved"]["sha256"] == sha
    assert first["recipe"]["resolved"]["sha256"] == sha
    assert hex64(first["source"]["resolved"]["tree256"])
    assert "tree256" not in first["recipe"]["resolved"]

    # first WARM apply — the unrooted update cache is already there
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / LOCK).read_bytes() == baseline
    assert all(path.read_text() == "v1\n" for path in dests)

    # first COLD apply — evict the unrooted-legal way: no generation
    # exists yet only if nothing was applied; here generation 1 roots
    # the paths, so the scoped eviction is the cold reconstruct.
    evict_generation_paths(sandbox)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / LOCK).read_bytes() == baseline
    assert all(path.read_text() == "v1\n" for path in dests)
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr

    # a satisfied re-apply and a repeated update are both byte-stable
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / LOCK).read_bytes() == baseline
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / LOCK).read_bytes() == baseline


def test_unrooted_cache_eviction_cold_first_apply(sandbox):
    """The update-time source cache is unrooted until the first apply —
    evicting the whole store then is the one legal whole-store wipe
    (0042 C), and the cold FIRST apply reconstructs from the pins
    without touching the lock."""
    payload = make_tarball(sandbox / "payload.tar.gz", {"input": b"v1\n"})
    repo = make_env_repo(sandbox / "env", {
        "source": f"""import {{ fileFetch, module, trackedCopy }} from "@gripsack/core";
export default module("source", {{
  fetch: fileFetch({json.dumps(str(payload))}),
  install: {{ input: trackedCopy("~/.pins/source") }},
}});""",
    })
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    baseline = (repo / LOCK).read_bytes()
    store = sandbox / ".local/share/gripsack/store"
    assert store.is_dir() and any(store.iterdir()), "update must publish the cache"

    remove_unrooted_store(sandbox)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / LOCK).read_bytes() == baseline
    assert (sandbox / ".pins/source").read_text() == "v1\n"
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr

    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / LOCK).read_bytes() == baseline, "repeated update must be stable"


def test_repo_overlay_moves_tree_pin_not_transport_pin(sandbox):
    """A repo-sourced config file joins the staged overlay: editing it
    moves repo256/tree256 (and the deployed bytes) without touching the
    transport sha256 — the pin split 0042 C completes."""
    payload = make_tarball(sandbox / "payload.tar.gz", {"payload": b"payload-v1\n"})
    repo = make_env_repo(sandbox / "env", {
        "cfg": f"""import {{ fileFetch, module, trackedCopy }} from "@gripsack/core";
export default module("cfg", {{
  fetch: fileFetch({json.dumps(str(payload))}),
  install: {{
    payload: trackedCopy("~/.pins/overlay/payload"),
    "app.conf": trackedCopy("~/.config/app/app.conf"),
  }},
}});""",
    })
    (repo / "app.conf").write_text("conf-v1\n")
    conf_dest = sandbox / ".config/app/app.conf"

    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    v1 = pins(repo)["cfg"]["resolved"]
    transport = v1["sha256"]
    assert transport == hashlib.sha256(payload.read_bytes()).hexdigest()
    assert hex64(v1["repo256"]) and hex64(v1["tree256"])

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert conf_dest.read_text() == "conf-v1\n"
    locked = (repo / LOCK).read_bytes()

    # the repo overlay moves; the fetched payload does not
    (repo / "app.conf").write_text("conf-v2\n")
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    v2 = pins(repo)["cfg"]["resolved"]
    assert v2["sha256"] == transport, "repo edits must not move the transport pin"
    assert v2["repo256"] != v1["repo256"]
    assert v2["tree256"] != v1["tree256"]

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert conf_dest.read_text() == "conf-v2\n"
    assert (sandbox / ".pins/overlay/payload").read_text() == "payload-v1\n"
    assert (repo / LOCK).read_bytes() != locked  # update moved it — apply did not
    locked = (repo / LOCK).read_bytes()

    # cold reconstruct from the new pin; retained roots stay verified
    evict_generation_paths(sandbox)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / LOCK).read_bytes() == locked
    assert conf_dest.read_text() == "conf-v2\n"
    out = grip("store-verify", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr


def test_release_primary_version_pins_and_substitutes(sandbox):
    """The resolved primary version is recorded from the release tag and
    drives {version} substitution in install paths — in update's pin and
    again on every reconstruction."""
    server = Releases()
    try:
        server.serve("v1.0.0", {"tool-v1.0.0/bin/tool": b"tool v1.0.0\n"})
        repo = make_env_repo(sandbox / "env", {"rel": release_module(server.base)})
        tool = sandbox / ".local/bin/tool"

        out = grip("update", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        v1 = pins(repo)["rel"]["resolved"]
        assert v1["version"] == "v1.0.0"
        assert hex64(v1["sha256"]) and hex64(v1["tree256"])
        locked = (repo / LOCK).read_bytes()

        out = grip("apply", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        assert tool.read_text() == "tool v1.0.0\n"
        assert (repo / LOCK).read_bytes() == locked

        server.serve("v1.1.0", {"tool-v1.1.0/bin/tool": b"tool v1.1.0\n"})
        # The old pin must reconstruct without resolving the registry's new
        # latest tag. This observes URL pinning rather than asserting a copy.
        evict_generation_paths(sandbox)
        out = grip("apply", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        assert tool.read_text() == "tool v1.0.0\n"
        assert (repo / LOCK).read_bytes() == locked
        out = grip("update", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        v2 = pins(repo)["rel"]["resolved"]
        assert v2["version"] == "v1.1.0"
        assert v2["sha256"] != v1["sha256"] and v2["tree256"] != v1["tree256"]
        locked = (repo / LOCK).read_bytes()

        out = grip("apply", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        assert tool.read_text() == "tool v1.1.0\n"
        assert (repo / LOCK).read_bytes() == locked

        out = grip("update", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        assert (repo / LOCK).read_bytes() == locked, "repeated update must be stable"
    finally:
        server._server.shutdown()


def test_pinned_release_declaration_is_respected_not_upgraded(sandbox):
    """A version-pinned github_release declaration is not upgraded
    behind the author's back when upstream moves."""
    server = Releases()
    try:
        server.serve("v1.0.0", {"tool-v1.0.0/bin/tool": b"tool v1.0.0\n"})
        repo = make_env_repo(sandbox / "env", {
            "rel": release_module(server.base, pinned="v1.0.0"),
        })
        out = grip("update", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        assert pins(repo)["rel"]["resolved"]["version"] == "v1.0.0"
        locked = (repo / LOCK).read_bytes()

        server.serve("v1.1.0", {"tool-v1.1.0/bin/tool": b"tool v1.1.0\n"})
        out = grip("update", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        assert pins(repo)["rel"]["resolved"]["version"] == "v1.0.0"
        assert (repo / LOCK).read_bytes() == locked
        # the pin resolved through /releases/tags, not /releases/latest
        assert any("releases/tags/v1.0.0" in r for r in server.requests)
    finally:
        server._server.shutdown()


def test_release_tag_metadata_moves_without_payload_change(sandbox):
    """A re-tagged release with identical bytes moves only the version
    pin: sha256 and tree256 stay put, and the next apply stays warm."""
    server = Releases()
    try:
        # both tag spellings nest in the payload: after the version pin
        # moves, the {version}-substituted install path must still
        # resolve during the warm re-deploy
        files = {
            "tool-v1.0.0/bin/tool": b"tool payload\n",
            "tool-1.0.0/bin/tool": b"tool payload\n",
        }
        server.serve("v1.0.0", files)
        repo = make_env_repo(sandbox / "env", {"rel": release_module(server.base)})
        tool = sandbox / ".local/bin/tool"

        out = grip("update", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        out = grip("apply", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        assert tool.read_text() == "tool payload\n"
        v1 = pins(repo)["rel"]["resolved"]

        # same tarball bytes served under the bare tag spelling
        server.serve("1.0.0", files)
        out = grip("update", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        v2 = pins(repo)["rel"]["resolved"]
        assert v2["version"] == "1.0.0"
        assert v2["sha256"] == v1["sha256"], "version metadata is not a payload bump"
        assert v2["tree256"] == v1["tree256"]
        locked = (repo / LOCK).read_bytes()

        out = grip("apply", "--host", "testhost", cwd=repo)
        assert out.returncode == 0, out.stderr
        assert tool.read_text() == "tool payload\n"
        assert (repo / LOCK).read_bytes() == locked
    finally:
        server._server.shutdown()


def test_git_transports_pin_fixed_and_floating_refs(sandbox):
    """git(url) floats to the remote's HEAD and `update` moves it; a
    pinned rev stays exactly where the author put it."""
    remote, rev1 = git_remote(sandbox, {"doc": b"v1\n"})
    subprocess.run(["git", "branch", "moving", rev1], cwd=remote, env=GIT_ENV, check=True)
    repo = make_env_repo(sandbox / "env", {
        "fixed": git_module("fixed", remote, "~/.pins/fixed", rev=rev1),
        "float": git_module("float", remote, "~/.pins/float"),
        "named": git_module("named", remote, "~/.pins/named", rev="moving"),
    })
    float_dest = sandbox / ".pins/float"

    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    first = pins(repo)
    assert first["float"]["resolved"]["version"] == rev1
    assert first["fixed"]["resolved"]["version"] == rev1

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert float_dest.read_text() == "v1\n"

    rev2 = git_commit(remote, sandbox, {"doc": b"v2\n"})
    assert rev2 != rev1
    subprocess.run(["git", "branch", "-f", "moving", rev2], cwd=remote, env=GIT_ENV, check=True)

    # the lock pins HEAD: an apply must not follow upstream
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert float_dest.read_text() == "v1\n"
    evict_generation_paths(sandbox)
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".pins/named").read_text() == "v1\n"

    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    second = pins(repo)
    assert second["float"]["resolved"]["version"] == rev2
    assert second["fixed"]["resolved"] == first["fixed"]["resolved"]

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert float_dest.read_text() == "v2\n"
    assert (sandbox / ".pins/fixed").read_text() == "v1\n"
    assert (sandbox / ".pins/named").read_text() == "v2\n"


def test_failed_update_preserves_the_lock_byte_for_byte(sandbox):
    """The lock is written once, after every selected module succeeds —
    a mid-run failure keeps the previous bytes and leaves no scratch."""
    a = make_tarball(sandbox / "a.tar.gz", {"input": b"a-v1\n"})
    b = make_tarball(sandbox / "b.tar.gz", {"input": b"b-v1\n"})
    repo = make_env_repo(sandbox / "env", {
        "alpha": f"""import {{ fileFetch, module, trackedCopy }} from "@gripsack/core";
export default module("alpha", {{
  fetch: fileFetch({json.dumps(str(a))}), install: {{ input: trackedCopy("~/.pins/a") }},
}});""",
        "beta": f"""import {{ fileFetch, module, trackedCopy }} from "@gripsack/core";
export default module("beta", {{
  fetch: fileFetch({json.dumps(str(b))}), install: {{ input: trackedCopy("~/.pins/b") }},
}});""",
    })
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    locked = (repo / LOCK).read_bytes()
    before = pins(repo)
    tree = repo_tree(repo)

    # alpha would bump; beta's source is gone — the whole pass fails
    make_tarball(sandbox / "a.tar.gz", {"input": b"a-v2\n"})
    b_bytes = b.read_bytes()
    b.unlink()
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode != 0, out.stdout
    assert (repo / LOCK).read_bytes() == locked
    assert repo_tree(repo) == tree, "staging must never scratch inside the repo"

    # the healed rerun bumps alpha only; beta's pin is untouched
    b.write_bytes(b_bytes)
    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    after = pins(repo)
    assert after["alpha"]["resolved"]["sha256"] != before["alpha"]["resolved"]["sha256"]
    assert after["beta"]["resolved"] == before["beta"]["resolved"]


def test_update_subset_scopes_to_modules_and_their_dependencies(sandbox):
    """`grip update <module>` pins the module and its transitive
    dependencies, never the rest of the host's graph."""
    lib = make_tarball(sandbox / "lib.tar.gz", {"input": b"lib-v1\n"})
    app = make_tarball(sandbox / "app.tar.gz", {"input": b"app-v1\n"})
    solo = make_tarball(sandbox / "solo.tar.gz", {"input": b"solo-v1\n"})
    repo = make_env_repo(sandbox / "env", {
        "lib": f"""import {{ fileFetch, module, trackedCopy }} from "@gripsack/core";
export default module("lib", {{
  fetch: fileFetch({json.dumps(str(lib))}), install: {{ input: trackedCopy("~/.pins/lib") }},
}});""",
        "app": f"""import {{ dep, fileFetch, module, trackedCopy }} from "@gripsack/core";
export default module("app", {{
  depends: [dep("lib")],
  fetch: fileFetch({json.dumps(str(app))}), install: {{ input: trackedCopy("~/.pins/app") }},
}});""",
        "solo": f"""import {{ fileFetch, module, trackedCopy }} from "@gripsack/core";
export default module("solo", {{
  fetch: fileFetch({json.dumps(str(solo))}), install: {{ input: trackedCopy("~/.pins/solo") }},
}});""",
    })

    out = grip("update", "app", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert set(pins(repo)) == {"app", "lib"}, "subset scope includes dependencies only"

    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert set(pins(repo)) == {"app", "lib", "solo"}
    locked = (repo / LOCK).read_bytes()
    before = pins(repo)

    # lib's payload moves on disk, but only solo is in scope: the lock
    # must come back byte-identical — lib's pin is not refreshed
    make_tarball(sandbox / "lib.tar.gz", {"input": b"lib-v2\n"})
    out = grip("update", "solo", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (repo / LOCK).read_bytes() == locked
    assert pins(repo)["lib"]["resolved"] == before["lib"]["resolved"]

    # scoping through the dependency DOES refresh lib
    out = grip("update", "app", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    refreshed = pins(repo)
    assert refreshed["lib"]["resolved"]["sha256"] != before["lib"]["resolved"]["sha256"]
    assert refreshed["app"]["resolved"] == before["app"]["resolved"]
    assert refreshed["solo"]["resolved"] == before["solo"]["resolved"]

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert (sandbox / ".pins/lib").read_text() == "lib-v2\n"


def test_update_acquires_recipe_sources_without_executing_recipes(sandbox):
    """Build recipes are NOT executed by update: their pins describe
    source identity, not unknowable build output — and apply must not
    erase or rewrite the finalized pin."""
    payload = make_tarball(sandbox / "payload.tar.gz", {"input": b"src\n"})
    marker = sandbox / "proof-of-build"
    repo = make_env_repo(sandbox / "env", {
        "rec": f"""import {{ fetchStep, fileFetch, installStep, module,
  runStep, trackedCopy }} from "@gripsack/core";
export default module("rec", {{ steps: [
  fetchStep(fileFetch({json.dumps(str(payload))})),
  runStep(["sh", "-c", {json.dumps(f"cp input built && cp input {marker}")}],
    "build", {{ needs: ["fetch"], outputs: ["built"] }}),
  installStep({{ built: trackedCopy("~/.pins/built") }}, "install", {{ needs: ["build"] }}),
] }});""",
    })

    out = grip("update", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert not marker.exists(), "update must never execute recipes"
    pin = pins(repo)["rec"]["resolved"]
    assert pin["sha256"] == hashlib.sha256(payload.read_bytes()).hexdigest()
    assert "tree256" not in pin, "a recipe's unknowable output is not pinnable"
    locked = (repo / LOCK).read_bytes()

    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stderr
    assert marker.exists(), "apply runs the recipe"
    assert (sandbox / ".pins/built").read_text() == "src\n"
    assert (repo / LOCK).read_bytes() == locked, "apply must not rewrite finalized pins"


def test_locked_bottle_reconstructs_without_registry_resolution(sandbox, monkeypatch):
    """A legacy bottle URL/version remains usable after stable moves or goes
    offline. Any accidental registry lookup hits a closed loopback proxy."""
    for name in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
        monkeypatch.setenv(name, "http://127.0.0.1:9")
    monkeypatch.setenv("NO_PROXY", "")
    monkeypatch.setenv("no_proxy", "")
    archive = make_tarball(sandbox / "bottle.tar.gz", {"bin/tool": b"pinned bottle"})
    repo = make_env_repo(sandbox / "bottle-env", {
        "bottle": '''import { module, symlink } from "@gripsack/core";
export default module("bottle", {
  fetch: {kind: "brew", formula: "unreachable-fixture"},
  install: {"bin/tool": symlink("~/.local/bin/bottle")},
});''',
    })
    (repo / "locks").mkdir()
    (repo / LOCK).write_text(json.dumps({"modules": {"bottle": {
        "fetch": {"kind": "brew", "formula": "unreachable-fixture"},
        "resolved": {"url": archive.as_uri(), "version": "1.0.0",
                     "sha256": hashlib.sha256(archive.read_bytes()).hexdigest()},
    }}}))
    result = grip("apply", "--host", "testhost", cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    baseline = (repo / LOCK).read_bytes()
    evict_generation_paths(sandbox)
    result = grip("apply", "--host", "testhost", cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    assert (sandbox / ".local/bin/bottle").read_bytes() == b"pinned bottle"
    assert (repo / LOCK).read_bytes() == baseline
