"""Real pinned-helper Conda archive reconstruction through package pack/import.

Uses the existing local TLS/Pixi archive fixture convention, extended with a
static executable and prefix-replaced data. The separately prepared real Python
lane exercises compiled extensions, Conda noarch entrypoints and a pinned wheel.
Requires a locally staged current-release helper; tests never download or skip.
"""
from __future__ import annotations

import base64
import bz2
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import tarfile
import zipfile
from urllib.parse import unquote, urlparse

import pytest

from conftest import GRIP, run_grip
from test_http_policy import HttpFixture
from test_package_sharing import canonical, checked, independent_sign, native_target, package, recipient
from test_update_surveys import survey_snapshot

pytestmark = pytest.mark.skipif(
    platform.system() != "Linux" or platform.machine() != "x86_64",
    reason="native Linux x86_64 Conda sharing contract",
)

PLACEHOLDER = "/opt/anaconda1anaconda2anaconda3"
PROBE = r'''
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
int main(void) {
    char root[PATH_MAX], data[PATH_MAX], value[PATH_MAX], expected[PATH_MAX];
    ssize_t n = readlink("/proc/self/exe", root, sizeof(root) - 1);
    if (n <= 0 || n >= sizeof(root) - 1) return 91;
    root[n] = 0;
    char *p = strrchr(root, '/'); if (!p) return 92; *p = 0;
    p = strrchr(root, '/'); if (!p) return 93; *p = 0;
    snprintf(data, sizeof(data), "%s/share/value", root);
    FILE *f = fopen(data, "r"); if (!f) return 94;
    if (!fgets(value, sizeof(value), f)) return 95;
    fclose(f);
    snprintf(expected, sizeof(expected), "%s/retained-value\n", root);
    if (strcmp(value, expected)) return 96;
    printf("conda-prefix=%s\n", root);
    return 0;
}
'''


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pinned_helper(producer):
    """Mirror exact release bytes; an operator override is not package authority."""
    root = Path(__file__).resolve().parents[1]
    pins = (root / "crates/gripsack-fetch/src/conda_pins.rs").read_text()
    version = re.search(r'CONDA_VERSION: &str = "([^"]+)"', pins).group(1)
    hashes = re.findall(r'"([0-9a-f]{64})"', pins)
    assert len(hashes) == 1, "Linux-only helper pin table must contain exactly one digest"
    name = f"gripsack-conda-{version}-x86_64-unknown-linux-musl"
    mirror = os.environ.get("GRIPSACK_CONDA_HELPER_MIRROR")
    override = os.environ.get("GRIPSACK_CONDA_HELPER")
    if mirror:
        parsed = urlparse(mirror)
        assert parsed.scheme in ("", "file"), "offline fixture requires a local helper mirror"
        source = Path(unquote(parsed.path) if parsed.scheme else mirror) / name
    elif override:
        source = Path(override)
    else:
        source = root / "dist" / name
    assert source.is_file(), (
        "required core-pinned helper is absent; provide GRIPSACK_CONDA_HELPER_MIRROR "
        f"with {name} or GRIPSACK_CONDA_HELPER pointing to those exact release bytes"
    )
    assert sha(source) == hashes[0], "fixture helper differs from the current compiled core pin"
    target = producer / "helper mirror"
    target.mkdir()
    shutil.copyfile(source, target / name)
    (target / name).chmod(0o755)
    return target, hashes[0]


def original_archive(producer, version="1.0"):
    compiler = shutil.which("cc")
    assert compiler, "Conda runtime fixture requires cc and static libc development files"
    source = producer / "probe.c"
    source.write_text(PROBE)
    executable = producer / "probe"
    checked([compiler, "-static", source, "-o", executable])
    value = f"{PLACEHOLDER}/retained-value\n".encode()
    payload = {"bin/probe": executable.read_bytes(), "share/value": value}
    paths = []
    for name, data in payload.items():
        entry = {"_path": name, "path_type": "hardlink",
                 "sha256": hashlib.sha256(data).hexdigest(), "size_in_bytes": len(data)}
        if name == "share/value":
            entry.update(prefix_placeholder=PLACEHOLDER, file_mode="text")
        paths.append(entry)
    files = {
        "info/index.json": canonical({"name": "fixture", "version": version, "build": "0",
                                      "build_number": 0, "subdir": "linux-64", "depends": []}),
        "info/paths.json": canonical({"paths_version": 1, "paths": paths}),
        "info/files": b"bin/probe\nshare/value\n",
        "info/has_prefix": f"{PLACEHOLDER} text share/value\n".encode(),
        **payload,
    }
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w") as archive:
        for name, data in files.items():
            member = tarfile.TarInfo(name)
            member.mode = 0o755 if name == "bin/probe" else 0o644
            member.size = len(data)
            member.mtime = 0
            archive.addfile(member, io.BytesIO(data))
    result = producer / f"fixture-{version}-0.tar.bz2"
    result.write_bytes(bz2.compress(stream.getvalue()))
    return result


