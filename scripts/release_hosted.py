#!/usr/bin/env python3
"""Hosted approval/publication only: never builds or executes candidate contents."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import tempfile

import release_consumer as consumer
import release_manifest as schema
from release_admission import fetch_status


def gh(*args):
    return consumer.command(["gh", *map(str, args)])


def manifest_path(root):
    paths = list(root.glob("gripsack-release-*.json"))
    consumer.require(len(paths) == 1, "candidate needs exactly one release manifest")
    return paths[0]


def hosted_assets(release):
    pages = json.loads(gh("api", f"repos/{consumer.REPOSITORY}/releases/{release['id']}/assets?per_page=100",
                          "--paginate", "--slurp"))
    assets = [item for page in pages for item in page]
    names = [item["name"] for item in assets]
    consumer.require(len(names) == len(set(names)), "duplicate release asset name")
    return {item["name"]: item for item in assets}


def admit(args):
    identity = {key: os.environ[key] for key in (
        "BUNDLE_URL", "BUNDLE_SHA256", "MANIFEST_SHA256", "SOURCE_COMMIT",
        "RELEASE_TAG", "CANDIDATE_ID", "REQUEST_ID")}
    for key in ("BUNDLE_SHA256", "MANIFEST_SHA256", "CANDIDATE_ID", "REQUEST_ID"):
        consumer.require(re.fullmatch(r"[0-9a-f]{64}", identity[key]) is not None, f"invalid {key}")
    consumer.require(re.fullmatch(r"[0-9a-f]{40}", identity["SOURCE_COMMIT"]) is not None,
                     "invalid source commit")
    consumer.require(consumer.stable_version(identity["RELEASE_TAG"]) is not None, "invalid release tag")
    consumer.require(os.environ["GITHUB_REPOSITORY"] == consumer.REPOSITORY
                     and os.environ["GITHUB_REF"] == f"refs/tags/{identity['RELEASE_TAG']}"
                     and os.environ["GITHUB_SHA"] == identity["SOURCE_COMMIT"],
                     "dispatch must execute the nominated immutable source tag")
    from release_operator import request_id
    binding = {"repository": consumer.REPOSITORY, "workflow": consumer.WORKFLOW,
               "bundle_url": identity["BUNDLE_URL"], "bundle_sha256": identity["BUNDLE_SHA256"],
               "manifest_sha256": identity["MANIFEST_SHA256"], "source_commit": identity["SOURCE_COMMIT"],
               "tag": identity["RELEASE_TAG"], "candidate_id": identity["CANDIDATE_ID"]}
    consumer.require(request_id(binding) == identity["REQUEST_ID"], "release request identity mismatch")
    # Independent authenticated controller witness, not portable bundle claims.
    # Exact-source API order matters: a superseding failure must block signing.
    fetch_status(binding, lambda endpoint: json.loads(gh("api", "--hostname", "github.com", endpoint)))
    consumer.download(identity["BUNDLE_URL"], args.archive, schema.MAX_BUNDLE_BYTES)
    consumer.command(["python3", "scripts/release_manifest.py", "extract", "--bundle", str(args.archive),
                      "--sha256", identity["BUNDLE_SHA256"], "--destination", str(args.bundle)])
    manifest = schema.validate_bundle(args.bundle, source=Path.cwd())
    path = manifest_path(args.bundle)
    consumer.require(schema.digest(path) == identity["MANIFEST_SHA256"]
                     and manifest["source"]["commit"] == identity["SOURCE_COMMIT"]
                     and manifest["candidate_id"] == identity["CANDIDATE_ID"]
                     and manifest["signer"]["ref"] == f"refs/tags/{identity['RELEASE_TAG']}",
                     "nominated candidate identity mismatch")
    # Re-fetch immediately before making the signing predicate available.
    fetch_status(binding, lambda endpoint: json.loads(gh("api", "--hostname", "github.com", endpoint)))
    args.predicate.write_text(json.dumps(consumer.expected_predicate(manifest), sort_keys=True) + "\n")
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
        output.write(f"manifest={path}\nmanifest_name={path.name}\n")


def asset_bytes(tag, name, directory):
    gh("release", "download", tag, "--repo", consumer.REPOSITORY,
       "--pattern", name, "--dir", directory)
    return Path(directory) / name


def publish(args):
    manifest = schema.validate_bundle(args.bundle, source=Path.cwd())
    path = manifest_path(args.bundle)
    tag = manifest["signer"]["ref"].removeprefix("refs/tags/")
    result = schema.load_json(args.publication)
    consumer.require(result.get("status") == "publication_complete", "registry publication is incomplete")
    # The registry publisher re-downloads all payload identities on every resume.
    # Never infer publication success merely from an existing local result file.
    consumer.require(result.get("identity") == {
        "manifest_sha256": schema.digest(path), "source": manifest["source"],
        "candidate_id": manifest["candidate_id"]},
        "registry publication belongs to another manifest")
    expected_packages = [
        {key: package[key] for key in ("registry", "name", "version", "size_bytes")}
        | {"remote_sha256": package["sha256"]} for package in manifest["packages"]]
    consumer.require(result.get("packages") == expected_packages
                     and result.get("signed_release_complete") is False,
                     "registry result does not reconcile the complete package set")
    signature_name = f"{path.name}.sigstore.json"
    consumer.verify_attestation(path, manifest, args.signature)
    expected = {Path(item["path"]).name: (args.bundle / item["path"], item["sha256"])
                for item in manifest["artifacts"]}
    expected[path.name] = (path, schema.digest(path))
    pages = json.loads(gh("api", f"repos/{consumer.REPOSITORY}/releases?per_page=100", "--paginate", "--slurp"))
    listed = [item for page in pages for item in page]
    existing = [item for item in listed if item["tag_name"] == tag]
    consumer.require(len(existing) <= 1, "duplicate release identity")
    if not existing:
        gh("release", "create", tag, "--repo", consumer.REPOSITORY, "--verify-tag", "--draft",
           "--title", f"gripsack {manifest['compatible']['core']}",
           "--notes", "Locally built and qualified. GitHub approves and signs the exact release manifest; no hosted compilation is claimed.")
    release = json.loads(gh("api", f"repos/{consumer.REPOSITORY}/releases/tags/{tag}"))
    assets = hosted_assets(release)
    consumer.require(set(assets) <= set(expected) | {signature_name}, "unexpected existing release asset")
    with tempfile.TemporaryDirectory(prefix="gripsack-reconcile-") as directory:
        for name, (local, digest) in expected.items():
            if name in assets:
                remote = asset_bytes(tag, name, directory)
                consumer.require(schema.digest(remote) == digest, f"already-existing asset identity differs: {name}")
            else:
                consumer.require(release["draft"], "published release is incomplete; refusing mutation")
                gh("release", "upload", tag, local, "--repo", consumer.REPOSITORY)
        if signature_name in assets:
            remote = asset_bytes(tag, signature_name, directory)
            consumer.verify_attestation(path, manifest, remote)
        else:
            consumer.require(release["draft"], "published release has no attestation bundle")
            signature = Path(directory) / signature_name
            shutil.copyfile(args.signature, signature)
            gh("release", "upload", tag, signature, "--repo", consumer.REPOSITORY)
    release = json.loads(gh("api", f"repos/{consumer.REPOSITORY}/releases/tags/{tag}"))
    consumer.require(set(hosted_assets(release)) == set(expected) | {signature_name},
                     "final release asset inventory differs")
    # Read back every upload before the public commit point.
    with tempfile.TemporaryDirectory(prefix="gripsack-readback-") as directory:
        for name, (_, digest) in expected.items():
            consumer.require(schema.digest(asset_bytes(tag, name, directory)) == digest,
                             f"uploaded asset differs: {name}")
        consumer.verify_attestation(path, manifest, asset_bytes(tag, signature_name, directory))
    gh("release", "edit", tag, "--repo", consumer.REPOSITORY, "--draft=false", "--latest")
    print(json.dumps({"status": "signed_published_pending_consumer_verification",
                      "manifest_sha256": schema.digest(path), "tag": tag}, sort_keys=True))


def verify(args):
    manifest = schema.validate_bundle(args.bundle, source=Path.cwd())
    consumer.verifier()
    consumer.verify_attestation(manifest_path(args.bundle), manifest, args.signature)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    admission = sub.add_parser("admit")
    admission.add_argument("--archive", type=Path, required=True)
    admission.add_argument("--bundle", type=Path, required=True)
    admission.add_argument("--predicate", type=Path, required=True)
    verification = sub.add_parser("verify")
    verification.add_argument("--bundle", type=Path, required=True)
    verification.add_argument("--signature", type=Path, required=True)
    publication = sub.add_parser("publish")
    publication.add_argument("--bundle", type=Path, required=True)
    publication.add_argument("--publication", type=Path, required=True)
    publication.add_argument("--signature", type=Path, required=True)
    args = parser.parse_args()
    {"admit": admit, "verify": verify, "publish": publish}[args.command](args)


if __name__ == "__main__":
    main()
