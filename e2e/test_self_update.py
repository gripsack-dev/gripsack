"""0042 F: durable concurrent self-update against the REAL CLI.

The installed executable is a copy of the real `grip`; updates come
from a loopback GitHub-releases fixture serving sha-verified tarballs
whose `grip` is a tiny executable version peer (it must answer
`--version` with exactly `grip X.Y.Z`). Two processes start from the
same old inode; API/download barriers force one updater to publish
while the other waits on the per-executable coordination lock, proving
the under-lock re-check: no downgrade, no second overwrite.

Publication faults are driven through the real operation trace
(GRIPSACK_FS_TRACE/GRIPSACK_FS_CUT, debug builds only): a cut before
FilePublish leaves the old bytes, a cut after FilePublish or at DirSync
leaves the NEW bytes and reports a durability failure — never a
rollback. No network beyond loopback; nothing writes outside the
sandbox (TMPDIR included).
"""

from __future__ import annotations

import gzip
import hashlib
import io
import json
import os
import shutil
import stat
import subprocess
import tarfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest
from conftest import GRIP

TRIPLES = (
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)


def version_peer(version: str) -> bytes:
    """The tiny executable version peer: one line, exactly the contract
    payload::version parses."""
    return f"#!/bin/sh\necho grip {version}\n".encode()


def release_tarball(version: str) -> bytes:
    """The real release layout: one executable `grip` nested in a
    single directory (release naming), so selection is unambiguous."""
    payload = version_peer(version)
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as tar:
            info = tarfile.TarInfo(f"gripsack-{version}-x86_64-unknown-linux-musl/grip")
            info.size = len(payload)
            info.mode = 0o755
            tar.addfile(info, io.BytesIO(payload))
    return buf.getvalue()


def custom_tarball(members: list[tarfile.TarInfo], contents: list[bytes]) -> bytes:
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as tar:
            for info, content in zip(members, contents):
                info.size = len(content)
                tar.addfile(info, io.BytesIO(content))
    return buf.getvalue()