def workspace_command(env, repo, *arguments, code=0):
    result = run_grip([str(GRIP.resolve()), *arguments], env=env, cwd=repo,
                      capture_output=True, text=True, timeout=120)
    assert result.returncode == code, result.stdout + result.stderr
    return result


def policy(env, public):
    directory = Path(env["GRIPSACK_HOME"]) / "sharing"
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "trust.json").write_bytes(canonical({
        "version": 1,
        "keys": [{"key_id": "fixture", "publisher": "fixture-publisher",
                  "namespaces": ["fixture/conda"], "public_key": public,
                  "allowed_layouts": ["conda"],
                  "required_validators": ["manifest-integrity", "payload-integrity", "layout-safety"]}],
        "revoked_keys": [], "revoked_manifests": [],
    }))


def offline_recipient(root, public):
    env = recipient(root)
    env.update(GRIPSACK_CONDA_HELPER=str(root / "forbidden ambient helper"),
               GRIPSACK_CONDA_HELPER_MIRROR=str(root / "absent helper mirror"),
               HTTPS_PROXY="http://127.0.0.1:9", HTTP_PROXY="http://127.0.0.1:9",
               ALL_PROXY="http://127.0.0.1:9", NO_PROXY="")
    for name in ("SSL_CERT_FILE", "PYTHONHOME", "PYTHONPATH"):
        env.pop(name, None)
    policy(env, public)
    return env


def remove_producer(path):
    # Published files are immutable; only disposable fixture directory modes
    # are relaxed to remove the entire producer store before import.
    for directory, _, _ in os.walk(path):
        Path(directory).chmod(0o700)
    shutil.rmtree(path)


def pixi_documents(repo, stem, server, archive):
    channel = server.base + "/channel"
    url = f"{channel}/linux-64/{archive.name}"
    version = archive.name.removeprefix("fixture-").removesuffix("-0.tar.bz2")
    (repo / f"{stem}.toml").write_text(
        f"[workspace]\nname='fixture'\nchannels=['{channel}']\nplatforms=['linux-64']\n"
        f"[dependencies]\nfixture='=={version}'\n")
    (repo / f"{stem}.lock").write_text(
        "version: 6\nenvironments:\n  default:\n    channels:\n"
        f"    - url: {channel}/\n    packages:\n      linux-64:\n      - conda: {url}\n"
        f"packages:\n- conda: {url}\n  sha256: {sha(archive)}\n  size: {archive.stat().st_size}\n"
        "  depends: []\n  constrains: []\n")


def pixi_catalog(repo, outputs):
    (repo / "gripsack.ts").write_text('''import {
  defineWorkspace, workspace, pkg, provider, pixi, inputFile,
} from "@gripsack/core";
const outputs: [string, string][] = ''' + json.dumps(list(outputs.items())) + ''';
export default defineWorkspace(() => workspace({
  inputs: outputs.flatMap(([name, stem]) => [
    inputFile(name + "-manifest", stem + ".toml"), inputFile(name + "-lock", stem + ".lock"),
  ]),
  outputs: outputs.map(([name]) => pkg(name, {
    producer: provider(pixi.fromLock({manifest:name+"-manifest",lock:name+"-lock",environment:"default"})),
    commands: {probe: "bin/probe"}, target:{os:"linux",arch:"x86_64",abi:"gnu"},
    layout:{kind:"prefix_materialized"},
  })),
}));
''')


def pixi_workspace(repo, server, archive):
    repo.mkdir()
    (repo / "env.toml").write_text('[env]\nname = "portable-conda-fixture"\n\n[eval]\n'
        + f'env = {{ SSL_CERT_FILE = {json.dumps(str(server.cert))} }}\n')
    pixi_documents(repo, "pixi", server, archive)
    pixi_catalog(repo, {"data": "pixi"})
    return repo


def test_pixi_failed_survey_leaves_no_captured_inputs(sandbox):
    env = recipient(sandbox / "recipient")
    state = Path(env["GRIPSACK_HOME"])
    archive = original_archive(sandbox)
    server = HttpFixture(tls=True)
    try:
        repo = pixi_workspace(sandbox / "workspace", server, archive)
        (repo / "pixi.lock").write_bytes(b"\xff")
        before = survey_snapshot(repo, state)
        result = workspace_command(env, repo, "update", "--check", code=2)
        assert "pixi lock is not UTF-8" in result.stdout
        assert survey_snapshot(repo, state) == before
        assert not server.requests
    finally:
        server.close()


