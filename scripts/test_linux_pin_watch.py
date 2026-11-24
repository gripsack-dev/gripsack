"""Offline metadata and disposable copies of the shipped Linux pin consumers."""
from contextlib import chdir, redirect_stdout
import io
import json
from pathlib import Path
import re
import shutil
import tempfile
import unittest
from unittest.mock import patch

import check_pins as watch
from pin_pr import apply_edits


SOURCE = Path(__file__).resolve().parents[1]
HELPER = "tools/conda-helper/package.py"
CHANNEL = "https://static.rust-lang.org/dist/channel-rust-stable.toml"
REGISTRY = "https://registry-1.docker.io/v2/library/rust/manifests/"
NODE = "https://nodejs.org/dist"
PYPI = "https://pypi.org/pypi/uv"


def bump(pin):
    major_minor, patch = pin.rsplit(".", 1)
    return f"{major_minor}.{int(patch) + 1}"


class LinuxPinWatch(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        for name in ("Dockerfile", HELPER, "proofs/Dockerfile"):
            (self.root / name).parent.mkdir(exist_ok=True, parents=True)
            shutil.copyfile(SOURCE / name, self.root / name)
        self.df = self.root / "Dockerfile"
        self.urls = {}
        self.urls["https://auth.docker.io/token?service=registry.docker.io&scope=repository%3Alibrary/rust%3Apull"] = {"token": "fixture"}
        mock = patch.object(watch.urllib.request, "urlopen", side_effect=self.open_url)
        mock.start()
        self.addCleanup(mock.stop)

    def open_url(self, request, *, timeout):
        url = request if isinstance(request, str) else request.full_url
        if url not in self.urls:
            raise OSError(f"no offline metadata for {url}")
        data = self.urls[url]
        if url.startswith(REGISTRY):
            response = io.BytesIO()
            response.headers = {"Docker-Content-Digest": data}
            return response
        return io.BytesIO((data if isinstance(data, str) else json.dumps(data)).encode())

    def sites(self):
        return watch.PinSites(str(self.root))

    def snapshot(self):
        return {str(p.relative_to(self.root)): p.read_text() for p in self.root.rglob("*") if p.is_file()}

    def assert_proposal(self, builder, expected_lines):
        before = self.snapshot()
        with (chdir(self.root), redirect_stdout(io.StringIO()) as report,
              patch.object(watch, "build_units", side_effect=lambda sites: [builder(sites)]),
              patch.object(watch, "apply_units") as publish):
            self.assertEqual(watch.main(["--dry-run"]), 0)
        publish.assert_not_called()
        self.assertIn("+++ Dockerfile (proposed)", report.getvalue())
        self.assertEqual(before, self.snapshot())

        unit = builder(self.sites())
        self.assertEqual(unit.status, "ready", unit.note)
        apply_edits(unit.edits, str(self.root))
        expected = before.copy()
        for path, changes in expected_lines.items():
            lines = expected[path].splitlines(keepends=True)
            for prefix, replacement in changes.items():
                index, = (i for i, line in enumerate(lines) if line.startswith(prefix))
                lines[index] = replacement + "\n"
            expected[path] = "".join(lines)
        self.assertEqual(self.snapshot(), expected)  # only the named live consumers changed
        again = builder(self.sites())
        self.assertEqual((again.status, again.edits), ("current", []))

    def assert_no_patch(self, builder, status="blocked"):
        unit = builder(self.sites())
        self.assertEqual((unit.status, unit.edits), (status, []), unit.note)

    def rust_metadata(self, version):
        self.urls[CHANNEL] = f'[pkg.rust]\nversion = "{version} (deadbeef 2026-11-21)"\n'
        self.urls[REGISTRY + "alpine"] = self.urls[REGISTRY + version + "-alpine"] = "sha256:" + "a" * 64
        self.urls[REGISTRY + version] = "sha256:" + "b" * 64

    def test_rust_compiler_and_helper_consume_the_same_images(self):
        version = bump(re.search(r"^ENV FUZZ_TOOLCHAIN=(.+)$", self.df.read_text(), re.M)[1])
        self.rust_metadata(version)
        image = "rust:alpine@sha256:" + "a" * 64
        self.assert_proposal(watch.unit_rust, {
            "Dockerfile": {"FROM rust:": f"FROM {image} AS builder",
                           "ARG GNU_RUST_IMAGE=": f"ARG GNU_RUST_IMAGE=rust:{version}@sha256:" + "b" * 64,
                           "ENV FUZZ_TOOLCHAIN=": f"ENV FUZZ_TOOLCHAIN={version}"},
            HELPER: {"COMPILER = ": f'COMPILER = "{version}"', "LINUX_IMAGE = ": f'LINUX_IMAGE = "{image}"'},
        })

    def test_rust_digest_only_refresh_and_divergent_inputs(self):
        pinned = re.search(r"^ENV FUZZ_TOOLCHAIN=(.+)$", self.df.read_text(), re.M)[1]
        self.rust_metadata(pinned)
        image = "rust:alpine@sha256:" + "a" * 64
        self.assert_proposal(watch.unit_rust, {
            "Dockerfile": {"FROM rust:": f"FROM {image} AS builder",
                           "ARG GNU_RUST_IMAGE=": f"ARG GNU_RUST_IMAGE=rust:{pinned}@sha256:" + "b" * 64},
            HELPER: {"LINUX_IMAGE = ": f'LINUX_IMAGE = "{image}"'},
        })
        self.df.write_text(self.df.read_text().replace(f"ENV FUZZ_TOOLCHAIN={pinned}", "ENV FUZZ_TOOLCHAIN=0.0.0"))
        self.assert_no_patch(watch.unit_rust)

    def test_rust_missing_malformed_or_lagging_image_blocks_whole_unit(self):
        self.assert_no_patch(watch.unit_rust, "unknown")
        version = bump(re.search(r"^ENV FUZZ_TOOLCHAIN=(.+)$", self.df.read_text(), re.M)[1])
        for tag, header in ((version, None), (version, "sha256:bad"), (version + "-alpine", "sha256:" + "c" * 64)):
            with self.subTest(tag=tag, header=header):
                self.rust_metadata(version)
                if header is None:
                    del self.urls[REGISTRY + tag]
                else:
                    self.urls[REGISTRY + tag] = header
                self.assert_no_patch(watch.unit_rust)

    def node_metadata(self):
        pinned = self.sites().dockerfile_arg("LOCAL_NODE_VERSION")
        version = bump(pinned)
        self.urls[NODE + "/index.json"] = [{"version": v} for v in
                                          ("v99.0.0", "v" + pinned, "v" + bump(version) + "-rc.1", "v" + version)]
        url = f"{NODE}/v{version}/SHASUMS256.txt"
        return version, url, f"node-v{version}-linux-x64.tar.xz"

    def test_node_updates_the_exact_linux_x64_tarball(self):
        version, url, artifact = self.node_metadata()
        self.urls[url] = "c" * 64 + "  " + artifact.replace("x64", "arm64") + "\n" + "d" * 64 + "  " + artifact
        self.assert_proposal(watch.unit_node, {"Dockerfile": {
            "ARG LOCAL_NODE_VERSION=": f"ARG LOCAL_NODE_VERSION={version}",
            "ARG LOCAL_NODE_SHA256=": "ARG LOCAL_NODE_SHA256=" + "d" * 64,
        }})

    def test_node_refuses_missing_ambiguous_and_wrong_artifact_metadata(self):
        self.assert_no_patch(watch.unit_node, "unknown")
        _, url, artifact = self.node_metadata()
        self.assert_no_patch(watch.unit_node)
        row = "d" * 64 + "  " + artifact + "\n"
        for rows in (row.replace("x64", "arm64"), row * 2, "bad  " + artifact):
            with self.subTest(rows=rows):
                self.urls[url] = rows
                self.assert_no_patch(watch.unit_node)

    def uv_metadata(self):
        version = bump(self.sites().dockerfile_arg("UV_VERSION"))
        self.urls[PYPI + "/json"] = {"info": {"version": version}}
        wheel = {"filename": f"uv-{version}-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
                 "digests": {"sha256": "e" * 64}}
        return version, f"{PYPI}/{version}/json", wheel

    def test_uv_uses_only_the_live_linux_wheel_requirement(self):
        version, url, wheel = self.uv_metadata()
        self.urls[url] = {"urls": [wheel, {"filename": f"uv-{version}-py3-none-manylinux_2_28_x86_64.whl",
                                           "digests": {"sha256": "f" * 64}}]}
        requirement = next(line for line in self.df.read_text().splitlines() if line.startswith("RUN printf 'uv=="))
        # Also retain an inert copy of the *whole* old instruction in a comment.
        self.df.write_text(self.df.read_text() + "\n# " + requirement + "\n")
        self.assert_proposal(watch.unit_uv, {"Dockerfile": {
            "ARG UV_VERSION=": f"ARG UV_VERSION={version}",
            "RUN printf 'uv==": "RUN printf 'uv==%s --hash=sha256:" + "e" * 64 + "\\n' \"$UV_VERSION\" > /tmp/uv.req \\",
        }})

    def test_uv_refuses_missing_ambiguous_and_invalid_wheel_metadata(self):
        self.assert_no_patch(watch.unit_uv, "unknown")
        _, url, wheel = self.uv_metadata()
        self.assert_no_patch(watch.unit_uv)
        for wheels in ([{"filename": wheel["filename"].replace("x86_64", "aarch64")}], [wheel, wheel],
                       [{**wheel, "digests": {"sha256": "bad"}}], [{**wheel, "yanked": True}]):
            with self.subTest(wheels=wheels):
                self.urls[url] = {"urls": wheels}
                self.assert_no_patch(watch.unit_uv)
