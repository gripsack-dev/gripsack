"""Passive mixed environments use retained originals, never installed-prefix copies.

Ordinary tests are offline, using a test-owned static native executable. The
explicit native_integration lane also requires the independently prepared real
Python/Conda/wheel bundle; missing inputs fail rather than skip. Preparation is
owned by test_package_conda_offline.py and is never invoked by these tests.

Prepare separately (the preparation command is intentionally online):
  GRIPSACK_BIN=/path/to/grip GRIPSACK_CONDA_HELPER_MIRROR=/path/to/dist \
    python e2e/test_package_conda_offline.py prepare --directory /tmp/conda-runtime-inputs
Run the complete file with the already-built binary and evaluator:
  GRIPSACK_BIN=/path/to/grip GRIPSACK_DENO=/path/to/deno \
    GRIPSACK_TEST_CONDA_RUNTIME_BUNDLE=/tmp/conda-runtime-inputs/bundle \
    pytest -c e2e/pyproject.toml -o addopts='' e2e/test_package_environment.py
The prepared bundle contains the exact pinned helper and notices; recipients do
not need a helper override, producer prefix, cache, transport or network.
"""
from __future__ import annotations

import base64
import copy
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess

import pytest

from conftest import GRIP, make_workspace_repo, run_grip
from test_package_conda_offline import remove_producer, sha
from test_package_sharing import (
    canonical, checked, independent_sign, native_target, package, recipient,
)

pytestmark = pytest.mark.skipif(
    platform.system() != "Linux" or platform.machine() != "x86_64",
    reason="native Linux x86_64 passive environment qualification",
)

LITERAL = "signed literal $HOME ${USER}"
NATIVE = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char **argv) {
    const char *shared = getenv("PORTABLE_LITERAL");
    const char *local = getenv("LOCAL_LITERAL");
    if (!shared || !local) return 81;
    const char *composed = getenv("COMPOSED_LITERAL");
    if (!composed || strcmp(composed, "recipient-override")) return 82;
    printf("%s\n%s\n", shared, local);
    for (int i = 1; i < argc; ++i) printf("%s\n", argv[i]);
    const char *tool_only = getenv("TOOL_ONLY");
    if (tool_only) printf("tool-only=%s\n", tool_only);
    const char *task = getenv("TASK_ONLY");
    if (task) printf("%s\n", task);
    return 0;
}
'''
PROBE = '''import _ssl, _sqlite3, gripsack_offline_wheel, json, os, pathlib, subprocess, sys
report = {
    "prefix": sys.prefix,
    "ssl": _ssl.__file__,
    "sqlite": _sqlite3.__file__,
    "wheel_module": gripsack_offline_wheel.__file__,
    "native": subprocess.check_output(["native-child", "from-python"], text=True),
    "wheel": subprocess.check_output(["offline-wheel"], text=True).strip(),
    "noarch": subprocess.check_output(["wheel", "version"], text=True).strip(),
    "portable": os.environ["PORTABLE_LITERAL"],
    "local": os.environ["LOCAL_LITERAL"],
    "cmake": os.environ["CMAKE_PREFIX_PATH"],
    "pkgconfig": os.environ["PKG_CONFIG_PATH"],
    "composed": os.environ["COMPOSED_LITERAL"],
    "task_only": os.environ.get("TASK_ONLY"),
}
assert report["composed"] == "recipient-override"
if len(sys.argv) > 1:
    assert report["task_only"] == "task-scope"
    assert report["native"].endswith("from-python\\ntask-scope\\n")
    pathlib.Path(sys.argv[1]).write_text("explicit task only\\n")
else:
    assert report["task_only"] is None