@pytest.mark.native_integration
def test_pixi_survey_validates_without_publishing(sandbox):
    env = recipient(sandbox / "recipient")
    if not env.get("GRIPSACK_CONDA_HELPER"):
        mirror = env.get("GRIPSACK_CONDA_HELPER_MIRROR")
        assert mirror and urlparse(mirror).scheme in ("", "file"), (
            "supply a local Conda helper or offline helper mirror"
        )
    state = Path(env["GRIPSACK_HOME"])
    store = state / "store"
    archive = original_archive(sandbox)
    server = HttpFixture(tls=True)
    server.archive = archive.read_bytes()
    try:
        repo = pixi_workspace(sandbox / "workspace", server, archive)
        cold = survey_snapshot(repo, state)
        assert cold == (None, None, None)
        workspace_command(env, repo, "update", "--check", code=1)
        assert survey_snapshot(repo, state) == cold
        assert server.requests == [f"/channel/linux-64/{archive.name}"]

        workspace_command(env, repo, "update")
        workspace_command(env, repo, "gc")
        documents = [repo / "pixi.toml", repo / "pixi.lock"]
        assert {p.read_bytes() for p in store.glob("*-workspace-input/value")} == {
            path.read_bytes() for path in documents
        }
        assert [p.read_bytes() for p in store.glob("*-conda-archive/archive")] == [server.archive]
        warm = survey_snapshot(repo, state)
        pins = [pin for resolution in json.loads(warm[0])["resolutions"].values()
                for pin in resolution["pins"]]
        assert len(pins) == 1
        assert pins[0]["conda"]["packages"][0]["sha256"] == sha(archive)
        for field in ("manifest_sha256", "lock_sha256"):
            assert re.fullmatch("[a-f0-9]{64}", pins[0]["source"][field])
        workspace_command(env, repo, "update", "--check")
        assert survey_snapshot(repo, state) == warm

        # Both documents now have unretained identities but select the very same archive.
        for path in documents:
            with path.open("a") as stream:
                stream.write("# new input identity\n")
        workspace_command(env, repo, "update", "--check", code=1)
        assert survey_snapshot(repo, state) == warm
        server.archive = b"corrupt archive from the origin"
        result = workspace_command(env, repo, "update", "--check", code=2)
        assert "fixture archive download" in result.stdout
        assert survey_snapshot(repo, state) == warm
    finally:
        server.close()


def printed_probe(output, env):
    prefix = Path(output["path"])
    assert Path(output["commands"]["probe"]) == prefix / "bin/probe"
    assert checked([output["commands"]["probe"]], env=env, text=True).stdout == f"conda-prefix={prefix}\n"


def historical_lock_roots(state, repo, inputs):
    """Install the prior version's actual v2 (repo, output) roots, not extra consumers."""
    roots = state / "roots/outputs"
    old_ids = {hashlib.sha256(canonical([str(repo.resolve()), name])).hexdigest() for name in inputs}
    if {p.stem for p in roots.glob("*.json")} == old_ids:
        return  # Already the historical producer when exercising a pre-fix binary.
    [current] = roots.glob("*.json")
    record = json.loads(current.read_bytes())
    assert record["version"] == 2 and record["kind"] == "output"
    paths = {Path(path) for path in record["store_paths"]}
    expected = set()
    for name, (stem, archive) in inputs.items():
        documents = {(repo / f"{stem}.{ext}").read_bytes() for ext in ("toml", "lock")}
        closure = {p for p in paths if (
            p.name.endswith("-conda-archive") and sha(p / "archive") == sha(archive)
            or p.name.endswith("-workspace-input") and (p / "value").read_bytes() in documents)}
        assert len(closure) == 3
        expected.update(closure)
        identity = hashlib.sha256(canonical([str(repo.resolve()), name])).hexdigest()
        (roots / f"{identity}.json").write_bytes(canonical({
            **record, "id": identity, "store_paths": sorted(map(str, closure)),
        }))
    assert paths == expected
    current.unlink()


@pytest.mark.native_integration
@pytest.mark.parametrize("legacy", [False, True], ids=["current", "first-after-historical-build"])
def test_unchanged_conda_update_keeps_printed_build_after_gc(sandbox, legacy):
    env = recipient(sandbox / "isolated home")
    state = Path(env["GRIPSACK_HOME"])
    archive = original_archive(sandbox)
    server = HttpFixture(tls=True)
    server.archive = archive.read_bytes()
    try:
        repo = pixi_workspace(sandbox / "workspace", server, archive)
        workspace_command(env, repo, "update")
        if legacy:
            historical_lock_roots(state, repo, {"data": ("pixi", archive)})
        # No project, profile, import or other home pins this prefix.
        built = json.loads(workspace_command(env, repo, "build", "data", "--json").stdout)["outputs"][0]
        lock = (repo / "gripsack.lock").read_bytes()
        workspace_command(env, repo, "update")
        assert (repo / "gripsack.lock").read_bytes() == lock
    finally:
        server.close()
    workspace_command(env, repo, "gc")
    assert not (state / "current").exists()
    printed_probe(built, env)