class SelfUpdateReleases:
    """Loopback GitHub-releases API behind GRIPSACK_UPDATE_API. Serves
    the newest core-v* release (ts-v tags ship alongside and must not
    win), the platform tarball and its mandatory sha256 sidecar. Every
    served version stays downloadable after a newer one becomes latest.

    `hold_tarball()` parks the next tarball response inside the server
    thread AFTER its payload was captured — the client holds the
    self-update lock mid-download until `release_tarball()`."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self.releases: dict[str, dict[str, bytes]] = {}
        self.latest = ""
        self.requests: list[str] = []
        self._holding = False
        self.tarball_entered = threading.Event()
        self.tarball_release = threading.Event()
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        threading.Thread(target=self._server.serve_forever, daemon=True).start()

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self._server.server_port}"

    def serve(self, version: str, tarball: bytes) -> None:
        sha = hashlib.sha256(tarball).hexdigest()
        base = self.base
        assets = []
        for triple in TRIPLES:
            assets.append({
                "name": f"gripsack-{version}-{triple}.tar.gz",
                "browser_download_url": f"{base}/dl/{version}/t.tar.gz",
                "url": f"{base}/api/{version}/{triple}",
            })
            assets.append({
                "name": f"gripsack-{version}-{triple}.tar.gz.sha256",
                "browser_download_url": f"{base}/dl/{version}/t.sha256",
                "url": f"{base}/api/{version}/{triple}.sha256",
            })
        listing = json.dumps([
            {"tag_name": "ts-v0.0.0", "assets": []},
            {"tag_name": f"core-v{version}", "assets": assets},
        ]).encode()
        with self._lock:
            self.releases[version] = {
                "listing": listing,
                "tarball": tarball,
                "sidecar": f"{sha}  gripsack.tar.gz\n".encode(),
            }
            self.latest = version

    def hold_tarball(self) -> None:
        self.tarball_release.clear()
        self.tarball_entered.clear()
        self._holding = True

    def release_tarball(self) -> None:
        self._holding = False
        self.tarball_release.set()

    def count(self, suffix: str) -> int:
        with self._lock:
            return sum(1 for r in self.requests if r.split("?")[0].endswith(suffix))

    def _handler(self):
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                path = self.path.split("?")[0]
                with fixture._lock:
                    fixture.requests.append(self.path)
                    latest = fixture.latest
                    served = dict(fixture.releases)
                    hold = fixture._holding
                if path.startswith("/repos/"):
                    body = served[latest]["listing"]
                elif path.endswith(".sha256"):
                    body = served[latest]["sidecar"]
                elif path.endswith(".tar.gz"):
                    body = served[latest]["tarball"]
                    if hold:
                        # the payload is already captured; the client is
                        # now parked holding the self-update lock
                        fixture.tarball_entered.set()
                        if not fixture.tarball_release.wait(180):
                            self.send_error(500, "barrier timeout")
                            return
                else:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        return Handler


def install_real_cli(sandbox: Path) -> Path:
    """The copied REAL CLI — both concurrent updaters start from this
    same inode."""
    exe = sandbox / "bin" / "grip"
    exe.parent.mkdir(parents=True)
    shutil.copyfile(GRIP.resolve(), exe)
    exe.chmod(0o755)
    return exe


def cli_env(sandbox: Path, server: SelfUpdateReleases, **extra: str) -> dict:
    (sandbox / "tmp").mkdir(exist_ok=True)
    env = dict(os.environ)
    env.update({
        "HOME": str(sandbox),
        "GRIPSACK_HOME": str(sandbox / ".local/share/gripsack"),
        "GRIPSACK_UPDATE_API": server.base,
        "TMPDIR": str(sandbox / "tmp"),
    })
    env.update(extra)
    return env


def restore_real_cli(exe: Path) -> bytes:
    real = GRIP.resolve().read_bytes()
    shutil.copyfile(GRIP.resolve(), exe)
    exe.chmod(0o755)
    return real


def exe_state(exe: Path) -> tuple[bytes, int, int]:
    info = exe.stat()
    return exe.read_bytes(), info.st_ino, info.st_mtime_ns


def wait_for(predicate, timeout: float = 60.0, what: str = "condition") -> None:
    import time

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {what}")


# --------------------------------------------------------------------------
# concurrent updaters starting from one old inode


def test_waiting_older_updater_never_downgrades_or_overwrites(sandbox):
    """The newest installation wins BEFORE an older waiting updater
    acquires the lock: P1 (barriered on its download) publishes 3.0.0;
    P2 resolved 2.0.0 earlier and is parked on the lock. P2's under-lock
    re-probe must see 3.0.0, report current, and write nothing."""
    server = SelfUpdateReleases()
    try:
        server.serve("3.0.0", release_tarball("3.0.0"))
        exe = install_real_cli(sandbox)
        original = exe.read_bytes()
        env = cli_env(sandbox, server)

        server.hold_tarball()
        p1 = subprocess.Popen([str(exe), "self-update"], env=env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        assert server.tarball_entered.wait(90), "P1 never reached its held download"
        assert exe.read_bytes() == original, "nothing may publish while P1 waits"

        # the newest release is now already installed-in-flight; a later
        # resolver sees an OLDER latest
        server.serve("2.0.0", release_tarball("2.0.0"))
        p2 = subprocess.Popen([str(exe), "self-update"], env=env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        wait_for(lambda: server.count(".sha256") >= 2, what="P2's resolution")
        # P2 has resolved and can only be parked on the coordination lock
        assert p2.poll() is None, f"P2 finished early: {p2.poll()}"
        assert exe.read_bytes() == original, "P2 must not publish while P1 holds the lock"

        server.release_tarball()
        out1, err1 = p1.communicate(timeout=120)
        assert p1.returncode == 0, err1
        assert "3.0.0" in out1 and "updated" in out1, out1

        published = exe_state(exe)
        assert published[0] == version_peer("3.0.0"), "P1 published the newer release"
        out2, err2 = p2.communicate(timeout=120)
        assert p2.returncode == 0, err2
        assert exe_state(exe) == published, "P2 downgraded or overwrote the executable"
        probe = subprocess.run([str(exe), "--version"], capture_output=True, text=True)
        assert probe.stdout.strip() == "grip 3.0.0"

        # the coordination file is never unlinked — waiters keep locking
        # the same inode across publications
        flocks = list((sandbox / "bin").glob(".grip-self-update-*.flock"))
        assert len(flocks) == 1 and flocks[0].is_file()
    finally:
        server.release_tarball()
        server._server.shutdown()


def publication_cuts(trace: list[tuple[int, str, str, str]], exe: Path) -> dict[str, int]:
    """Ordinals of the executable swap's durability boundaries. The
    streamed write traces its DESTINATION name — the bare `grip` the
    installation pins — and its DirSync traces the parent rel path."""
    swap = [row for row in trace if row[3] == exe.name and row[2] in
            {"Write", "Mode", "FileSync", "FilePublish"}]
    assert swap, f"no publication boundaries for {exe.name} in the trace"
    publish = [row for row in swap if row[2] == "FilePublish"]
    assert [row[1] for row in publish] == ["Before", "After"], publish
    index = trace.index(publish[1])
    dirsync = [row for row in trace[index + 1:] if row[2] == "DirSync"][:2]
    assert [row[1] for row in dirsync] == ["Before", "After"], dirsync
    return {
        "prepublish": publish[0][0],
        "postpublish": publish[1][0],
        "predirsync": dirsync[0][0],
        "postdirsync": dirsync[1][0],
    }


def test_waiting_newer_updater_wins_after_the_reprobe(sandbox):
    """The other direction: P1 publishes the older 2.0.0 first; P2
    (which resolved 3.0.0 before P1 published) re-probes under the lock,
    still sees room to move, and publishes 3.0.0 — exactly two
    publications, ending on the newest."""
    server = SelfUpdateReleases()
    try:
        server.serve("2.0.0", release_tarball("2.0.0"))
        exe = install_real_cli(sandbox)
        original = exe.read_bytes()
        env = cli_env(sandbox, server)

        server.hold_tarball()
        p1 = subprocess.Popen([str(exe), "self-update"], env=env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        assert server.tarball_entered.wait(90), "P1 never reached its held download"

        server.serve("3.0.0", release_tarball("3.0.0"))
        p2 = subprocess.Popen([str(exe), "self-update"], env=env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        wait_for(lambda: server.count(".sha256") >= 2, what="P2's resolution")
        assert p2.poll() is None, f"P2 finished early: {p2.poll()}"
        assert exe.read_bytes() == original

        server.release_tarball()
        out1, err1 = p1.communicate(timeout=120)
        assert p1.returncode == 0, err1
        first = exe_state(exe)
        assert first[0] == version_peer("2.0.0")

        out2, err2 = p2.communicate(timeout=120)
        assert p2.returncode == 0, err2
        second = exe_state(exe)
        assert second[0] == version_peer("3.0.0")
        assert second[1] != first[1], "the second publication must replace the file"
        probe = subprocess.run([str(exe), "--version"], capture_output=True, text=True)
        assert probe.stdout.strip() == "grip 3.0.0"
    finally:
        server.release_tarball()
        server._server.shutdown()


# --------------------------------------------------------------------------
# publication faults, driven through the real operation trace


def parse_trace(path: Path) -> list[tuple[int, str, str, str]]:
    rows = []
    for line in path.read_text().splitlines():
        ordinal, edge, boundary, traced = line.split("\t", 3)
        rows.append((int(ordinal), edge, boundary, traced))
    return rows


def publication_cuts(trace: list[tuple[int, str, str, str]], exe: Path) -> dict[str, int]:
    """Ordinals of the executable swap's durability boundaries."""
    swap = [row for row in trace if row[3] == json.dumps(exe.name)]
    publish = [row for row in swap if row[2] == "FilePublish"]
    assert [row[1] for row in publish] == ["Before", "After"], publish
    index = trace.index(publish[1])
    after = trace[index + 1:]
    dirsync = [row for row in after if row[2] == "DirSync"][:2]
    assert [row[1] for row in dirsync] == ["Before", "After"], dirsync
    return {
        "prepublish": publish[0][0],
        "postpublish": publish[1][0],
        "predirsync": dirsync[0][0],
        "postdirsync": dirsync[1][0],
    }


