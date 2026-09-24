#!/usr/bin/env python3
"""Independent OCI-layout verifier for B0-01 probe 4 (handover Epic B).

Deliberately shares no code with the Go bridge or buildkit: it parses
the exported tar itself, checks every blob digest against its
content-addressed path, walks index -> manifest -> config, extracts
layer contents, and — given two exports from two FRESH workers —
compares them for reproduction (content DiffIDs, normalized config,
extracted file digests). Cache reuse is never invoked: the driver
destroyed both workers and their cache volumes before this runs.
"""
from __future__ import annotations

import hashlib
import io
import json
import sys
import tarfile
from pathlib import Path

ALLOWED_LAYER_MEDIA = {
    "application/vnd.oci.image.layer.v1.tar+gzip",
    "application/vnd.oci.image.layer.v1.tar",
    "application/vnd.oci.image.layer.nondistributable.v1.tar+gzip",
}


def read_layout(tar_path: Path) -> dict:
    members = {}
    with tarfile.open(tar_path) as tf:
        for member in tf.getmembers():
            if not member.isfile():
                continue
            members[member.name] = tf.extractfile(member).read()
    report = {"tar": str(tar_path), "tar-bytes": tar_path.stat().st_size}
    for required in ("oci-layout", "index.json"):
        if required not in members:
            sys.exit(f"{tar_path}: missing {required}")
    if json.loads(members["oci-layout"]).get("imageLayoutVersion") != "1.0.0":
        sys.exit(f"{tar_path}: unexpected oci-layout version")
    report["layout-version"] = "1.0.0"

    index = json.loads(members["index.json"])
    manifests = [m for m in index.get("manifests", []) if m.get("mediaType", "").endswith("manifest.v1+json")]
    if len(manifests) != 1:
        sys.exit(f"{tar_path}: expected exactly one image manifest, got {len(manifests)}")
    descriptor = manifests[0]
    report["manifest-digest"] = descriptor["digest"]

    manifest_bytes = blob(members, descriptor["digest"])
    manifest = json.loads(manifest_bytes)
    config_descriptor = manifest["config"]
    if config_descriptor["mediaType"] != "application/vnd.oci.image.config.v1+json":
        sys.exit(f"{tar_path}: unexpected config media type {config_descriptor['mediaType']}")
    report["config-digest"] = config_descriptor["digest"]
    config = json.loads(blob(members, config_descriptor["digest"]))

    arch = config.get("architecture"), config.get("os")
    report["platform"] = f"{arch[1]}/{arch[0]}"
    diff_ids = config["rootfs"]["diff_ids"]
    report["diff-ids"] = diff_ids

    layers = manifest["layers"]
    if len(layers) != len(diff_ids):
        sys.exit(f"{tar_path}: {len(layers)} layers vs {len(diff_ids)} diff_ids")
    uncompressed = []
    for layer, diff_id in zip(layers, diff_ids):
        if layer["mediaType"] not in ALLOWED_LAYER_MEDIA:
            sys.exit(f"{tar_path}: unsupported layer media type {layer['mediaType']}")
        data = blob(members, layer["digest"])
        if layer["mediaType"].endswith("+gzip"):
            import gzip

            raw = gzip.decompress(data)
        else:
            raw = data
        actual = "sha256:" + hashlib.sha256(raw).hexdigest()
        if actual != diff_id:
            sys.exit(f"{tar_path}: DiffID mismatch {actual} != {diff_id}")
        uncompressed.append(raw)
    report["layer-digests"] = [layer["digest"] for layer in layers]

    # Extract the stacked filesystem (independent of any runtime).
    files: dict[str, str] = {}
    for raw in uncompressed:
        with tarfile.open(fileobj=io.BytesIO(raw)) as layer_tar:
            for member in layer_tar.getmembers():
                if member.isfile():
                    content = layer_tar.extractfile(member).read()
                    files[member.name] = hashlib.sha256(content).hexdigest()
    report["files"] = files
    report["config-normalized"] = normalize(config)
    return report


def blob(members: dict[str, bytes], digest: str) -> bytes:
    algo, _, hexdigest = digest.partition(":")
    path = f"blobs/{algo}/{hexdigest}"
    if path not in members:
        sys.exit(f"missing blob {path}")
    data = members[path]
    actual = f"{algo}:" + hashlib.new(algo, data).hexdigest()
    if actual != digest:
        sys.exit(f"blob digest mismatch at {path}: {actual} != {digest}")
    return data


def normalize(config: dict) -> dict:
    """Config minus fields the exporter may stamp with wall-clock time;
    reproduction must hold over everything semantic."""
    out = json.loads(json.dumps(config))
    out.pop("created", None)
    for entry in out.get("history", []):
        entry.pop("created", None)
    return out


def main() -> int:
    if len(sys.argv) < 2:
        sys.exit("usage: verify_oci.py LAYOUT.tar [LAYOUT2.tar]")
    first = read_layout(Path(sys.argv[1]))
    result = {"verdicts": [f"{first['tar']}: all blob digests, DiffIDs, media types and platform verified"]}
    if len(sys.argv) == 3:
        second = read_layout(Path(sys.argv[2]))
        reproduced = (
            first["diff-ids"] == second["diff-ids"]
            and first["config-normalized"] == second["config-normalized"]
            and first["files"] == second["files"]
        )
        result["reproduction"] = {
            "diff-ids-match": first["diff-ids"] == second["diff-ids"],
            "config-normalized-match": first["config-normalized"] == second["config-normalized"],
            "files-match": first["files"] == second["files"],
            "manifest-digests": [first["manifest-digest"], second["manifest-digest"]],
            "verdict": "reproduced from two clean workers" if reproduced else "DIFFERS between clean builds",
        }
        if not reproduced:
            sys.exit("clean-build reproduction FAILED")
        result["verdicts"].append("two fresh-worker exports reproduce (DiffIDs, normalized config, file digests)")
    summary = {
        "verifier": "independent python OCI parser (no buildkit code)",
        "first": {k: v for k, v in first.items() if k != "config-normalized"},
        "verdicts": result["verdicts"],
        **{k: v for k, v in result.items() if k != "verdicts"},
    }
    print(json.dumps(summary, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
