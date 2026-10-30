"""Independent signed package DAGs through real offline/OCI CLI boundaries."""
from __future__ import annotations

import base64
import json
import os
from pathlib import Path
import shutil

import pytest

from test_package_sharing import (
    assert_native, canonical, checked, imported, native_target, package,
    published, recipient, trust,
)


def writable_remove(path):
    for directory in [path, *path.rglob("*")]:
        if directory.is_dir() and not directory.is_symlink():
            directory.chmod(0o700)
    shutil.rmtree(path)


def root_policy(env, child_public, parent_public):
    trust(env, child_public)
    path = Path(env["GRIPSACK_HOME"]) / "sharing/trust.json"
    policy = json.loads(path.read_bytes())
    policy["keys"].append({
        "key_id": "application", "publisher": "application-publisher",
        "namespaces": ["fixture/application"], "public_key": parent_public,
        "allowed_layouts": ["relocatable"],
        "required_validators": ["manifest-integrity", "payload-integrity", "layout-safety"],
    })
    path.write_bytes(canonical(policy))
    return path


def resign(env, bundle, key):
    output = bundle / "replacement-envelope.json"
    package(env, "sign", "--manifest", bundle / "manifest.json", "--key", key,
            "--key-id", "application", "--output", output)
    output.replace(bundle / "envelope.json")


@pytest.fixture
def external_bundle(sandbox, published):
    child_bundle, child_public, child_digest, _, _ = published
    producer = sandbox / "external-publisher"
    env = recipient(producer)
    trust(env, child_public)
    imported(env, child_bundle, child_digest)
    private = producer / "private.pem"
    checked(["openssl", "genpkey", "-algorithm", "ED25519", "-out", private])
    private_der = producer / "private.der"
    public_der = producer / "public.der"
    checked(["openssl", "pkey", "-in", private, "-outform", "DER", "-out", private_der])
    checked(["openssl", "pkey", "-in", private, "-pubout", "-outform", "DER", "-out", public_der])
    raw = private_der.read_bytes()
    assert raw[:16] == bytes.fromhex("302e020100300506032b657004220420") and len(raw) == 48
    seed = sandbox / "application-seed"
    seed.write_bytes(raw[-32:])
    seed.chmod(0o600)
    parent_public = base64.b64encode(public_der.read_bytes()[-32:]).decode()
    spec = {
        "version": 1, "namespace": "fixture/application", "publisher": "application-publisher",
        "target": native_target(), "layout": {"kind": "relocatable", "contract": {
            "behavior": "Complete independently signed runtime tool is embedded without byte rewriting"}},
        "components": [{"id": "dependency", "payload": None, "tree": None, "runtime": [],
                        "conda": None, "external": {"manifest": child_digest,
                        "namespace": "fixture/native", "publisher": "fixture-publisher"}, "inventory": []}],
        "roots": ["dependency"],
        "commands": {"static": {"component": "dependency", "executable": "components/native/bin/static", "args": []}},
        "environment": {}, "licenses": [], "redistribution_authorized": True,
    }
    specification = producer / "specification.json"
    specification.write_bytes(canonical(spec))
    source = producer / "empty-local-source"
    source.mkdir()
    bundle = sandbox / "external-bundle"
    result = json.loads(package(env, "pack", "--manifest", specification, "--source", source,
                                "--output", bundle, "--key", seed, "--key-id", "application").stdout)
    assert (bundle / "manifests" / child_digest / "manifest.json").read_bytes() == (child_bundle / "manifest.json").read_bytes()
    # Only the complete offline closure and independently configured keys remain.
    writable_remove(producer)
    shutil.rmtree(child_bundle)
    return bundle, child_public, parent_public, result["manifest"], child_digest, seed


def test_external_closure_roundtrip_has_independent_authority_and_retention(sandbox, external_bundle):
    bundle, child_public, parent_public, digest, child_digest, _ = external_bundle
    first = recipient(sandbox / "first external recipient")
    root_policy(first, child_public, parent_public)
    payload = imported(first, bundle, digest)
    assert_native(payload / "components/dependency", first)
    # Embedded dependency bytes do not implicitly register a standalone package.
    package(first, "inspect", "--manifest", child_digest, ok=False)
    exported = sandbox / "exported-external"
    package(first, "export", "--manifest", digest, "--output", exported)
    assert (exported / "manifests" / child_digest / "manifest.json").read_bytes() == (bundle / "manifests" / child_digest / "manifest.json").read_bytes()
    writable_remove(Path(first["HOME"]))
    shutil.rmtree(bundle)
    second = recipient(sandbox / "second external recipient with spaces")
    root_policy(second, child_public, parent_public)
    received = imported(second, exported, digest)
    shutil.rmtree(exported)
    assert_native(received / "components/dependency", second)
    package(second, "inspect", "--manifest", digest)


@pytest.mark.parametrize("damage", ["missing-manifest", "blob", "wrong-namespace", "root-kernel", "child-key-revoked"])
def test_external_closure_rejects_missing_tampered_or_disallowed_members(sandbox, external_bundle, damage):
    bundle, child_public, parent_public, digest, child_digest, seed = external_bundle
    env = recipient(sandbox / "rejecting-external-recipient")
    policy_path = root_policy(env, child_public, parent_public)
    if damage == "missing-manifest":
        shutil.rmtree(bundle / "manifests" / child_digest)
    elif damage == "blob":
        blob = next((bundle / "blobs").iterdir())
        blob.write_bytes(blob.read_bytes() + b"tamper")
    elif damage == "child-key-revoked":
        imported(env, bundle, digest)
        policy = json.loads(policy_path.read_bytes())
        policy["revoked_keys"] = ["fixture"]
        policy_path.write_bytes(canonical(policy))
        package(env, "inspect", "--manifest", digest, ok=False)
    else:
        manifest = json.loads((bundle / "manifest.json").read_bytes())
        if damage == "wrong-namespace":
            manifest["components"][0]["external"]["namespace"] = "fixture/application"
        else:
            manifest["target"]["minimum_kernel"] = "9999.0"
        (bundle / "manifest.json").write_bytes(canonical(manifest))
        resign(env, bundle, seed)
    package(env, "import", "--bundle", bundle, ok=False)
    assert not (Path(env["GRIPSACK_HOME"]) / "current").exists()


@pytest.mark.native_integration
def test_registry_carries_complete_independently_signed_external_closure(sandbox, external_bundle):
    url = os.environ.get("GRIPSACK_TEST_PACKAGE_REGISTRY")
    assert url, "explicit local registry fixture is required when selecting native_integration"
    bundle, child_public, parent_public, digest, child_digest, _ = external_bundle
    sender = recipient(sandbox / "external-registry-sender")
    root_policy(sender, child_public, parent_public)
    imported(sender, bundle, digest)
    repository = "gripsack-external/" + digest
    pushed = json.loads(package(sender, "push", "--manifest", digest, "--registry", url,
                                "--repository", repository, "--reference", "fixture",
                                "--insecure-loopback").stdout)
    writable_remove(Path(sender["HOME"]))
    shutil.rmtree(bundle)
    receiver = recipient(sandbox / "external-registry-receiver")
    root_policy(receiver, child_public, parent_public)
    received = json.loads(package(receiver, "pull", "--digest", pushed["transport_digest"],
                                  "--registry", url, "--repository", repository,
                                  "--insecure-loopback").stdout)
    assert received["manifest"] == digest
    assert_native(Path(received["payload"]) / "components/dependency", receiver)
    package(receiver, "inspect", "--manifest", child_digest, ok=False)