def test_publication_faults_keep_the_commit_point_honest(sandbox):
    """Before FilePublish: old bytes survive. After FilePublish and at
    either DirSync edge: the NEW bytes are installed, the command
    reports a durability failure, and nothing rolls back."""
    server = SelfUpdateReleases()
    try:
        server.serve("9.9.9", release_tarball("9.9.9"))
        exe = install_real_cli(sandbox)
        real = restore_real_cli(exe)

        states = sandbox / "states"
        states.mkdir()
        trace = sandbox / "trace-baseline.tsv"
        env = cli_env(sandbox, server, GRIPSACK_HOME=str(states / "baseline"),
                      GRIPSACK_FS_TRACE=str(trace))
        done = subprocess.run([str(exe), "self-update"], env=env,
                              capture_output=True, text=True, timeout=120)
        assert done.returncode == 0, done.stderr
        assert exe.read_bytes() == version_peer("9.9.9"), done.stdout
        cuts = publication_cuts(parse_trace(trace), exe)
        assert cuts["prepublish"] < cuts["postpublish"] < cuts["predirsync"] \
            < cuts["postdirsync"], cuts

        cases = [
            ("prepublish", False),   # nothing committed: old bytes stay
            ("postpublish", True),   # rename done: new bytes, durability unknown
            ("predirsync", True),
            ("postdirsync", True),
        ]
        for name, committed in cases:
            restore_real_cli(exe)
            trace = sandbox / f"trace-{name}.tsv"
            env = cli_env(sandbox, server, GRIPSACK_HOME=str(states / name),
                          GRIPSACK_FS_TRACE=str(trace),
                          GRIPSACK_FS_CUT=str(cuts[name]))
            done = subprocess.run([str(exe), "self-update"], env=env,
                                  capture_output=True, text=True, timeout=120)
            assert done.returncode != 0, f"{name}: {done.stdout}"
            rows = parse_trace(trace)
            cut_row = next((r for r in rows if r[0] == cuts[name]), None)
            assert cut_row is not None, f"{name}: the cut ordinal never ran"
            boundary = "FilePublish" if name in {"prepublish", "postpublish"} else "DirSync"
            assert cut_row[2] == boundary, f"{name}: cut hit {cut_row}"
            assert cut_row[1] == ("Before" if name.startswith("pre") else "After")
            if committed:
                assert exe.read_bytes() == version_peer("9.9.9"), (
                    f"{name}: the committed publication must stay"
                )
                mode = stat.S_IMODE(exe.stat().st_mode)
                assert mode & 0o111, f"{name}: installed executable lost its mode"
            else:
                assert exe.read_bytes() == real, f"{name}: old bytes were disturbed"
    finally:
        server._server.shutdown()