print(json.dumps(report))
'''


def authority(directory):
    directory.mkdir()
    private = directory / "private.pem"
    private_der = directory / "private.der"
    public_der = directory / "public.der"
    checked(["openssl", "genpkey", "-algorithm", "ED25519", "-out", private])
    checked(["openssl", "pkey", "-in", private, "-outform", "DER", "-out", private_der])
    checked(["openssl", "pkey", "-in", private, "-pubout", "-outform", "DER", "-out", public_der])
    raw = private_der.read_bytes()
    assert len(raw) == 48 and raw[:16] == bytes.fromhex("302e020100300506032b657004220420")
    seed = directory / "seed"
    seed.write_bytes(raw[-32:])
    seed.chmod(0o600)
    return private, seed, base64.b64encode(public_der.read_bytes()[-32:]).decode()


def sign_unchecked(bundle, private, scratch, key_id):
    """Sign malformed fixtures independently so import, not sign, must reject."""
    independent_sign(bundle, private, scratch)
    envelope = json.loads((bundle / "envelope.json").read_bytes())
    envelope["signatures"][0]["keyid"] = key_id
    (bundle / "envelope.json").write_bytes(canonical(envelope))


def trust_environment(env, public, runtime_public=None):
    path = Path(env["GRIPSACK_HOME"]) / "sharing/trust.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    keys = [{
        "key_id": "environment", "publisher": "fixture-publisher",
        "namespaces": ["fixture/environment"], "public_key": public,
        "allowed_layouts": ["environment"],
        "required_validators": ["manifest-integrity", "payload-integrity", "layout-safety"],
    }]
    if runtime_public is not None:
        keys.append({
            "key_id": "fixture", "publisher": "fixture-publisher",
            "namespaces": ["fixture/conda"], "public_key": runtime_public,
            "allowed_layouts": ["conda"],
            "required_validators": ["manifest-integrity", "payload-integrity", "layout-safety"],
        })
    path.write_bytes(canonical({"version": 1, "keys": keys,
                                "revoked_keys": [], "revoked_manifests": []}))
    return path


def offline_environment(root, public, runtime_public=None):
    env = recipient(root)
    env.update(GRIPSACK_CONDA_HELPER=str(root / "forbidden ambient helper"),
               GRIPSACK_CONDA_HELPER_MIRROR=str(root / "absent helper mirror"),
               XDG_CACHE_HOME=str(root / ".cache"),
               HTTPS_PROXY="http://127.0.0.1:9", HTTP_PROXY="http://127.0.0.1:9",
               ALL_PROXY="http://127.0.0.1:9", NO_PROXY="")
    for name in ("SSL_CERT_FILE", "PYTHONHOME", "PYTHONPATH", "PYTHONSTARTUP",
                 "BASH_ENV", "ENV", "PORTABLE_LITERAL", "LOCAL_LITERAL",
                 "COMPOSED_LITERAL", "TASK_ONLY", "TOOL_ONLY",
                 "CMAKE_PREFIX_PATH", "PKG_CONFIG_PATH", "GRIPSACK_TEST_CONDA_RUNTIME_BUNDLE"):
        env.pop(name, None)
    trust_environment(env, public, runtime_public)
    return env


def native_source(producer):
    source = producer / "source"
    binary = source / "components/native/bin/native-probe"
    binary.parent.mkdir(parents=True)
    compiler = shutil.which("cc")
    assert compiler, "passive environment fixture requires cc and static libc development files"
    program = producer / "native.c"
    program.write_text(NATIVE)
    checked([compiler, "-static", program, "-o", binary])
    metadata = source / "components/native/share/pkgconfig"
    metadata.mkdir(parents=True)
    (metadata / "fixture.pc").write_text("Name: passive-native\nVersion: 1.0\n")
    return source, binary.read_bytes()


def specification():
    return {
        "version": 1, "namespace": "fixture/environment", "publisher": "fixture-publisher",
        "target": native_target(), "layout": {"kind": "environment"},
        "components": [{"id": "native", "payload": None, "tree": None,
                        "runtime": [], "conda": None, "external": None, "inventory": []}],
        "roots": ["native"],
        "commands": {"native-child": {"component": "native", "executable": "bin/native-probe",
                                      "args": ["from-manifest"]}},
        "environment": {
            "COMPOSED_LITERAL": {"kind": "literal", "value": "publisher-default"},
            "PKG_CONFIG_PATH": {"kind": "artifact", "output": "native", "selector": "share/pkgconfig"},
            "PORTABLE_LITERAL": {"kind": "literal", "value": LITERAL},
            "PYTHONDONTWRITEBYTECODE": {"kind": "literal", "value": "1"},
        },
        "licenses": [], "redistribution_authorized": True,
    }


def pack_environment(env, spec, source, bundle, seed):
    path = Path(env["HOME"]) / "environment.json"
    path.write_bytes(canonical(spec))
    return json.loads(package(env, "pack", "--manifest", path, "--source", source,
                              "--output", bundle, "--key", seed,
                              "--key-id", "environment").stdout)["manifest"]


def project(env, repo, *args, ok=True):
    result = run_grip([str(GRIP.resolve()), *args], cwd=repo, env=env,
                      capture_output=True, text=True, timeout=120)
    assert (result.returncode == 0) == ok, result.stdout + result.stderr
    return result


def consumer_workspace(home, identity, commands, *, mixed=False):
    task = ""
    if mixed:
        argv = ["python", "-I", "-B", "-c", PROBE, str(home / "explicit-task-ran")]
        task = ', task("probe", {environment:"dev",steps:[exec({argv:[' + ",".join(
            "lit(" + json.dumps(value) + ")" for value in argv
        ) + '],env:{TASK_ONLY:lit("task-scope")}})]})'
    return make_workspace_repo(home / "local project", '''