@pytest.mark.native_integration
@pytest.mark.parametrize("outcome", ["failed-sibling", "scoped-success", "remove-output"])
def test_conda_lock_archives_follow_only_published_pins(sandbox, outcome):
    env = recipient(sandbox / "isolated home")
    state = Path(env["GRIPSACK_HOME"])
    store = state / "store"
    first, sibling, replacement = [original_archive(sandbox, version) for version in ("1.0", "2.0", "3.0")]
    a, b = HttpFixture(tls=True), HttpFixture(tls=True)
    a.archive, b.archive = first.read_bytes(), sibling.read_bytes()
    try:
        repo = pixi_workspace(sandbox / "workspace", a, first)
        pixi_documents(repo, "second", b, sibling)
        pixi_catalog(repo, {"data": "pixi", "sibling": "second"})
        workspace_command(env, repo, "update")
        if outcome == "scoped-success":
            historical_lock_roots(state, repo, {"data": ("pixi", first), "sibling": ("second", sibling)})
        lock = repo / "gripsack.lock"
        original = lock.read_bytes()
        a.archive = replacement.read_bytes()
        pixi_documents(repo, "pixi", a, replacement)
        if outcome == "failed-sibling":
            (repo / "second.lock").write_bytes(b"\xff")
            result = workspace_command(env, repo, "update", code=1)
            assert "pixi lock is not UTF-8" in result.stderr
            assert lock.read_bytes() == original
            assert a.requests.count(f"/channel/linux-64/{replacement.name}") == 1
            pixi_documents(repo, "pixi", a, first)
            pixi_documents(repo, "second", b, sibling)
            active = {"data": ("pixi", first), "sibling": ("second", sibling)}
        elif outcome == "scoped-success":
            workspace_command(env, repo, "update", "data")
            assert lock.read_bytes() != original
            active = {"data": ("pixi", replacement), "sibling": ("second", sibling)}
        else:
            pixi_catalog(repo, {"data": "pixi"})
            workspace_command(env, repo, "update")
            active = {"data": ("pixi", replacement)}
    finally:
        a.close()
        b.close()
    # No build, profile or process lease has protected either candidate or old
    # lock. Compare the complete surviving archive set, not merely existence.
    workspace_command(env, repo, "gc")
    assert not (state / "current").exists()
    assert not list(store.glob("*-workspace-package"))
    assert {sha(p) for p in store.glob("*-conda-archive/archive")} == {
        sha(archive) for _, archive in active.values()
    }
    assert {p.read_bytes() for p in store.glob("*-workspace-input/value")} == {
        (repo / f"{stem}.{ext}").read_bytes() for stem, _ in active.values() for ext in ("toml", "lock")
    }
    locked = {pin["output"]: pin["conda"]["packages"][0]["sha256"]
              for resolution in json.loads(lock.read_bytes())["resolutions"].values()
              for pin in resolution["pins"]}
    assert locked == {name: sha(archive) for name, (_, archive) in active.items()}
    built = json.loads(workspace_command(env, repo, "build", *active, "--json").stdout)
    for output in built["outputs"]:
        printed_probe(output, env)


@pytest.mark.native_integration
@pytest.mark.requires_fault_controls
def test_interrupted_conda_lock_handoff_survives_gc_and_reconciles(sandbox):
    home = sandbox / "recipient"
    env = recipient(home)
    state = Path(env["GRIPSACK_HOME"])
    first, replacement = [original_archive(sandbox, version) for version in ("1.0", "2.0")]
    def archives():
        return {sha(p) for p in (state / "store").glob("*-conda-archive/archive")}

    server = HttpFixture(tls=True)
    try:
        repo = pixi_workspace(sandbox / "workspace", server, first)
        server.archive = first.read_bytes()
        workspace_command(env, repo, "update")
        lock = repo / "gripsack.lock"
        old = lock.read_bytes()
        [root] = (state / "roots/outputs").glob("*.json")
        server.archive = replacement.read_bytes()
        pixi_documents(repo, "pixi", server, replacement)
        saved = sandbox / "before-update"
        shutil.copytree(home, saved, symlinks=True)
        trace = sandbox / "update.trace"
        workspace_command(dict(env, GRIPSACK_FS_TRACE=str(trace)), repo, "update")
        new = lock.read_bytes()
        assert new != old
        rows = [line.split("\t", 3) for line in trace.read_text().splitlines()]
        publishes = [(row[0], json.loads(row[3])) for row in rows if row[1:3] == ["After", "FilePublish"]]
        cuts = [(ordinal, name) for ordinal, name in publishes if name in (root.name, "gripsack.lock")]
        assert [name for _, name in cuts] == [root.name, "gripsack.lock", root.name]
        for index, (ordinal, name) in enumerate(cuts):
            remove_producer(home)
            shutil.copytree(saved, home, symlinks=True)
            lock.write_bytes(old)
            pixi_documents(repo, "pixi", server, replacement)
            trace.unlink()
            failed_env = dict(env, GRIPSACK_FS_TRACE=str(trace), GRIPSACK_FS_CUT=ordinal)
            workspace_command(failed_env, repo, "update", code=1)
            observed = trace.read_text().splitlines()[int(ordinal) - 1].split("\t", 3)
            assert observed[1:] == ["After", "FilePublish", json.dumps(name)]
            visible = first if index == 0 else replacement
            assert lock.read_bytes() == (old if index == 0 else new)
            workspace_command(env, repo, "gc")
            assert archives() == ({sha(first), sha(replacement)} if index < 2 else {sha(replacement)})
            # The retry is a no-change publication against whatever lock name
            # survived. It must seal that name before dropping excess roots.
            pixi_documents(repo, "pixi", server, visible)
            before = lock.read_bytes()
            workspace_command(env, repo, "update")
            assert lock.read_bytes() == before
            workspace_command(env, repo, "gc")
            assert archives() == {sha(visible)}
    finally:
        server.close()


