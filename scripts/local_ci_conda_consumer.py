"""Controlled real Conda archives through the installed core and pinned helper.

The archive layout follows workspace/conda/validation_tests.rs; TLS transport
uses the existing e2e HttpFixture. No helper protocol or native validator is
mocked: update imports Pixi, and build invokes the real patched materializer
and the core's independent archive/tree/receipt admission.
"""
from __future__ import annotations

import bz2
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import zipfile

from examples_check.fixtures import install_sdk

PLACEHOLDER = "/opt/anaconda1anaconda2anaconda3"


def tar_bytes(files):
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        for name, data in files.items():
            entry = tarfile.TarInfo(name)
            entry.size = len(data)
            entry.mode = 0o644
            entry.mtime = 0
            archive.addfile(entry, io.BytesIO(data))
    return buffer.getvalue()


def package_archive(consumer, base, extension):
    payload = (PLACEHOLDER + "/retained-value\n").encode()
    paths = {"paths_version": 1, "paths": [{
        "_path": "share/value", "path_type": "hardlink",
        "sha256": hashlib.sha256(payload).hexdigest(), "size_in_bytes": len(payload),
        "prefix_placeholder": PLACEHOLDER, "file_mode": "text",
    }]}
    info = {
        "info/index.json": json.dumps({"name": "fixture", "version": "1.0", "build": "0",
            "build_number": 0, "subdir": "linux-64", "depends": []}).encode(),
        "info/paths.json": json.dumps(paths).encode(),
        "info/files": b"share/value\n",
        "info/has_prefix": f'{PLACEHOLDER} text share/value\n'.encode(),
    }
    target = base / f"fixture-1.0-0{extension}"
    if extension == ".tar.bz2":
        target.write_bytes(bz2.compress(tar_bytes({**info, "share/value": payload})))
    else:
        with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_STORED) as archive:
            archive.writestr("metadata.json", '{"conda_pkg_format_version":2}')
            for name, files in (("info", info), ("pkg", {"share/value": payload})):
                uncompressed = base / f"{name}-fixture.tar"
                uncompressed.write_bytes(tar_bytes(files))
                compressed = uncompressed.with_suffix(".tar.zst")
                consumer.run(["zstd", "-q", "-1", "--no-progress", uncompressed, "-o", compressed])
                archive.write(compressed, f"{name}-fixture.tar.zst")
    return target


def require(condition, detail):
    if not condition:
        raise RuntimeError(detail)


def isolated_env(consumer, base):
    home = base / "home"
    home.mkdir(mode=0o700, parents=True)
    return dict(consumer.env, HOME=str(home), GRIPSACK_HOME=str(home / ".gs"))


def refused_build(consumer, repo, env, expected_detail):
    store = Path(env["GRIPSACK_HOME"]) / "store"
    prior = {str(path): path.read_bytes() for path in store.glob("**/share/value")}
    result = json.loads(consumer.run([consumer.grip, "build", "data", "--json"], cwd=repo, env=env, expected=1))
    require(result["ok"] is False, "negative archive build returned success")
    require(any(item["code"] == "E301" and expected_detail in item["message"]
                for item in result["diagnostics"]),
            f"negative build did not reach attributed E301 ({expected_detail}): {result}")
    require({str(path): path.read_bytes() for path in store.glob("**/share/value")} == prior,
            "negative build published or changed a materialized prefix")


