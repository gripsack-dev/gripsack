"""A fake GitHub release host on a local port: `releases/latest` redirects to
the newest tag; downloads serve a tarball and its `.sha256` sidecar."""
import gzip
import hashlib
import io
import tarfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TARGET = "x86_64-unknown-linux-musl"


def peer(version: str) -> bytes:
    return f"#!/bin/sh\necho grip {version}\n".encode()


def tarball(version: str, payload: bytes | None = None) -> bytes:
    payload = peer(version) if payload is None else payload
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as z, tarfile.open(fileobj=z, mode="w") as tar:
        info = tarfile.TarInfo(f"gripsack-{version}-{TARGET}/grip")
        info.size, info.mode = len(payload), 0o755
        tar.addfile(info, io.BytesIO(payload))
    return buf.getvalue()


class FakeRelease:
    def __init__(self):
        self.files: dict[str, bytes] = {}
        self.latest: str | None = None
        self.requests: list[str] = []
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                owner.requests.append(self.path)
                if self.path == "/releases/latest" and owner.latest:
                    self.send_response(302)
                    self.send_header("Location", f"/releases/tag/core-v{owner.latest}")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                body = b"release page" if self.path.startswith("/releases/tag/") else owner.files.get(self.path)
                self.send_response(200 if body is not None else 404)
                self.send_header("Content-Length", str(len(body or b"")))
                self.end_headers()
                self.wfile.write(body or b"")

            def log_message(self, *_):
                pass

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self._server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self._server.server_address[1]}"

    def publish(self, version: str, payload: bytes | None = None, *, latest: bool = True,
                corrupt_checksum: bool = False):
        name = f"gripsack-{version}-{TARGET}.tar.gz"
        data = tarball(version, payload)
        digest = "0" * 64 if corrupt_checksum else hashlib.sha256(data).hexdigest()
        base = f"/releases/download/core-v{version}/{name}"
        self.files[base] = data
        self.files[base + ".sha256"] = f"{digest}  {name}\n".encode()
        if latest:
            self.latest = version

    def close(self):
        self._server.shutdown()
        self._server.server_close()