@pytest.mark.native_integration
def test_original_archives_reconstruct_at_two_offline_final_prefixes(sandbox):
    producer = sandbox / "producer home with spaces"
    env = recipient(producer)
    mirror, helper_digest = pinned_helper(producer)
    env["GRIPSACK_CONDA_HELPER_MIRROR"] = str(mirror)
    env.pop("GRIPSACK_CONDA_HELPER", None)
    archive = original_archive(producer)
    original = archive.read_bytes()
    archive_digest = sha(archive)
    server = HttpFixture(tls=True)
    server.archive = original
    try:
        repo = pixi_workspace(producer / "workspace", server, archive)
        workspace_command(env, repo, "update")
        built = json.loads(workspace_command(env, repo, "build", "data", "--json").stdout)
        producer_prefix = Path(built["outputs"][0]["path"])
        assert checked([producer_prefix / "bin/probe"], env=env, text=True).stdout == f"conda-prefix={producer_prefix}\n"
        frozen = json.loads((repo / "gripsack.lock").read_bytes())
        closures = [pin["conda"] for resolution in frozen["resolutions"].values()
                    for pin in resolution["pins"] if "conda" in pin]
        assert len(closures) == 1
        assert closures[0]["packages"][0]["sha256"] == archive_digest
        assert server.requests and set(server.requests) == {f"/channel/linux-64/{archive.name}"}
    finally:
        # Every import below runs after the sole archive origin has disappeared.
        server.close()

    private = producer / "private.pem"
    public_der = producer / "public.der"
    private_der = producer / "private.der"
    checked(["openssl", "genpkey", "-algorithm", "ED25519", "-out", private])
    checked(["openssl", "pkey", "-in", private, "-outform", "DER", "-out", private_der])
    checked(["openssl", "pkey", "-in", private, "-pubout", "-outform", "DER", "-out", public_der])
    raw = private_der.read_bytes()
    assert len(raw) == 48 and raw[:16] == bytes.fromhex("302e020100300506032b657004220420")
    seed = producer / "seed"
    seed.write_bytes(raw[-32:])
    seed.chmod(0o600)
    public = base64.b64encode(public_der.read_bytes()[-32:]).decode()
    specification = {
        "version": 1, "namespace": "fixture/conda", "publisher": "fixture-publisher",
        "target": native_target(), "layout": {"kind": "conda"},
        "components": [{"id": "environment", "payload": None, "tree": None, "runtime": [],
                        "inventory": [], "external": None, "conda": {"lock": closures[0], "archives": [],
                        "helper": {"sha256": "0" * 64, "size": 0, "media_type": ""}, "helper_notices": {}}}],
        "roots": ["environment"],
        "commands": {"probe": {"component": "environment", "executable": "bin/probe", "args": []}},
        "environment": {}, "licenses": [], "redistribution_authorized": True,
    }
    spec = producer / "portable.json"
    spec.write_bytes(canonical(specification))
    bundle = sandbox / "offline conda bundle"
    package(env, "pack", "--manifest", spec, "--source", repo, "--output", bundle,
            "--key", seed, "--key-id", "fixture")
    independent_sign(bundle, private, producer)
    manifest_bytes = (bundle / "manifest.json").read_bytes()
    manifest = json.loads(manifest_bytes)
    identity = hashlib.sha256(manifest_bytes).hexdigest()
    portable = manifest["components"][0]["conda"]
    assert portable["lock"] == closures[0]
    assert portable["helper"]["sha256"] == helper_digest
    assert portable["archives"] == [{"sha256": archive_digest, "size": len(original),
                                     "media_type": "application/vnd.gripsack.conda.archive.v1"}]
    assert (bundle / "blobs" / archive_digest).read_bytes() == original
    assert sha(bundle / "blobs" / helper_digest) == helper_digest
    remove_producer(producer)
    assert not producer.exists()

    imported = []
    for name in ("recipient one", "recipient two with a substantially longer spaced home"):
        consumer = offline_recipient(sandbox / name, public)
        facts = json.loads(package(consumer, "import", "--bundle", bundle).stdout)
        prefix = Path(facts["payload"])
        assert facts["manifest"] == identity
        assert prefix.is_relative_to(Path(consumer["GRIPSACK_HOME"]))
        assert prefix != producer_prefix
        assert (prefix / "share/value").read_text() == f"{prefix}/retained-value\n"
        assert checked([prefix / "bin/probe"], env=consumer, text=True).stdout == f"conda-prefix={prefix}\n"
        state = Path(consumer["GRIPSACK_HOME"])
        assert not (state / "current").exists()
        assert not (Path(consumer["HOME"]) / ".config/systemd/user").exists()
        archives = list((state / "store").rglob("archive"))
        assert any(path.read_bytes() == original for path in archives)
        helpers = list((state / "store").rglob("gripsack-conda"))
        assert any(sha(path) == helper_digest for path in helpers)
        assert json.loads(package(consumer, "inspect", "--manifest", identity).stdout)["manifest"] == identity
        exported = Path(consumer["HOME"]) / "reexport"
        package(consumer, "export", "--manifest", identity, "--output", exported)
        assert (exported / "manifest.json").read_bytes() == manifest_bytes
        assert (exported / "blobs" / archive_digest).read_bytes() == original
        imported.append((consumer, prefix))
    assert imported[0][1] != imported[1][1]
    assert (imported[0][1] / "share/value").read_bytes() != (imported[1][1] / "share/value").read_bytes()

    for damage, digest in (("missing archive", archive_digest), ("missing helper", helper_digest),
                           ("tampered archive", archive_digest), ("tampered helper", helper_digest)):
        damaged = sandbox / damage
        shutil.copytree(bundle, damaged)
        blob = damaged / "blobs" / digest
        if damage.startswith("missing"):
            blob.unlink()
        else:
            with blob.open("r+b") as file:
                byte = file.read(1)
                file.seek(0)
                file.write(bytes([byte[0] ^ 1]))
        consumer = offline_recipient(sandbox / (damage + " recipient"), public)
        package(consumer, "import", "--bundle", damaged, ok=False)
        assert not list((Path(consumer["GRIPSACK_HOME"]) / "store").rglob("bin/probe"))

    shutil.rmtree(bundle)
    for consumer, prefix in imported:
        package(consumer, "inspect", "--manifest", identity)
        assert checked([prefix / "bin/probe"], env=consumer, text=True).stdout == f"conda-prefix={prefix}\n"


