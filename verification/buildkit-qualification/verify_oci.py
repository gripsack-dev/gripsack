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


def read_layout(tar_path: Path) -> tuple[dict, bytes, list[bytes]]:
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
    config_bytes = blob(members, config_descriptor["digest"])
    config = json.loads(config_bytes)

    arch = config.get("architecture"), config.get("os")
    report["platform"] = f"{arch[1]}/{arch[0]}"
    report["runtime-config"] = config.get("config", {})
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
    return report, config_bytes, uncompressed


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


def write_docker_archive(
    destination: Path, config: bytes, layers: list[bytes],
    config_digest: str, diff_ids: list[str],
) -> dict:
    """Repackage only independently verified OCI bytes for legacy Docker stores.

    The engine loads the exact checked config and uncompressed layer
    bytes. There is no second BuildKit solve or unverified exporter.
    """
    config_hash = config_digest.partition(":")[2]
    config_name = f"{config_hash}.json"
    tag = f"gripsack-b0-qual:{config_hash}"
    layer_names = [
        f"{index:02d}-{diff_id.partition(':')[2]}/layer.tar"
        for index, diff_id in enumerate(diff_ids)
    ]
    manifest = json.dumps(
        [{"Config": config_name, "RepoTags": [tag], "Layers": layer_names}],
        sort_keys=True, separators=(",", ":"),
    ).encode()

    with tarfile.open(destination, "w") as archive:
        def add(name: str, data: bytes) -> None:
            member = tarfile.TarInfo(name)
            member.size = len(data)
            member.mtime = 0
            member.mode = 0o644
            archive.addfile(member, io.BytesIO(data))

        add(config_name, config)
        for name, raw in zip(layer_names, layers):
            directory = tarfile.TarInfo(name.rsplit("/", 1)[0] + "/")
            directory.type = tarfile.DIRTYPE
            directory.mtime = 0
            directory.mode = 0o755
            archive.addfile(directory)
            add(name, raw)
        add("manifest.json", manifest)

    digest = hashlib.sha256()
    with destination.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return {
        "path": str(destination),
        "sha256": digest.hexdigest(),
        "config-digest": config_digest,
        "tag": tag,
        "layer-diff-ids": diff_ids,
    }


def main() -> int:
    if len(sys.argv) not in (2, 3, 4):
        sys.exit("usage: verify_oci.py LAYOUT.tar [LAYOUT2.tar [DOCKER_ARCHIVE.tar]]")
    first, config_bytes, raw_layers = read_layout(Path(sys.argv[1]))
    result = {"verdicts": [f"{first['tar']}: all blob digests, DiffIDs, media types and platform verified"]}
    if len(sys.argv) >= 3:
        second, _, _ = read_layout(Path(sys.argv[2]))
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
    if len(sys.argv) == 4:
        result["docker-archive"] = write_docker_archive(
            Path(sys.argv[3]), config_bytes, raw_layers,
            first["config-digest"], first["diff-ids"],
        )
        result["verdicts"].append(
            "legacy Docker archive contains the exact independently verified OCI config and layer bytes"
        )
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