def consume_archives(consumer):
    root = Path(__file__).resolve().parents[1]
    sys.path.insert(0, str(root / "e2e"))
    from test_http_policy import HttpFixture
    from local_artifact_consumers import sha, snapshot

    outcomes = []
    consumer.receipt["conda_archive_consumers"] = outcomes
    for extension in (".conda", ".tar.bz2"):
        base = consumer.work / ("conda-v2" if extension == ".conda" else "conda-bzip2")
        base.mkdir(mode=0o700)
        archive = package_archive(consumer, base, extension)
        digest = sha(archive)
        server = HttpFixture(tls=True)
        server.archive = archive.read_bytes()
        closed = False
        try:
            repo = base / "repo"
            repo.mkdir()
            install_sdk(repo, consumer.sdk)
            env = isolated_env(consumer, base)
            # The actual CA is supplied to the core fetch context; no TLS bypass.
            (repo / "env.toml").write_text('[env]\nname = "local-conda-consumer"\n\n[eval]\n'
                + f'env = {{ SSL_CERT_FILE = {json.dumps(str(server.cert))} }}\n')
            channel = server.base + "/channel"
            url = f"{channel}/linux-64/{archive.name}"
            manifest = f"[workspace]\nname='fixture'\nchannels=['{channel}']\nplatforms=['linux-64']\n[dependencies]\nfixture='==1.0'\n"
            pixi_lock = ("version: 6\nenvironments:\n  default:\n    channels:\n"
                f"    - url: {channel}/\n    packages:\n      linux-64:\n      - conda: {url}\n"
                f"packages:\n- conda: {url}\n  sha256: {digest}\n  size: {archive.stat().st_size}\n"
                "  depends: []\n  constrains: []\n")
            (repo / "pixi.toml").write_text(manifest)
            (repo / "pixi.lock").write_text(pixi_lock)
            (repo / "gripsack.ts").write_text('''import {
  defineWorkspace, workspace, pkg, provider, pixi, inputFile,
} from "@gripsack/core";
export default defineWorkspace(() => workspace({
  inputs: [inputFile("manifest", "pixi.toml"), inputFile("lock", "pixi.lock")],
  outputs: [pkg("data", {
    producer: provider(pixi.fromLock({manifest:"manifest",lock:"lock",environment:"default"})),
    commands: {}, target:{os:"linux",arch:"x86_64",abi:"gnu"},
    layout:{kind:"prefix_materialized"},
  })],
}));
''')
            consumer.approve(repo, env)
            consumer.run([consumer.grip, "update"], cwd=repo, env=env)
            consumer.approve(repo, env)
            lock = repo / "gripsack.lock"
            frozen = lock.read_bytes()
            parsed = json.loads(frozen)
            records = [record for resolution in parsed["resolutions"].values()
                       for pin in resolution["pins"] for record in pin["conda"]["packages"]]
            require(len(records) == 1 and records[0]["sha256"] == digest and records[0]["url"] == url,
                    "Pixi import changed immutable archive identity")
            consumer.run([consumer.grip, "check"], cwd=repo, env=env)
            result = json.loads(consumer.run([consumer.grip, "build", "data", "--json"], cwd=repo, env=env))
            require([(output["name"], output["kind"]) for output in result["outputs"]] == [("data", "package")],
                    "archive build did not return the requested package")
            prefix = Path(result["outputs"][0]["path"])
            require((prefix / "share/value").read_text() == f"{prefix}/retained-value\n", "real helper prefix relocation differs")
            original = snapshot(prefix)
            require(server.requests and set(server.requests) == {f"/channel/linux-64/{archive.name}"},
                    f"frozen Pixi import unexpectedly queried repodata: {server.requests}")

            # A corrupt private mirror must not bypass the core's compiled helper pin.
            mirror = base / "corrupt-helper-mirror"
            mirror.mkdir()
            (mirror / consumer.helper.name).write_bytes(b"not the candidate helper\n")
            wrong_helper = isolated_env(consumer, base / "wrong-helper")
            wrong_helper["GRIPSACK_CONDA_HELPER_MIRROR"] = str(mirror)
            consumer.approve(repo, wrong_helper)
            refused_build(consumer, repo, wrong_helper, "sha256 mismatch")

            # Fresh state cannot acquire a changed archive under the frozen hash.
            bad_archive = isolated_env(consumer, base / "wrong-archive")
            consumer.approve(repo, bad_archive)
            server.archive = b"changed archive bytes\n"
            refused_build(consumer, repo, bad_archive, "archive download: sha256 mismatch")
            server.archive = archive.read_bytes()

            # Source approval and frozen input identity are distinct gates.
            (repo / "pixi.toml").write_text(manifest + "\n# changed reviewed input\n")
            consumer.run([consumer.grip, "check"], cwd=repo, env=env, expected=1)
            consumer.approve(repo, env)
            refused_build(consumer, repo, env, "changed after its lock")
            require(lock.read_bytes() == frozen and snapshot(prefix) == original,
                    "stale Pixi refusal changed prior lock/prefix")
            (repo / "pixi.toml").write_text(manifest)
            consumer.approve(repo, env)

            # Closing the only acquisition origin proves retained offline reuse.
            requests = list(server.requests)
            server.close()
            closed = True
            offline = dict(env, GRIPSACK_CONDA_HELPER=str(base / "unavailable-helper"))
            warm = json.loads(consumer.run([consumer.grip, "build", "data", "--json"], cwd=repo, env=offline))
            require(warm == result, "offline build selected a different retained output")
            consumer.run([consumer.grip, "gc"], cwd=repo, env=offline)
            require(lock.read_bytes() == frozen and (repo / "pixi.lock").read_text() == pixi_lock,
                    "offline consumer rewrote a frozen lock")
            require(snapshot(prefix) == original and sha(archive) == digest, "offline reuse changed prior bytes")
            outcomes.append({"format": extension, "archive": str(archive), "sha256": digest,
                "bytes": archive.stat().st_size, "pixi_lock_sha256": sha(repo / "pixi.lock"),
                "gripsack_lock_sha256": sha(lock), "prefix": str(prefix), "retained_prefix": original,
                "requests": requests, "status": "passed", "helper_selection": "compiled-pin private mirror",
                "exercise": "real Pixi import + archive hash admission + prefix patch + independent native core receipt/tree validation",
                "negative_cases": ["wrong helper pin", "wrong archive digest", "unapproved source", "approved stale Pixi input"],
                "reuse": "origin stopped and helper unavailable; retained build/gc; locks and prefix unchanged"})
        finally:
            if not closed:
                server.close()