@pytest.mark.native_integration
def test_python_compiled_extensions_and_noarch_entrypoint_from_offline_bundle(sandbox):
    fixture = os.environ.get("GRIPSACK_TEST_CONDA_RUNTIME_BUNDLE")
    assert fixture, (
        "required Python runtime archive fixture is absent; explicitly prepare it with "
        "python e2e/test_package_conda_offline.py prepare --directory /tmp/conda-runtime-inputs, "
        "then set GRIPSACK_TEST_CONDA_RUNTIME_BUNDLE=/tmp/conda-runtime-inputs/bundle"
    )
    source = Path(fixture).resolve()
    assert source.is_dir(), f"required runtime bundle directory is absent: {source}"
    bundle = sandbox / "python offline transfer"
    shutil.copytree(source, bundle)
    manifest_bytes = (bundle / "manifest.json").read_bytes()
    manifest = json.loads(manifest_bytes)
    assert manifest["namespace"] == "fixture/conda"
    assert manifest["publisher"] == "fixture-publisher"
    assert manifest["layout"] == {"kind": "conda"}
    assert len(manifest["components"]) == 1
    portable = manifest["components"][0]["conda"]
    records = {record["name"]: record for record in portable["lock"]["packages"]}
    assert "python" in records and records["wheel"]["noarch"] == "python"
    assert manifest["commands"]["python"]["executable"] == "bin/python"
    assert manifest["commands"]["wheel"]["executable"] == "bin/wheel"
    wheels = {record["name"]: record for record in portable["lock"]["wheels"]}
    assert wheels["gripsack-offline-wheel"]["version"] == "1.0"
    wheel_digest = wheels["gripsack-offline-wheel"]["sha256"]
    assert any(blob["sha256"] == wheel_digest
               and blob["media_type"] == "application/vnd.gripsack.python.wheel.v1"
               for blob in portable["archives"])
    original = {descriptor["sha256"]: sha(bundle / "blobs" / descriptor["sha256"])
                for descriptor in [*portable["archives"], portable["helper"]]}
    assert all(expected == actual for expected, actual in original.items())

    # Test-local authority signs the unchanged canonical production manifest.
    # The external fixture never supplies a recipient trust configuration.
    signing = sandbox / "runtime fixture authority"
    signing.mkdir()
    private = signing / "private.pem"
    public_der = signing / "public.der"
    checked(["openssl", "genpkey", "-algorithm", "ED25519", "-out", private])
    checked(["openssl", "pkey", "-in", private, "-pubout", "-outform", "DER", "-out", public_der])
    public = base64.b64encode(public_der.read_bytes()[-32:]).decode()
    independent_sign(bundle, private, signing)
    identity = hashlib.sha256(manifest_bytes).hexdigest()
    wheel_blob = bundle / "blobs" / wheel_digest
    wheel_bytes = wheel_blob.read_bytes()
    for damage in ("missing wheel", "tampered wheel"):
        if damage == "missing wheel":
            wheel_blob.unlink()
        else:
            wheel_blob.write_bytes(bytes([wheel_bytes[0] ^ 1]) + wheel_bytes[1:])
        rejected = offline_recipient(sandbox / damage, public)
        package(rejected, "import", "--bundle", bundle, ok=False)
        assert not list((Path(rejected["GRIPSACK_HOME"]) / "store").rglob("bin/python"))
        wheel_blob.write_bytes(wheel_bytes)
    imported = []
    for name in ("Python recipient one with spaces", "Python recipient two " + "longer " * 8):
        env = offline_recipient(sandbox / name, public)
        facts = json.loads(package(env, "import", "--bundle", bundle).stdout)
        assert facts["manifest"] == identity
        prefix = Path(facts["payload"])
        assert prefix.is_relative_to(Path(env["GRIPSACK_HOME"]))
        imported.append((env, prefix))
    assert imported[0][1] != imported[1][1]
    shutil.rmtree(bundle)
    shutil.rmtree(signing)
    for env, prefix in imported:
        # Drop re-export/cache inputs before execution. Durable original inputs,
        # helper and realized prefix must survive the ordinary GC root protocol.
        workspace_command(env, Path(env["HOME"]), "gc")
        before = {str(path.relative_to(prefix)): sha(path) for path in prefix.rglob("*")
                  if path.is_file() and not path.is_symlink()}
        probe = (
            "import _ssl, _sqlite3, gripsack_offline_wheel, json, sys; "
            "print(json.dumps({'prefix':sys.prefix,'ssl':_ssl.__file__,"
            "'sqlite':_sqlite3.__file__,'wheel':gripsack_offline_wheel.__file__}))"
        )
        result = checked([prefix / "bin/python", "-I", "-B", "-c", probe], env=env, text=True)
        measured = json.loads(result.stdout)
        assert Path(measured["prefix"]) == prefix
        for module in ("ssl", "sqlite"):
            extension = Path(measured[module])
            assert extension.is_relative_to(prefix) and extension.suffix == ".so"
        assert Path(measured["wheel"]).is_relative_to(prefix)
        wheel = checked([prefix / "bin/wheel", "version"],
                        env=dict(env, PYTHONDONTWRITEBYTECODE="1"), text=True)
        assert wheel.stdout.strip() == f"wheel {records['wheel']['version']}"
        installed = checked([prefix / "bin/offline-wheel"],
                            env=dict(env, PYTHONDONTWRITEBYTECODE="1"), text=True)
        assert installed.stdout.strip() == "gripsack offline wheel 1.0"
        package(env, "inspect", "--manifest", identity)
        after = {str(path.relative_to(prefix)): sha(path) for path in prefix.rglob("*")
                 if path.is_file() and not path.is_symlink()}
        assert before == after, "runtime invocation changed the immutable Conda prefix"
        state = Path(env["GRIPSACK_HOME"])
        retained = {sha(path) for path in (state / "store").rglob("archive")}
        assert set(original) - {portable["helper"]["sha256"]} <= retained
        assert any(sha(path) == portable["helper"]["sha256"]
                   for path in (state / "store").rglob("gripsack-conda"))
        assert not (state / "current").exists()


