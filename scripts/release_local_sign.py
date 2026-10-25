#!/usr/bin/env python3
"""Owner-key offline signing of an already locally qualified release bundle.

Runs only on the dedicated signer host. The private cosign key is provisioned
there (--key or GRIPSACK_SIGNER_KEY) and never enters the repository or the
bundle. Signing admits nothing new: only a bundle that already passes full
local qualification admission (release_manifest.validate_bundle) is signed,
and the nominated identity (manifest sha256, source commit, tag, candidate id)
must match that bundle exactly, mirroring the hosted admission contract.

The emitted gripsack.local-signature.v1 document is deterministic (canonical
JSON, no timestamps): the same key, bundle and cosign build produce the same
bytes. Consumers verify it with release_consumer.verify_local_signature.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

import release_consumer as consumer
import release_manifest as schema

require = schema.require


def cosign(*args):
    return consumer.command(["cosign", *map(str, args)], maximum=64 * 1024)


def signer_key_path(args):
    named = args.key or os.environ.get("GRIPSACK_SIGNER_KEY", "")
    require(named, "name the signer private key via --key or GRIPSACK_SIGNER_KEY")
    require(Path(named).is_absolute(), "signer private key path must be absolute")
    path = schema.regular(Path(named))
    bundle = args.bundle.resolve()
    require(not path.is_relative_to(bundle), "signer private key must not live inside the release bundle")
    return path


def nominated_identity(manifest, manifest_path):
    return {"manifest_sha256": schema.digest(manifest_path),
            "source_commit": manifest["source"]["commit"],
            "tag": manifest["signer"]["ref"].removeprefix("refs/tags/"),
            "candidate_id": manifest["candidate_id"]}


def sign(args):
    # Refuse dirty or unadmitted inputs: only a fully qualified, internally
    # consistent bundle reaches the private key.
    manifest = schema.validate_bundle(args.bundle)
    name = f"gripsack-release-{manifest['compatible']['core']}.json"
    manifest_path = args.bundle / name
    identity = nominated_identity(manifest, manifest_path)
    nominated = {"manifest_sha256": args.manifest_sha256, "source_commit": args.source_commit,
                 "tag": args.tag, "candidate_id": args.candidate_id}
    require(nominated == identity, "nominated identity differs from the admitted bundle")
    key = signer_key_path(args)
    public_key = cosign("public-key", "--key", key).strip()
    fingerprint = schema.key_fingerprint(public_key)
    payload = schema.signature_payload(manifest, identity["manifest_sha256"], fingerprint)
    with tempfile.TemporaryDirectory(prefix="gripsack-local-sign-") as directory:
        blob = Path(directory) / "payload.json"
        blob.write_bytes(schema.canonical(payload))
        signature = cosign("sign-blob", "--yes", "--tlog-upload=false", "--key", key, blob).strip()
    document = {"format": schema.LOCAL_SIGNATURE_FORMAT, "payload": payload,
                "public_key": public_key, "key_fingerprint": fingerprint, "signature": signature}
    output = args.out or Path.cwd() / f"{name}{consumer.LOCAL_SIGNATURE_SUFFIX}"
    require(output.parent.is_dir() and not output.parent.is_symlink()
            and not output.is_symlink() and (not output.exists() or output.is_file()),
            "signature output path is not a regular file path")
    require(not output.resolve().is_relative_to(args.bundle.resolve()),
            "signature document must not be written inside the release bundle")
    output.write_bytes(schema.canonical(schema.validate_signature_document(document)) + b"\n")
    # Self-check the emitted document against the actual key before handoff.
    consumer.verify_local_signature(manifest_path, manifest, output, owner_key=public_key)
    print(json.dumps({"status": "signed_local_owner_key", "manifest_sha256": identity["manifest_sha256"],
                      "tag": identity["tag"], "key_fingerprint": fingerprint,
                      "signature_document": str(output)}, sort_keys=True))
    return output


def verify(args):
    manifest_path = schema.regular(args.manifest)
    manifest = schema.validate_manifest(schema.load_json(manifest_path))
    consumer.local_verifier()
    document = consumer.verify_local_signature(manifest_path, manifest, args.signature)
    print(json.dumps({"status": "verified_local_owner_key",
                      "manifest_sha256": document["payload"]["manifest_sha256"],
                      "key_fingerprint": document["payload"]["key_fingerprint"]}, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    signing = sub.add_parser("sign")
    signing.add_argument("--bundle", type=Path, required=True, help="admitted extracted release bundle directory")
    signing.add_argument("--key", type=Path, help="signer private key path (or GRIPSACK_SIGNER_KEY)")
    signing.add_argument("--manifest-sha256", required=True)
    signing.add_argument("--source-commit", required=True)
    signing.add_argument("--tag", required=True)
    signing.add_argument("--candidate-id", required=True)
    signing.add_argument("--out", type=Path, help="output document path (default: beside the caller)")
    checking = sub.add_parser("verify")
    checking.add_argument("--manifest", type=Path, required=True)
    checking.add_argument("--signature", type=Path, required=True)
    args = parser.parse_args()
    try:
        {"sign": sign, "verify": verify}[args.command](args)
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        raise SystemExit(f"gripsack local signer: {error}\n") from error

if __name__ == "__main__":
    main()