# --------------------------------------------------------------------------
# strict payload selection

@pytest.mark.parametrize("kind", ["ambiguous", "symlink"])
def test_strict_payload_selection_defends_the_install(sandbox, kind):
    """A tarball with two regular `grip` binaries is ambiguous; a
    symlink `grip` is not a regular executable. Both fail closed with
    the old bytes untouched."""
    server = SelfUpdateReleases()
    try:
        if kind == "ambiguous":
            first = tarfile.TarInfo("a/grip")
            second = tarfile.TarInfo("b/grip")
            first.mode = second.mode = 0o755
            payload = version_peer("9.9.9")
            tarball = custom_tarball([first, second], [payload, payload])
        else:
            link = tarfile.TarInfo("grip")
            link.type = tarfile.SYMTYPE
            link.linkname = "elsewhere"
            tarball = custom_tarball([link], [b""])
        server.serve("9.9.9", tarball)
        exe = install_real_cli(sandbox)
        original = exe.read_bytes()
        env = cli_env(sandbox, server)
        done = subprocess.run([str(exe), "self-update"], env=env,
                              capture_output=True, text=True, timeout=120)
        assert done.returncode != 0, done.stdout
        assert exe.read_bytes() == original, "a rejected payload must not install"
        probe = subprocess.run([str(exe), "--version"], capture_output=True, text=True)
        reference = subprocess.run([str(GRIP.resolve()), "--version"],
                                   capture_output=True, text=True)
        assert probe.stdout == reference.stdout, "the old executable must still run"
    finally:
        server._server.shutdown()