import {pkg, provider, source, environment, profile, task, exec, lit} from "@gripsack/core";
const target = {os:"linux",arch:"x86_64",abi:"gnu"};
export default [
  pkg("shared", {
    producer:provider(source.package({manifest:''' + json.dumps(identity) + '''})),
    commands:''' + json.dumps({name: "bin/" + name for name in commands}) + ''',
    target, layout:{kind:"prefix_materialized"},
  }),
  environment("dev", {packages:["shared"],target,env:{
    LOCAL_LITERAL:lit("recipient-local"), COMPOSED_LITERAL:lit("recipient-override"),
  }}),
  profile("selected", {environment:"dev"})''' + task + ''',
];
''')


def assert_passive(env):
    home = Path(env["HOME"])
    state = Path(env["GRIPSACK_HOME"])
    assert not (state / "current").exists()
    assert not (state / "buildkit").exists()
    assert not (home / ".config/systemd/user").exists()
    assert not (home / "explicit-task-ran").exists()
    assert "PORTABLE_LITERAL" not in env


def tree_bytes(prefix):
    return {str(path.relative_to(prefix)): sha(path) for path in prefix.rglob("*")
            if path.is_file() and not path.is_symlink()}


@pytest.fixture
def native_environment(sandbox):
    producer = sandbox / "native producer"
    env = recipient(producer)
    private, seed, public = authority(producer / "authority")
    source, original = native_source(producer)
    bundle = sandbox / "native environment transfer"
    identity = pack_environment(env, specification(), source, bundle, seed)
    return bundle, identity, public, private, producer, original


def test_native_environment_is_passive_and_consumed_by_existing_package_source(sandbox, native_environment):
    bundle, identity, public, _, producer, original = native_environment
    env = offline_environment(sandbox / "native recipient", public)
    repo = consumer_workspace(Path(env["HOME"]), identity, ["native-child"])
    remove_producer(producer)
    facts = json.loads(package(env, "import", "--bundle", bundle).stdout)
    assert facts["manifest"] == identity and facts["layout"] == "environment"
    assert_passive(env)
    shutil.rmtree(bundle)
    project(env, repo, "update")
    result = project(env, repo, "run", "--env", "dev", "--", "native-child", "recipient")
    assert result.stdout == f"{LITERAL}\nrecipient-local\nfrom-manifest\nrecipient\n"
    assert_passive(env)
    assert any(path.read_bytes() == original
               for path in (Path(env["GRIPSACK_HOME"]) / "store").rglob("native-probe"))


def test_environment_import_rejects_automation_unknown_fields_and_host_controls(sandbox, native_environment):
    bundle, _, public, private, producer, _ = native_environment
    original = json.loads((bundle / "manifest.json").read_bytes())
    forbidden = {
        "tasks": [{"run": "touch forbidden"}],
        "profiles": [{"activate": True}],
        "schedules": [{"when": "* * * * *"}],
        "hooks": [{"trigger": "post_activate", "run": "touch forbidden"}],
        "host": {"environment": {"HOME": "/forbidden"}},
        "unknown_authority": True,
    }
    cases = [(name, {**copy.deepcopy(original), name: value}) for name, value in forbidden.items()]
    for name in ("PATH", "LD_LIBRARY_PATH", "LD_PRELOAD", "PYTHONHOME", "PYTHONPATH",
                 "PYTHONSTARTUP", "BASH_ENV", "ENV", "SHELLOPTS", "BASHOPTS"):
        damaged = copy.deepcopy(original)
        damaged["environment"][name] = {"kind": "literal", "value": "forbidden"}
        damaged["environment"] = dict(sorted(damaged["environment"].items()))
        cases.append((name, damaged))
    for name, value in (
        ("host-reference", {"kind": "host", "name": "HOME"}),
        ("input-reference", {"kind": "input", "name": "unreviewed"}),
        ("unknown-value-kind", {"kind": "execute", "command": "touch forbidden"}),
        ("missing-artifact", {"kind": "artifact", "output": "absent", "selector": "."}),
        ("escaping-artifact", {"kind": "artifact", "output": "native", "selector": "../outside"}),
        ("producer-path-literal", {"kind": "literal", "value": str(producer / "installed-prefix")}),
    ):
        damaged = copy.deepcopy(original)
        damaged["environment"]["PORTABLE_LITERAL"] = value
        cases.append((name, damaged))
    for name, damaged in cases:
        (bundle / "manifest.json").write_bytes(canonical(damaged))
        sign_unchecked(bundle, private, producer, "environment")
        env = offline_environment(sandbox / ("reject " + name), public)
        package(env, "import", "--bundle", bundle, ok=False)
        assert_passive(env)
        assert not list((Path(env["GRIPSACK_HOME"]) / "store").rglob("native-probe"))


def test_environment_import_rejects_missing_selected_member(sandbox, native_environment):
    bundle, _, public, private, producer, _ = native_environment
    manifest = json.loads((bundle / "manifest.json").read_bytes())
    manifest["roots"].append("missing-selected-package")
    (bundle / "manifest.json").write_bytes(canonical(manifest))
    sign_unchecked(bundle, private, producer, "environment")
    env = offline_environment(sandbox / "missing selected recipient", public)
    package(env, "import", "--bundle", bundle, ok=False)
    assert_passive(env)
    assert not list((Path(env["GRIPSACK_HOME"]) / "store").rglob("native-probe"))


@pytest.mark.native_integration
@pytest.mark.parametrize("runtime_kind", ["local", "external"])
def test_mixed_environment_roundtrip_consumes_real_python_wheel_native_profile_and_task(sandbox, runtime_kind):
    prepared = os.environ.get("GRIPSACK_TEST_CONDA_RUNTIME_BUNDLE")
    assert prepared, (
        "required independently prepared Python+wheel runtime bundle is absent; set "
        "GRIPSACK_TEST_CONDA_RUNTIME_BUNDLE=/tmp/conda-runtime-inputs/bundle after running "
        "python e2e/test_package_conda_offline.py prepare --directory /tmp/conda-runtime-inputs"
    )
    prepared = Path(prepared).resolve()
    assert prepared.is_dir(), f"required runtime bundle directory is absent: {prepared}"
    producer = sandbox / "mixed producer"
    producer_env = recipient(producer)
    private, seed, public = authority(producer / "environment authority")
    runtime_private, _, runtime_public = authority(producer / "runtime authority")
    source, native_original = native_source(producer)
    runtime_bundle = producer / "independent runtime bundle"
    shutil.copytree(prepared, runtime_bundle)
    runtime_manifest_bytes = (runtime_bundle / "manifest.json").read_bytes()
    runtime_manifest = json.loads(runtime_manifest_bytes)
    assert runtime_manifest["namespace"] == "fixture/conda"
    assert runtime_manifest["publisher"] == "fixture-publisher"
    assert runtime_manifest["layout"] == {"kind": "conda"}
    assert len(runtime_manifest["components"]) == 1
    conda = runtime_manifest["components"][0]["conda"]
    records = {record["name"]: record for record in conda["lock"]["packages"]}
    wheels = {record["name"]: record for record in conda["lock"]["wheels"]}
    assert "python" in records and records["wheel"]["noarch"] == "python"
    assert wheels["gripsack-offline-wheel"]["version"] == "1.0"
    assert conda["helper_notices"], "real pinned helper license/notice inputs are required"
    originals = tree_bytes(runtime_bundle / "blobs")
    assert originals and all(name == digest for name, digest in originals.items())
    for descriptor in [*conda["archives"], conda["helper"], *conda["helper_notices"].values()]:
        assert originals[descriptor["sha256"]] == descriptor["sha256"]
    independent_sign(runtime_bundle, runtime_private, producer)
    runtime_identity = sha(runtime_bundle / "manifest.json")
    trust_environment(producer_env, public, runtime_public)
    producer_runtime = json.loads(package(producer_env, "import", "--bundle", runtime_bundle).stdout)
    producer_prefix = Path(producer_runtime["payload"])

    spec = specification()
    runtime = {"id": "runtime", "payload": None, "tree": None, "runtime": [],
               "conda": None, "external": None, "inventory": []}
    if runtime_kind == "external":
        runtime["external"] = {"manifest": runtime_identity, "namespace": "fixture/conda",
                               "publisher": "fixture-publisher"}
    else:
        # Pack resolves these original archive/helper identities from durable
        # imported inputs. No installed prefix enters the native source tree.
        runtime["conda"] = copy.deepcopy(conda)
    spec["components"].append(runtime)
    spec["environment"]["CMAKE_PREFIX_PATH"] = {
        "kind": "artifact", "output": "runtime", "selector": ".",
    }
    # Deliberately not component-ID sort order: selection order is signed data.
    spec["roots"] = ["runtime", "native"]
    for name in ("python", "wheel", "offline-wheel"):
        spec["commands"][name] = {"component": "runtime", "executable": "bin/" + name, "args": []}
    bundle = sandbox / "mixed environment transfer"
    identity = pack_environment(producer_env, spec, source, bundle, seed)
    manifest_bytes = (bundle / "manifest.json").read_bytes()
    assert json.loads(manifest_bytes)["roots"] == ["runtime", "native"]
    if runtime_kind == "external":
        embedded = bundle / "manifests" / runtime_identity
        assert (embedded / "manifest.json").read_bytes() == runtime_manifest_bytes
    assert tree_bytes(runtime_bundle / "blobs") == originals
    assert tree_bytes(bundle / "blobs").items() >= originals.items()
    remove_producer(producer)
    assert not producer.exists() and not producer_prefix.exists()

    if runtime_kind == "external":
        # The root publisher cannot vouch for the independently named runtime.
        unauthorized = offline_environment(sandbox / "root only authority", public)
        package(unauthorized, "import", "--bundle", bundle, ok=False)
        assert_passive(unauthorized)
        missing = sandbox / "missing independently selected runtime"
        shutil.copytree(bundle, missing)
        shutil.rmtree(missing / "manifests" / runtime_identity)
        rejected = offline_environment(sandbox / "missing runtime recipient", public, runtime_public)
        package(rejected, "import", "--bundle", missing, ok=False)
        assert_passive(rejected)
        shutil.rmtree(missing)

    consumers = []
    transport = bundle
    for index, name in enumerate(("mixed recipient one", "mixed recipient two with a longer spaced home")):
        env = offline_environment(sandbox / name, public,
                                  runtime_public if runtime_kind == "external" else None)
        home = Path(env["HOME"])
        state = Path(env["GRIPSACK_HOME"])
        repo = consumer_workspace(home, identity, spec["commands"], mixed=True)
        facts = json.loads(package(env, "import", "--bundle", transport).stdout)
        assert facts["manifest"] == identity and facts["layout"] == "environment"
        assert Path(facts["payload"]).is_relative_to(state)
        expected_external = {runtime_identity} if runtime_kind == "external" else set()
        assert {child["manifest"] for child in facts["external"]} == expected_external
        assert_passive(env)
        # Inspect is read-only, including before any explicit managed acquisition.
        inventory = tree_bytes(state / "store")
        entries = sorted(str(path.relative_to(state / "store")) for path in (state / "store").rglob("*"))
        package(env, "inspect", "--manifest", identity)
        assert tree_bytes(state / "store") == inventory
        assert sorted(str(path.relative_to(state / "store")) for path in (state / "store").rglob("*")) == entries
        exported = sandbox / ("reexport " + str(index))
        package(env, "export", "--manifest", identity, "--output", exported)
        assert (exported / "manifest.json").read_bytes() == manifest_bytes
        if runtime_kind == "external":
            assert (exported / "manifests" / runtime_identity / "manifest.json").read_bytes() == runtime_manifest_bytes
        assert tree_bytes(exported / "blobs").items() >= originals.items()
        shutil.rmtree(transport)
        transport = exported
        consumers.append((env, repo))
    shutil.rmtree(transport)
    assert not bundle.exists() and not producer.exists()

    prefixes = []
    for env, repo in consumers:
        home = Path(env["HOME"])
        state = Path(env["GRIPSACK_HOME"])
        for cache in (Path(env["XDG_CACHE_HOME"]), state / "cache"):
            if cache.exists():
                remove_producer(cache)
            assert not cache.exists()
        project(env, repo, "gc")
        retained = {sha(path) for path in (state / "store").rglob("archive")}
        assert not producer.exists() and not bundle.exists() and not transport.exists()
        assert {item["sha256"] for item in conda["archives"]} <= retained
        assert any(sha(path) == conda["helper"]["sha256"]
                   for path in (state / "store").rglob("gripsack-conda"))
        assert any(path.read_bytes() == native_original for path in (state / "store").rglob("native-probe"))
        project(env, repo, "update")
        lock = (repo / "gripsack.lock").read_bytes()
        assert identity.encode() in lock
        result = project(env, repo, "run", "--env", "dev", "--", "python", "-I", "-B", "-c", PROBE)
        measured = json.loads(result.stdout)
        prefix = Path(measured["prefix"])
        prefixes.append(prefix)
        assert prefix.is_relative_to(state) and prefix != producer_prefix
        assert Path(measured["cmake"]) == prefix
        pkgconfig = Path(measured["pkgconfig"])
        assert pkgconfig.is_relative_to(state) and pkgconfig != producer / "source/components/native/share/pkgconfig"
        assert (pkgconfig / "fixture.pc").read_text() == "Name: passive-native\nVersion: 1.0\n"
        assert (pkgconfig.parents[1] / "bin/native-probe").read_bytes() == native_original
        assert measured["native"] == f"{LITERAL}\nrecipient-local\nfrom-manifest\nfrom-python\n"
        assert measured["portable"] == LITERAL and measured["local"] == "recipient-local"
        assert measured["wheel"] == "gripsack offline wheel 1.0"
        assert measured["noarch"] == f"wheel {records['wheel']['version']}"
        for module in ("ssl", "sqlite", "wheel_module"):
            assert Path(measured[module]).is_relative_to(prefix)
        for module in ("ssl", "sqlite"):
            assert Path(measured[module]).suffix == ".so"
        assert_passive(env)
        before = tree_bytes(prefix)
        marker = home / "explicit-task-ran"
        project(env, repo, "task", "run", "probe")
        assert marker.read_text() == "explicit task only\n"
        assert not (state / "current").exists()
        project(env, repo, "apply", "selected")
        profile = state / "current/env/profile.sh"
        assert profile.is_file()
        shell = ["/bin/sh", "-c", '. "$1"; exec python -I -B -c "$2"', "profile-fixture", profile, PROBE]
        profile_result = checked(shell, env=env, text=True)
        assert json.loads(profile_result.stdout) == measured
        assert tree_bytes(prefix) == before, "managed consumers modified the immutable Conda prefix"
        assert not list(prefix.rglob("*.pyc"))
        assert (repo / "gripsack.lock").read_bytes() == lock
        package(env, "inspect", "--manifest", identity)
        policy_path = state / "sharing/trust.json"
        policy = json.loads(policy_path.read_bytes())
        for key_id in (("environment", "fixture") if runtime_kind == "external" else ("environment",)):
            policy["revoked_keys"] = [key_id]
            policy_path.write_bytes(canonical(policy))
            blocked = project(env, repo, "run", "--env", "dev", "--", "python", "-I", "-B", "-c", PROBE, ok=False)
            assert "gripsack offline wheel 1.0" not in blocked.stdout
            marker.unlink()
            project(env, repo, "task", "run", "probe", ok=False)
            assert not marker.exists()
            profile_blocked = subprocess.run(list(map(str, shell)), env=env,
                                             capture_output=True, text=True, timeout=120)
            assert profile_blocked.returncode != 0
            assert LITERAL not in profile_blocked.stdout
            package(env, "inspect", "--manifest", identity, ok=False)
            policy["revoked_keys"] = []
            policy_path.write_bytes(canonical(policy))
            # Recreate the explicit task marker only by an authorized task run.
            project(env, repo, "task", "run", "probe")
        assert tree_bytes(prefix) == before
    assert len(set(prefixes)) == 2, "recipient Conda members must have distinct real final prefixes"
    assert all(prefix.exists() for prefix in prefixes)
    # The independently prepared input is read-only throughout the fixture.
    assert (prepared / "manifest.json").read_bytes() == runtime_manifest_bytes
    assert tree_bytes(prepared / "blobs") == originals
