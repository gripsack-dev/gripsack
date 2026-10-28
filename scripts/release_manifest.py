#!/usr/bin/env python3
"""Bounded admission for locally qualified releases; never a signature verifier.

The owner-approved signer attests this manifest, not a fictional hosted build.
Bundle validation checks internal consistency, not trusted controller authority.
The independent local-qualification witness admits original private receipts.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tarfile
import tomllib

FORMAT = "gripsack.release.v1"
REPOSITORY = "gripsack-dev/gripsack"
POLICY = "LOCAL-PRACTICAL-QUALIFICATION-2026-10-27"
WORKFLOW = ".github/workflows/release-core.yml"
TARGET = "x86_64-unknown-linux-musl"
MAX_JSON_BYTES = 4 * 1024 * 1024
MAX_JSON_DEPTH = 32
MAX_FILES = 4096
MAX_FILE_BYTES = 1024 * 1024 * 1024
MAX_BUNDLE_BYTES = 2 * 1024 * 1024 * 1024
# Private controller image outputs are references, not shipped bundle members.
MAX_OBSERVED_OUTPUT_BYTES = (1 << 63) - 1
EXCLUSIONS = ["REL-FUZZ-2026-09-27", "linux-aarch64", "macos", "native-windows",
              "hardware-power-loss", "github-hosted-build", "REL-PRACTICAL-050-2026-10-27",
              "formal-tlc", "formal-tlaps", "formal-verus"]
REQUIRED_GATES = frozenset((
    "inventory", "test", "gnu-test", "ts-test", "runtime-images",
    "dependency-audits", "bridge-repro-pins", "b0-linux", "helper-repro-pins",
    "core-package-static-sbom", "flows", "candidate-sdk", "external-source-pins",
    "installed-consumers-docs", "core-repro",
    "persistence-96", "publication-packages",
))
HEX64 = re.compile(r"[0-9a-f]{64}")
HEX40 = re.compile(r"[0-9a-f]{40}")
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+")
IDENTIFIER = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,199}")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def keys(value, expected):
    require(isinstance(value, dict) and set(value) == set(expected), "unexpected or missing object fields")


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON field")
        result[key] = value
    return result


def decode_json(data):
    require(len(data) <= MAX_JSON_BYTES, "JSON exceeds byte bound")
    try:
        value = json.loads(data, object_pairs_hook=unique_object,
                           parse_constant=lambda _: require(False, "nonfinite JSON"))
    except (RecursionError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("invalid bounded JSON") from error
    def visit(item, depth):
        require(depth <= MAX_JSON_DEPTH, "JSON exceeds depth bound")
        if isinstance(item, dict):
            for key, child in item.items():
                require(len(key) <= 1024, "oversized JSON key")
                visit(child, depth + 1)
        elif isinstance(item, list):
            require(len(item) <= MAX_FILES, "JSON exceeds list bound")
            for child in item:
                visit(child, depth + 1)
        elif isinstance(item, str):
            require(len(item) <= MAX_JSON_BYTES, "oversized JSON string")
    visit(value, 0)
    return value


def safe_path(value):
    require(isinstance(value, str) and 0 < len(value) <= 512, "invalid path length")
    require(not value.startswith("/") and "\\" not in value
            and all(32 <= ord(c) < 127 for c in value)
            and all(p not in ("", ".", "..") for p in value.split("/")), "unsafe archive path")
    return value


def regular(path):
    path = Path(path).absolute()
    require(".." not in path.parts, "parent path traversal")
    for parent in [*reversed(path.parents), path]:
        require(not parent.is_symlink(), "symlink in release path")
    require(stat.S_ISREG(path.stat().st_mode), "release input is not a regular file")
    return path


def load_json(path):
    with regular(path).open("rb") as stream:
        return decode_json(stream.read(MAX_JSON_BYTES + 1))


def digest(path):
    with regular(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def file_record(path, relative):
    return {"path": safe_path(relative), "size_bytes": regular(path).stat().st_size, "sha256": digest(path)}


def validate_record(item, extra=(), *, maximum=MAX_FILE_BYTES):
    keys(item, {"path", "size_bytes", "sha256", *extra})
    safe_path(item["path"])
    require(type(item["size_bytes"]) is int and 0 < item["size_bytes"] <= maximum, "invalid artifact byte size")
    require(isinstance(item["sha256"], str) and HEX64.fullmatch(item["sha256"]), "invalid artifact digest")


def unique(items, field):
    require(isinstance(items, list) and len(items) <= MAX_FILES, "invalid inventory")
    values = [item[field] for item in items]
    require(len(values) == len(set(values)), f"duplicate {field} inventory")
    return set(values)


def package_order(packages):
    names = unique(packages, "name")
    emitted = set()
    for package in packages:
        dependencies = package["dependencies"]
        require(isinstance(dependencies, list) and len(dependencies) == len(set(dependencies)), "duplicate package dependency")
        require(set(dependencies) <= names and set(dependencies) <= emitted, "package DAG is not dependency ordered")
        emitted.add(package["name"])


def validate_manifest(value):
    keys(value, {"format", "repository", "policy", "source", "candidate_id", "qualification_sha256",
                 "recipe_sha256", "platform", "signer", "compatible", "gates", "partitions", "exclusions",
                 "provenance", "artifacts", "packages", "evidence"})
    require(value["format"] == FORMAT and value["repository"] == REPOSITORY
            and value["policy"] == POLICY and value["platform"] == "linux-x86_64", "unsupported release authority/platform")
    keys(value["source"], {"commit", "tree"})
    for identity in value["source"].values():
        require(isinstance(identity, str) and HEX40.fullmatch(identity), "invalid source identity")
    for field in ("candidate_id", "qualification_sha256", "recipe_sha256"):
        require(isinstance(value[field], str) and HEX64.fullmatch(value[field]), f"invalid {field}")
    compatible = value["compatible"]
    keys(compatible, {"core", "sdk", "bridge", "conda", "ir", "helper_pins"})
    for component in ("core", "sdk", "bridge", "conda"):
        require(isinstance(compatible[component], str) and VERSION.fullmatch(compatible[component]), "invalid component version")
    require(compatible["core"] == compatible["bridge"] == compatible["conda"], "helper version differs from core")
    keys(compatible["ir"], {"writer", "readers"})
    require(compatible["ir"] == {"writer": 9, "readers": [9]}, "unsupported IR compatibility contract")
    keys(compatible["helper_pins"], {"bridge", "conda"})
    for pin in compatible["helper_pins"].values():
        require(isinstance(pin, str) and HEX64.fullmatch(pin), "invalid helper pin")
    require(value["signer"] == {"workflow": WORKFLOW, "ref": "refs/tags/core-v" + compatible["core"]}, "wrong signer workflow/ref")
    require(value["provenance"] == {"builder": "local", "hosted_build": False}
            and value["provenance"]["hosted_build"] is False, "false hosted provenance")
    require(value["exclusions"] == EXCLUSIONS, "changed owner exclusions")
    for inventory, fields, digest_field in (("gates", {"id", "receipt_sha256"}, "receipt_sha256"),
                                             ("partitions", {"id", "sha256"}, "sha256")):
        require(value[inventory], "empty required coverage")
        for item in value[inventory]:
            keys(item, fields)
            require(isinstance(item["id"], str) and IDENTIFIER.fullmatch(item["id"]), "invalid coverage id")
            require(isinstance(item[digest_field], str) and HEX64.fullmatch(item[digest_field]), "invalid evidence digest")
        unique(value[inventory], "id")
    require({gate["id"] for gate in value["gates"]} == REQUIRED_GATES, "missing or foreign required gate")
    expected_partitions = {f"Linux-{scenario}-{fault}-{shard}"
                           for scenario in ("apply-deploy", "apply-prune", "rollback-deploy",
                                            "rollback-prune", "apply-deploy-copy", "apply-prune-copy")
                           for fault in ("error", "kill") for shard in range(8)}
    require({part["id"] for part in value["partitions"]} == expected_partitions, "missing or foreign partition")
    all_paths = []
    for inventory, extra, prefix in (("artifacts", ("role",), "artifacts/"),
                                      ("packages", ("registry", "name", "version", "dependencies", "metadata"), "packages/"),
                                      ("evidence", (), "evidence/")):
        require(value[inventory], "empty release file inventory")
        for item in value[inventory]:
            validate_record(item, extra)
            require(item["path"].startswith(prefix), "wrong inventory path prefix")
            all_paths.append(item["path"])
        unique(value[inventory], "path")
    require(len(all_paths) == len(set(all_paths)), "duplicate release path")
    require(sum(item["size_bytes"] for key in ("artifacts", "packages", "evidence") for item in value[key]) <= MAX_BUNDLE_BYTES,
            "release exceeds total byte bound")
    roles = {}
    for artifact in value["artifacts"]:
        require(artifact["path"].count("/") == 1, "public artifact must have one basename")
        manifest_name = f"gripsack-release-{compatible['core']}.json"
        require(Path(artifact["path"]).name not in (manifest_name, manifest_name + ".sigstore.json"),
                "artifact shadows reserved manifest/signature name")
        require(artifact["role"] in ("core", "bridge", "conda", "auxiliary"), "unknown artifact role")
        if artifact["role"] != "auxiliary":
            require(artifact["role"] not in roles, "duplicate installable artifact")
            roles[artifact["role"]] = artifact
    require(set(roles) == {"core", "bridge", "conda"}, "missing compatible installable artifact")
    for role, stem, suffix in (("core", "gripsack", ".tar.gz"), ("bridge", "grip-buildkit-bridge", ""), ("conda", "gripsack-conda", "")):
        require(roles[role]["path"] == f"artifacts/{stem}-{compatible[role]}-{TARGET}{suffix}", "artifact version/target mismatch")
        if role != "core":
            require(roles[role]["sha256"] == compatible["helper_pins"][role], "wrong helper digest pin")
    evidence_paths = {item["path"] for item in value["evidence"]}
    for package in value["packages"]:
        require(package["registry"] in ("crates.io", "npm"), "unsupported package registry")
        require(isinstance(package["name"], str) and re.fullmatch(r"(?:@[a-z0-9_-]+/)?[a-z0-9_-]+", package["name"]), "invalid package name")
        expected_version = compatible["sdk"] if package["registry"] == "npm" else compatible["core"]
        require(package["version"] == expected_version, "wrong SDK/crate version")
        require(package["metadata"] in evidence_paths, "missing publication metadata")
        extension = ".tgz" if package["registry"] == "npm" else ".crate"
        require(package["path"].endswith(extension), "wrong publication payload format")
    require([p["name"] for p in value["packages"] if p["registry"] == "npm"] == ["@gripsack/core"], "missing or foreign SDK")
    package_order(value["packages"])
    return value


def git(source, *args):
    environment = {"PATH": os.defpath, "HOME": "/nonexistent", "LANG": "C", "GIT_CONFIG_GLOBAL": os.devnull,
                   "GIT_CONFIG_NOSYSTEM": "1", "GIT_NO_REPLACE_OBJECTS": "1", "GIT_NO_LAZY_FETCH": "1"}
    return subprocess.check_output(["git", "-C", str(source), *args], env=environment, timeout=60).decode().strip()


def clean_source(source):
    require(not git(source, "status", "--porcelain=v1", "--untracked-files=all", "--ignore-submodules=none"), "source dirty")
    require(all(not line.startswith("S ") and not line[:1].islower() for line in git(source, "ls-files", "-v").splitlines()), "hidden index changes")
    return {"commit": git(source, "rev-parse", "HEAD^{commit}"), "tree": git(source, "rev-parse", "HEAD^{tree}")}


def compatible_source(source):
    source = Path(source)
    core = tomllib.loads((source / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    sdk = load_json(source / "typescript/package.json")["version"]
    reader = (source / "crates/gripsack-ir/src/parse.rs").read_text()
    writer = (source / "typescript/src/workspace/emit.ts").read_text()
    require(re.search(r"pub const IR_VERSION: u32 = 9;", reader)
            and "version != u64::from(IR_VERSION)" in reader
            and re.search(r"export const IR_VERSION = 9;", writer), "changed IR reader/writer contract")
    result = {"core": core, "sdk": sdk, "ir": {"writer": 9, "readers": [9]}, "helper_pins": {}}
    for component in ("bridge", "conda"):
        text = (source / f"crates/gripsack-fetch/src/{component}_pins.rs").read_text()
        version = re.search(rf'{component.upper()}_VERSION: &str = "([^"]+)"', text)
        pin = re.search(r'AssetTarget::LinuxX86_64Musl,\s*"([a-f0-9]{64})"', text)
        require(version is not None and pin is not None, "missing compiled helper pin")
        result[component] = version.group(1)
        result["helper_pins"][component] = pin.group(1)
    return result


def verify_file(root, item):
    path = regular(Path(root) / item["path"])
    require(path.stat().st_size == item["size_bytes"] and digest(path) == item["sha256"], "changed release file: " + item["path"])


def validate_bundle(root, source=None):
    root = Path(root)
    manifests = list(root.glob("gripsack-release-*.json"))
    require(len(manifests) == 1, "bundle must contain exactly one manifest")
    manifest = validate_manifest(load_json(manifests[0]))
    require(manifests[0].name == f"gripsack-release-{manifest['compatible']['core']}.json", "manifest filename mismatch")
    inventory = manifest["artifacts"] + manifest["packages"] + manifest["evidence"]
    expected = {item["path"] for item in inventory} | {manifests[0].name}
    actual = set()
    for path in root.rglob("*"):
        require(not path.is_symlink(), "symlink in bundle")
        if not path.is_dir():
            regular(path)
            actual.add(path.relative_to(root).as_posix())
    require(actual == expected, "bundle has missing or unlisted files")
    for item in inventory:
        verify_file(root, item)
    candidate = load_json(root / "evidence/candidate.json")
    keys(candidate, {"schema", "candidate_id", "inputs"})
    inputs = candidate["inputs"]
    keys(inputs, {"repository", "commit", "tree", "bundle_sha256", "profile", "worker_image", "adapter_sha256", "policy"})
    require(candidate["schema"] == 1 and candidate["candidate_id"] == manifest["candidate_id"]
            == hashlib.sha256(canonical(inputs)).hexdigest(), "candidate identity mismatch")
    require(inputs["repository"] == f"https://github.com/{REPOSITORY}.git"
            and manifest["source"] == {"commit": inputs["commit"], "tree": inputs["tree"]}, "foreign source evidence")
    profile = inputs["profile"]
    require(manifest["recipe_sha256"] == hashlib.sha256(canonical(profile)).hexdigest(), "changed gate recipe")
    require(profile["external_inputs"]["authority"] == POLICY, "profile lacks standing local authority")
    gate_ids = unique(profile["gates"], "id")
    require(gate_ids == {g["id"] for g in manifest["gates"]} and "publication-packages" in gate_ids, "missing/foreign gate")
    qualification_path = root / "evidence/qualification.json"
    require(digest(qualification_path) == manifest["qualification_sha256"], "qualification digest mismatch")
    qualification = load_json(qualification_path)
    keys(qualification, {"schema", "kind", "candidate_id", "policy", "platform", "publication_authorized", "signed_release", "gates", "artifacts"})
    require(qualification["schema"] == 1 and qualification["kind"] == "trusted-local-pilot-qualification"
            and qualification["candidate_id"] == candidate["candidate_id"]
            and qualification["policy"] == inputs["policy"] and qualification["platform"] == manifest["platform"]
            and qualification["signed_release"] is False and qualification["publication_authorized"] is False,
            "foreign or falsely signed qualification")
    require(unique(qualification["gates"], "gate") == gate_ids, "qualification gate coverage mismatch")
    outputs = {}
    for gate in profile["gates"]:
        receipt_path = root / f"evidence/gates/{gate['id']}.json"
        receipt = load_json(receipt_path)
        keys(receipt, {"format", "receipt_sha256", "candidate_id", "source_commit", "worker_image",
                       "adapter_sha256", "gate", "argv", "attempt", "outcome", "returncode", "outputs"})
        require(receipt["format"] == "gripsack.gate-evidence.v1", "unsupported portable gate evidence")
        require(type(receipt["returncode"]) is int, "gate return code must be an integer")
        receipt_digest = receipt["receipt_sha256"]
        declared = next(g for g in manifest["gates"] if g["id"] == gate["id"])
        attribution = next(g for g in qualification["gates"] if g["gate"] == gate["id"])
        require(receipt_digest == declared["receipt_sha256"] == attribution["receipt_sha256"], "receipt digest mismatch")
        for field, value in (("candidate_id", candidate["candidate_id"]), ("source_commit", inputs["commit"]),
                             ("worker_image", inputs["worker_image"]), ("adapter_sha256", inputs["adapter_sha256"]),
                             ("gate", gate["id"]), ("argv", gate["argv"]), ("attempt", attribution["attempt"]),
                             ("outcome", "passed"), ("returncode", 0)):
            require(receipt[field] == value, "foreign or failed gate receipt: " + field)
        require(unique(receipt["outputs"], "source_path") == set(gate["outputs"]), "missing gate output")
        for item in receipt["outputs"]:
            validate_record(item, ("source_path",), maximum=MAX_OBSERVED_OUTPUT_BYTES)
            safe_path(item["source_path"])
            require(item["source_path"] not in outputs, "duplicate output producer")
            outputs[item["source_path"]] = item
    require(unique(qualification["artifacts"], "source_path") == set(profile["artifacts"]), "qualified artifact coverage mismatch")
    require({"dist/" + Path(a["path"]).name for a in manifest["artifacts"]} == set(profile["artifacts"]), "release artifact inventory differs from frozen profile")
    for artifact in manifest["artifacts"]:
        original = outputs["dist/" + Path(artifact["path"]).name]
        qualified = next(q for q in qualification["artifacts"] if q["source_path"] == original["source_path"])
        require(all(original[k] == artifact[k] == qualified[k] for k in ("sha256", "size_bytes")), "artifact is not locally qualified")
    partition_outputs = {Path(path).stem: item for path, item in outputs.items() if path.startswith("local-ci/persistence/reports/")}
    require(set(partition_outputs) == {p["id"] for p in manifest["partitions"]}, "partition coverage mismatch")
    reports = []
    for partition in manifest["partitions"]:
        path = root / f"evidence/partitions/{partition['id']}.json"
        require(digest(path) == partition["sha256"] == partition_outputs[partition["id"]]["sha256"], "partition digest mismatch")
        reports.append(load_json(path))
    from check_ci_gate import admit_persistence
    admit_persistence(reports, manifest["source"]["commit"])
    index_path = root / "evidence/publication-index.json"
    require(digest(index_path) == outputs["local-ci/publication/index.json"]["sha256"], "package index lacks gate evidence")
    index = load_json(index_path)
    keys(index, {"format", "source", "packages", "metadata"})
    require(index["format"] == "gripsack.publication.v1" and index["source"] == manifest["source"]["commit"]
            and index["packages"] == manifest["packages"], "foreign package inventory")
    require(all(item in manifest["evidence"] for item in index["metadata"]), "publication metadata mismatch")
    if source is not None:
        require(clean_source(source) == manifest["source"], "source checkout differs from qualification")
        require(load_json(Path(source) / ".ci/pilot.json") == profile, "changed source recipe")
        require(compatible_source(source) == manifest["compatible"], "wrong SDK/helper source pin")
        from release_inventory import validate_inventory
        validate_inventory(source, manifest["packages"])
    return manifest


def extract_archive(archive, destination, *, maximum=MAX_BUNDLE_BYTES):
    """Read only USTAR regular headers before any extended-header allocation."""
    archive = regular(archive)
    require(archive.stat().st_size <= MAX_BUNDLE_BYTES, "transport exceeds compressed byte bound")
    destination = Path(destination)
    require(not destination.exists(), "extraction destination already exists")
    destination.mkdir(mode=0o700, parents=False)
    seen = set()
    total = 0
    expanded = 0
    # One header and at most one padding block per bounded member, plus the
    # conventional 10 KiB end record. Even a gzip zero-padding bomb is bounded.
    expanded_limit = maximum + MAX_FILES * 1024 + 10240
    try:
        with archive.open("rb") as packed, gzip.GzipFile(fileobj=packed) as stream:
            def bounded_read(count):
                nonlocal expanded
                require(count <= 1024 * 1024, "unbounded archive read")
                data = stream.read(count)
                expanded += len(data)
                require(expanded <= expanded_limit, "archive decompression exceeds bound")
                return data

            while True:
                header = bounded_read(512)
                require(len(header) == 512, "truncated archive header")
                if header == bytes(512):
                    require(bounded_read(512) == bytes(512), "missing archive end marker")
                    while trailing := bounded_read(1024 * 1024):
                        require(not trailing.strip(b"\0"), "unlisted trailing archive content")
                    break
                # frombuf parses exactly one header; unlike TarFile.next it
                # cannot first allocate a PAX/GNU extension's declared payload.
                member = tarfile.TarInfo.frombuf(header, "utf-8", "strict")
                require(member.type in (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE),
                        "archive links/devices/extended headers are forbidden")
                name = safe_path(member.name.rstrip("/") if member.isdir() else member.name)
                require(name not in seen and len(seen) < MAX_FILES, "duplicate/excess archive member")
                seen.add(name)
                require(0 <= member.size <= MAX_FILE_BYTES, "archive member exceeds bound")
                if member.isdir():
                    require(member.size == 0, "directory archive member has payload")
                    (destination / name).mkdir(mode=0o700, parents=True, exist_ok=True)
                    continue
                total += member.size
                require(total <= maximum, "archive exceeds expanded byte bound")
                target = destination / name
                target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                remaining = member.size
                with target.open("xb") as outgoing:
                    while remaining:
                        block = bounded_read(min(remaining, 1024 * 1024))
                        require(block, "truncated archive member")
                        outgoing.write(block)
                        remaining -= len(block)
                padding = (-member.size) % 512
                require(len(bounded_read(padding)) == padding, "truncated archive padding")
    except BaseException:
        shutil.rmtree(destination)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    validate = commands.add_parser("validate")
    validate.add_argument("--manifest", type=Path, required=True)
    validate.add_argument("--root", type=Path, required=True)
    validate.add_argument("--source", type=Path)
    extract = commands.add_parser("extract")
    extract.add_argument("--bundle", type=Path, required=True)
    extract.add_argument("--sha256", required=True)
    extract.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "extract":
        require(digest(args.bundle) == args.sha256, "transport bundle digest mismatch")
        require(regular(args.bundle).stat().st_size <= MAX_BUNDLE_BYTES, "transport bundle exceeds byte bound")
        extract_archive(args.bundle, args.destination)
        value = validate_bundle(args.destination)
    else:
        value = validate_bundle(args.root, args.source)
        require(load_json(args.manifest) == value, "manifest not the admitted bundle subject")
    print(json.dumps({"candidate_id": value["candidate_id"], "source": value["source"], "signed_release": False}))


if __name__ == "__main__":
    main()
