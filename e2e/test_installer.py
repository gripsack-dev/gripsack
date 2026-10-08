"""Install the actual shell entrypoint against offline release HTTP fixtures.

The fixed GitHub origins are redirected; curl, checksums, tar and install are
real. Platform-refusal cases control uname's OS result. These tests qualify
selection/preservation, not native grip execution.
"""

from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest
from conftest import make_toolchain_tarball


class InstallerReleases:
    def __init__(self, root: Path):
        self.root = root
        self.tags: list[str] = []
        self.assets: dict[str, tuple[int, bytes]] = {}
        self.truncated_catalog = False
        self.requests: list[str] = []
        machine = platform.machine()
        assert machine in {"x86_64", "amd64", "aarch64", "arm64"}
        arch = "aarch64" if machine in {"aarch64", "arm64"} else "x86_64"
        assert platform.system() == "Linux"
        self.target = f"{arch}-unknown-linux-musl"
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                owner.requests.append(self.path)
                catalog = self.path.startswith("/api/repos/")
                if catalog:
                    status = 200
                    body = json.dumps([
                        {"ref": f"refs/tags/{tag}"} for tag in owner.tags
                    ], indent=2).encode()
                else:
                    status, body = owner.assets.get(self.path, (404, b"not published"))
                self.send_response(status)
                length = len(body) + (20 if catalog and owner.truncated_catalog else 0)
                self.send_header("Content-Length", str(length))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(body)
                self.close_connection = True

            def log_message(self, *args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def asset_path(self, version: str, suffix: str = "") -> str:
        name = f"gripsack-{version}-{self.target}.tar.gz{suffix}"
        return f"/releases/download/core-v{version}/{name}"

    def publish(self, version: str, *, bad_checksum: bool = False) -> bytes:
        payload = f"#!/bin/sh\necho grip {version}\n".encode()
        name = f"gripsack-{version}-{self.target}"
        archive = make_toolchain_tarball(
            self.root / f"{version}.tar.gz", {f"{name}/grip": payload}
        ).read_bytes()
        digest = "0" * 64 if bad_checksum else hashlib.sha256(archive).hexdigest()
        self.assets[self.asset_path(version)] = (200, archive)
        self.assets[self.asset_path(version, ".sha256")] = (
            200, f"{digest}  {name}.tar.gz\n".encode()
        )
        return payload

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


@pytest.fixture
def installer(tmp_path):
    releases = InstallerReleases(tmp_path)
    real_curl = shutil.which("curl")
    assert real_curl, "installer verification requires the real curl client"
    real_uname = shutil.which("uname")
    assert real_uname, "installer verification requires uname"
    tools = tmp_path / "tools"
    tools.mkdir()
    curl = tools / "curl"
    base = f"http://127.0.0.1:{releases.server.server_port}"
    curl.write_text(
        "#!/usr/bin/env python3\n"
        "import os, sys\n"
        "args = []\n"
        "for value in sys.argv[1:]:\n"
        "    if value.startswith('https://api.github.com/'):\n"
        f"        value = {base!r} + '/api' + value.removeprefix('https://api.github.com')\n"
        "    elif value.startswith('https://github.com/gripsack-dev/gripsack/'):\n"
        f"        value = {base!r} + value.removeprefix('https://github.com/gripsack-dev/gripsack')\n"
        "    elif value.startswith(('https://', 'http://')):\n"
        "        raise SystemExit('unexpected network origin in installer fixture')\n"
        "    args.append(value)\n"
        f"os.execv({real_curl!r}, [{real_curl!r}, *args])\n"
    )
    curl.chmod(0o755)
    home = tmp_path / "home"
    home.mkdir()
    destination = home / "bin" / "grip"
    destination.parent.mkdir()
    env = dict(os.environ)
    for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy",
                "GITHUB_TOKEN", "GH_TOKEN"):
        env.pop(key, None)
    env.update(HOME=str(home), CURL_HOME=str(home), GRIPSACK_BIN=str(destination.parent),
               TMPDIR=str(tmp_path), PATH=f"{tools}{os.pathsep}{env['PATH']}",
               NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
    script = Path(__file__).resolve().parent.parent / "install.sh"

    def run(*, os_name: str | None = None):
        uname = tools / "uname"
        if os_name is None:
            uname.unlink(missing_ok=True)
        else:
            uname.write_text(
                "#!/usr/bin/env python3\n"
                "import os, sys\n"
                "if sys.argv[1:] == ['-s']:\n"
                f"    print({os_name!r})\n"
                "else:\n"
                f"    os.execv({real_uname!r}, [{real_uname!r}, *sys.argv[1:]])\n"
            )
            uname.chmod(0o755)
        return subprocess.run(["sh", str(script)], cwd=tmp_path, env=env,
                              capture_output=True, text=True, timeout=30)

    try:
        yield releases, destination, run
    finally:
        releases.close()


def test_installer_skips_unpublished_tags_and_selects_highest_available_version(installer):
    releases, destination, run = installer
    releases.tags = ["core-v2.1.0", "core-v3.0.0", "ts-v99.0.0", "core-v2.10.0"]
    releases.publish("2.1.0")
    expected = releases.publish("2.10.0")
    result = run()
    assert result.returncode == 0, result.stderr
    assert destination.read_bytes() == expected
    assert os.access(destination, os.X_OK)


def test_installer_skips_a_platform_asset_pair_missing_its_archive(installer):
    releases, destination, run = installer
    releases.tags = ["core-v3.0.0", "core-v2.0.0"]
    releases.publish("3.0.0")
    del releases.assets[releases.asset_path("3.0.0")]
    expected = releases.publish("2.0.0")
    result = run()
    assert result.returncode == 0, result.stderr
    assert destination.read_bytes() == expected


def test_installer_does_not_select_prerelease_tags(installer):
    releases, destination, run = installer
    releases.tags = ["core-v9.0.0-rc.1", "core-v2.0.0"]
    releases.publish("9.0.0-rc.1")
    expected = releases.publish("2.0.0")
    result = run()
    assert result.returncode == 0, result.stderr
    assert destination.read_bytes() == expected


def test_installer_rejects_bad_checksum_without_downgrade_or_replacement(installer):
    releases, destination, run = installer
    releases.tags = ["core-v3.0.0", "core-v2.0.0"]
    releases.publish("3.0.0", bad_checksum=True)
    releases.publish("2.0.0")
    original = b"operator's existing executable\n"
    destination.write_bytes(original)
    result = run()
    assert result.returncode != 0
    assert destination.read_bytes() == original


@pytest.mark.parametrize("status", [403, 500])
def test_installer_does_not_treat_server_errors_as_missing_releases(installer, status):
    releases, destination, run = installer
    releases.tags = ["core-v3.0.0", "core-v2.0.0"]
    releases.assets[releases.asset_path("3.0.0", ".sha256")] = (status, b"server refused")
    releases.publish("2.0.0")
    original = b"operator's existing executable\n"
    destination.write_bytes(original)
    result = run()
    assert result.returncode != 0
    assert destination.read_bytes() == original


def test_installer_rejects_a_truncated_tag_response_before_installation(installer):
    releases, destination, run = installer
    releases.tags = ["core-v2.0.0"]
    releases.publish("2.0.0")
    releases.truncated_catalog = True
    original = b"operator's existing executable\n"
    destination.write_bytes(original)
    result = run()
    assert result.returncode != 0
    assert destination.read_bytes() == original


def test_installer_preserves_existing_binary_when_no_target_assets_are_published(installer):
    releases, destination, run = installer
    releases.tags = ["core-v3.0.0", "core-v2.0.0"]
    original = b"operator's existing executable\n"
    destination.write_bytes(original)
    result = run()
    assert result.returncode != 0
    assert destination.read_bytes() == original


def test_installer_refuses_macos_before_network_or_replacement(installer):
    releases, destination, run = installer
    releases.target = releases.target.removesuffix("-unknown-linux-musl") + "-apple-darwin"
    releases.tags = ["core-v0.43.0"]
    releases.publish("0.43.0")
    original = b"operator's existing executable\n"
    destination.write_bytes(original)
    original_mode = destination.stat().st_mode

    result = run(os_name="Darwin")

    assert result.returncode != 0
    assert releases.requests == []
    assert destination.read_bytes() == original
    assert destination.stat().st_mode == original_mode