def binary_wheel_fixture(directory):
    """A real wheel with hashed RECORD and a console entrypoint, not a mock."""
    distribution = "gripsack_offline_wheel-1.0.dist-info"
    files = {
        "gripsack_offline_wheel/__init__.py": (
            b'def main():\n    print("gripsack offline wheel 1.0")\n'
        ),
        f"{distribution}/METADATA": (
            b"Metadata-Version: 2.1\nName: gripsack-offline-wheel\nVersion: 1.0\n"
            b"Requires-Python: >=3.8\n\nOffline wheel acceptance fixture.\n"
        ),
        f"{distribution}/WHEEL": (
            b"Wheel-Version: 1.0\nGenerator: gripsack-acceptance\n"
            b"Root-Is-Purelib: true\nTag: py3-none-any\n"
        ),
        f"{distribution}/entry_points.txt": (
            b"[console_scripts]\noffline-wheel = gripsack_offline_wheel:main\n"
        ),
    }
    records = io.StringIO(newline="")
    writer = csv.writer(records, lineterminator="\n")
    for name, data in files.items():
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        writer.writerow([name, f"sha256={digest}", len(data)])
    writer.writerow([f"{distribution}/RECORD", "", ""])
    files[f"{distribution}/RECORD"] = records.getvalue().encode()
    wheel = directory / "gripsack_offline_wheel-1.0-py3-none-any.whl"
    with zipfile.ZipFile(wheel, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in files.items():
            member = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            member.external_attr = 0o100644 << 16
            archive.writestr(member, data)
    return wheel


def prepare_runtime_fixture(directory):
    """Explicit ONLINE fixture preparation; never called by pytest.

    Example:
      GRIPSACK_BIN=/path/to/grip GRIPSACK_CONDA_HELPER_MIRROR=/path/to/dist \
        python e2e/test_package_conda_offline.py prepare --directory /tmp/conda-runtime-inputs
      GRIPSACK_TEST_CONDA_RUNTIME_BUNDLE=/tmp/conda-runtime-inputs/bundle \
        pytest e2e/test_package_conda_offline.py

    Resolves once, preserves the complete production lock, builds through the
    pinned helper, and packs original retained archives. This is local fixture
    publication only; it does not publish to a registry or change real HOME.
    """
    directory = directory.resolve()
    assert not directory.exists(), "fixture preparation requires a fresh directory"
    directory.mkdir(parents=True)
    env = recipient(directory / "producer")
    mirror, _ = pinned_helper(Path(env["HOME"]))
    env["GRIPSACK_CONDA_HELPER_MIRROR"] = str(mirror)
    env.pop("GRIPSACK_CONDA_HELPER", None)
    repo = Path(env["HOME"]) / "workspace"
    repo.mkdir()
    wheel = binary_wheel_fixture(Path(env["HOME"]))
    server = HttpFixture(tls=True)
    server.archive = wheel.read_bytes()
    wheel_source = {
        "name": "gripsack-offline-wheel", "version": "1.0",
        "url": f"{server.base}/{wheel.name}", "sha256": sha(wheel),
    }
    (repo / "env.toml").write_text('[env]\nname = "portable-wheel-fixture"\n\n[eval]\n'
        + f'env = {{ SSL_CERT_FILE = {json.dumps(str(server.cert))} }}\n')
    (repo / "gripsack.ts").write_text('''import {
  defineWorkspace, workspace, pkg, provider, conda,
} from "@gripsack/core";
export default defineWorkspace(() => workspace({outputs:[pkg("runtime", {
  producer:provider(conda.environment({
    // Tk links X11; portable native use must own it rather than borrow the host.
    channels:["conda-forge"], packages:{python:"3.12.*",wheel:"==0.45.1","xorg-libx11":"*"},
    systemRequirements:{libc:{family:"glibc",version:"2.28"},linux:"4.18"},
    wheels:''' + json.dumps([wheel_source]) + ''',
  })),
  commands:{python:"bin/python",wheel:"bin/wheel","offline-wheel":"bin/offline-wheel"},
  target:{os:"linux",arch:"x86_64",abi:"gnu"}, layout:{kind:"prefix_materialized"},
})]}));
''')

    def approve():
        inspected = json.loads(workspace_command(env, repo, "trust", "inspect", str(repo), "--json").stdout)
        workspace_command(env, repo, "trust", "add", str(repo),
                          "--bundle", inspected["bundle_digest"], "--policy", inspected["policy_digest"])

    try:
        approve()
        workspace_command(env, repo, "update")
        approve()
        workspace_command(env, repo, "build", "runtime", "--json")
    finally:
        server.close()
    lock_bytes = (repo / "gripsack.lock").read_bytes()
    frozen = json.loads(lock_bytes)
    closures = [pin["conda"] for resolution in frozen["resolutions"].values()
                for pin in resolution["pins"] if "conda" in pin]
    assert len(closures) == 1
    (directory / "gripsack.lock").write_bytes(lock_bytes)
    specification = {
        "version": 1, "namespace": "fixture/conda", "publisher": "fixture-publisher",
        "target": {**native_target(), "abi": {"kind": "glibc", "minimum": "2.28"},
                   "minimum_kernel": "4.18"}, "layout": {"kind": "conda"},
        "components": [{"id": "environment", "payload": None, "tree": None, "runtime": [],
                        "inventory": [], "external": None, "conda": {"lock": closures[0], "archives": [],
                        "helper": {"sha256": "0" * 64, "size": 0, "media_type": ""}, "helper_notices": {}}}],
        "roots": ["environment"],
        "commands": {
            "python": {"component": "environment", "executable": "bin/python", "args": []},
            "wheel": {"component": "environment", "executable": "bin/wheel", "args": []},
            "offline-wheel": {"component": "environment", "executable": "bin/offline-wheel", "args": []},
        },
        "environment": {"PYTHONDONTWRITEBYTECODE": {"kind": "literal", "value": "1"}},
        "licenses": [], "redistribution_authorized": True,
    }
    spec = directory / "portable.json"
    spec.write_bytes(canonical(specification))
    seed = directory / "fixture-seed"
    seed.write_bytes(os.urandom(32))
    seed.chmod(0o600)
    bundle = directory / "bundle"
    package(env, "pack", "--manifest", spec, "--source", repo, "--output", bundle,
            "--key", seed, "--key-id", "fixture")
    seed.unlink()
    remove_producer(Path(env["HOME"]))
    print(json.dumps({"bundle": str(bundle), "manifest": sha(bundle / "manifest.json"),
                      "lock": str(directory / "gripsack.lock")}))


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="Explicit online preparation of a pinned offline runtime fixture")
    parser.add_argument("action", choices=["prepare"])
    parser.add_argument("--directory", type=Path, required=True)
    prepare_runtime_fixture(parser.parse_args().directory)
