"""plan/0059 step 1: every documented workspace runs on a stock glibc host.

Each complete workspace in the website docs and the repository READMEs is an
`examples/workspaces/*` directory. A README binds its code block to one with an
`<!-- example: examples/workspaces/<name>/gripsack.ts -->` line, checked here;
the website build checks its own pages the same way.

The e2e image is Debian (glibc), so these runs are the stock-glibc case. They
stay offline: dotfile and file-fetch examples evaluate, approve, update, plan
and apply; the release-archive example resolves against a local GitHub release
fixture. Conda/Pixi examples need conda-forge and the pinned Conda helper, and
the Docker example needs an Engine, so those evaluate, approve, check and plan
only (NETWORK below records which and why).
"""
from __future__ import annotations

import io
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import threading

import pytest

from conftest import grip

ROOT = Path(__file__).resolve().parent.parent
EXAMPLES = ROOT / "examples" / "workspaces"
READMES = [ROOT / "README.md", ROOT / "typescript" / "README.md"]
MARKER = re.compile(r"^<!-- example: (examples/workspaces/[^ ]+/gripsack\.ts) -->\n```(?:ts|typescript)\n(.*?)^```$", re.M | re.S)

# Examples that deploy offline: profile → {destination: expected text or None for "exists"}.
APPLY = {
    "01-dotfiles": ("dotfiles", {".config/editor/config.toml": 'theme = "dark"\n'}),
    "02-native-tool": ("personal", {".config/formatter/config.toml": "indent = 2\n"}),
    "05-first-workspace": ("dotfiles", {".config/editor/config.toml": 'theme = "dark"\n'}),
    "09-dotfile-tree": ("dotfiles", {
        ".config/editor/config.toml": 'theme = "dark"\nline-numbers = true\n',
        ".local/bin/editor-theme": "#!/bin/sh\nprintf '%s\\n' 'dark'\n",
    }),
    "12-file-check": ("dotfiles", {".config/shell/settings.sh": 'export EDITOR=vi\nalias ll="ls -l"\n'}),
    "14-managed-block": ("dotfiles", {".profile": None}),
    "15-symlink-dotfile": ("dotfiles", {".config/editor/config.toml": 'theme = "dark"\n'}),
}
# Run in full by test_release_archive_example_runs_on_glibc against a fixture.
RELEASE = "06-release-archive"
# Examples whose realization needs something the offline gate does not have.
# They evaluate, are approved, pass `check` and plan the listed profiles.
NETWORK = {
    "04-scheduled-task": (("personal",), "applying registers a systemd user timer"),
    "07-conda-launchers": (("personal",), "conda-forge solve + pinned Conda helper"),
    "08-pixi-import": ((), "conda-forge archives + pinned Conda helper"),
    "10-docker-service": ((), "Docker Engine and a registry pull"),
    "11-bash-task": ((), "conda-forge solve + pinned Conda helper"),
    "13-scheduled-bash-task": (("personal",), "conda-forge solve + pinned Conda helper"),
    "16-conda-python": ((), "conda-forge solve + pinned Conda helper"),
}


def copy_example(sandbox: Path, name: str) -> Path:
    repo = sandbox / "repos" / name
    shutil.copytree(EXAMPLES / name, repo)
    return repo


def ok(result: subprocess.CompletedProcess) -> subprocess.CompletedProcess:
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def test_every_example_is_gated():
    names = {path.name for path in EXAMPLES.iterdir() if (path / "gripsack.ts").is_file()}
    gated = set(APPLY) | {RELEASE} | set(NETWORK)
    assert names == gated, f"ungated: {sorted(names - gated)}; missing: {sorted(gated - names)}"


@pytest.mark.parametrize("readme", READMES, ids=lambda path: str(path.relative_to(ROOT)))
def test_readme_workspaces_are_the_example_files(readme):
    text = readme.read_text()
    bound = MARKER.findall(text)
    for path, body in bound:
        assert (ROOT / path).read_text() == body, f"{readme.name} drifted from {path}"
    # Every complete workspace block (SDK import plus default export) is bound.
    blocks = re.findall(r"^```(?:ts|typescript)\n(.*?)^```$", text, re.M | re.S)
    complete = [block for block in blocks if "export default" in block and "@gripsack/core" in block]
    assert len(complete) == len(bound), f"{readme.name} has an unbound complete workspace"


@pytest.mark.parametrize("name", sorted(APPLY))
def test_offline_example_applies(sandbox, name):
    profile, expected = APPLY[name]
    repo = copy_example(sandbox, name)
    ok(grip("check", cwd=repo))
    ok(grip("update", cwd=repo))
    ok(grip("plan", profile, cwd=repo))
    applied = ok(grip("apply", profile, cwd=repo))
    assert "generation 1 active" in applied.stdout
    for destination, content in expected.items():
        path = sandbox / destination
        assert path.exists(), destination
        if content is not None:
            assert path.read_text() == content, destination


@pytest.mark.parametrize("name", sorted(NETWORK))
def test_network_example_evaluates_and_plans_offline(sandbox, name):
    profiles, _reason = NETWORK[name]
    repo = copy_example(sandbox, name)
    checked = json.loads(ok(grip("check", "--json", cwd=repo)).stdout)
    assert checked["ok"] is True and checked["diagnostics"] == []
    for profile in profiles:
        ok(grip("plan", profile, cwd=repo))
    assert not (sandbox / ".local/share/gripsack/current").exists()


# --- the release archive, served by a local GitHub release fixture ---------

RG = r'''
#include <stdio.h>
int main(void) { puts("ripgrep 15.2.0 (static fixture)"); return 0; }
'''


class GithubRelease:
    """`/api/v3/repos/<repo>/releases/latest` plus the asset download."""

    def __init__(self, repo: str, tag: str, asset: str, archive: bytes):
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path.startswith(f"/api/v3/repos/{repo}/releases"):
                    body = json.dumps({"tag_name": tag, "assets": [{
                        "name": asset,
                        "browser_download_url": f"{owner.base}/downloads/{asset}",
                        "url": f"{owner.base}/api/v3/assets/{asset}",
                    }]}).encode()
                elif self.path.endswith(f"/{asset}"):
                    body = archive
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

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.server.server_port}"

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def compiled(sandbox: Path, *flags: str) -> bytes:
    source = sandbox / "rg.c"
    source.write_text(RG)
    binary = sandbox / "rg"
    subprocess.run(["cc", *flags, "-O2", "-o", binary, source], check=True)
    return binary.read_bytes()


def release_archive(root: str, binary: bytes) -> bytes:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        for name, data, mode in [(f"{root}/rg", binary, 0o755), (f"{root}/README.md", b"fixture\n", 0o644)]:
            info = tarfile.TarInfo(name)
            info.mode, info.size = mode, len(data)
            archive.addfile(info, io.BytesIO(data))
    return buffer.getvalue()


def release_workspace(sandbox: Path, binary: bytes, *, abi: str | None = None):
    """The documented example verbatim, pointed at the fixture by `base_url`."""
    root = "ripgrep-15.2.0-x86_64-unknown-linux-musl"
    server = GithubRelease("BurntSushi/ripgrep", "15.2.0", f"{root}.tar.gz", release_archive(root, binary))
    repo = copy_example(sandbox, RELEASE)
    source = (repo / "gripsack.ts").read_text()
    anchor = '    repo: "BurntSushi/ripgrep",\n'
    assert anchor in source
    source = source.replace(anchor, anchor + f"    base_url: {json.dumps(server.base)},\n")
    if abi is not None:
        declared = 'targetPlatform({ os: "linux", arch: "x86_64" })'
        assert declared in source
        source = source.replace(declared, f'targetPlatform({{ os: "linux", arch: "x86_64", abi: "{abi}" }})')
    (repo / "gripsack.ts").write_text(source)
    return server, repo


native = pytest.mark.skipif(
    platform.system() != "Linux" or platform.machine() != "x86_64"
    or platform.libc_ver()[0] != "glibc",
    reason="the stock glibc host case",
)


@native
@pytest.mark.parametrize("abi", [None, "musl", "gnu"], ids=["inferred", "declared-musl", "declared-gnu"])
def test_release_archive_example_runs_on_glibc(sandbox, abi):
    """A static binary runs on any host; `abi: "musl"` (the docs before 0059)
    on a glibc host used to refuse with E128 at the environment."""
    server, repo = release_workspace(sandbox, compiled(sandbox, "-static"), abi=abi)
    try:
        ok(grip("update", cwd=repo))
        ok(grip("check", cwd=repo))
        ok(grip("build", "ripgrep", cwd=repo))
        ran = ok(grip("run", "--env", "search", "--", "rg", "--version", cwd=repo))
        assert ran.stdout == "ripgrep 15.2.0 (static fixture)\n"
    finally:
        server.close()


@native
def test_declared_abi_is_checked_against_the_loader(sandbox):
    """A declared ABI is an assertion about the bytes: `musl` on a binary whose
    PT_INTERP is the glibc loader is refused before any library lookup."""
    server, repo = release_workspace(sandbox, compiled(sandbox), abi="musl")
    try:
        ok(grip("update", cwd=repo))
        refused = grip("run", "--env", "search", "--", "rg", "--version", cwd=repo)
        assert refused.returncode != 0
        assert "E128" in refused.stderr and "contradicts" in refused.stderr, refused.stderr
    finally:
        server.close()
