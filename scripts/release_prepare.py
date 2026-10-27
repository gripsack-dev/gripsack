#!/usr/bin/env python3
"""Prepare publication inputs in the local gate; assemble only fully admitted receipts.

No registry credentials, dispatch, signatures or publication are involved here.
Assembly needs the reviewed ci-infra Python package on PYTHONPATH; it invokes its
original complete evidence admission rather than reinterpreting partial receipts.
"""
from __future__ import annotations

import argparse
from collections import deque
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import tempfile

from release_manifest import (EXCLUSIONS, FORMAT, POLICY, REPOSITORY, WORKFLOW, TARGET,
                              canonical, clean_source, compatible_source, digest,
                              extract_archive, file_record, load_json, package_order,
                              regular, require, validate_bundle)


def save(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as stream:
        stream.write(canonical(value) + b"\n")


def source_aliases(source):
    """Resolve tracked file aliases without consulting paths outside the source."""
    source = Path(source).resolve(strict=True)
    tree = subprocess.check_output(["git", "ls-tree", "-rz", "--full-tree", "HEAD"], cwd=source)
    tracked = {}
    for entry in tree.decode().split("\0"):
        if entry:
            metadata, name = entry.split("\t", 1)
            tracked[name] = metadata.split()[0]
    directories = {str(parent) for name in tracked for parent in PurePosixPath(name).parents}
    aliases = {}
    for alias, mode in tracked.items():
        if mode != "120000":
            continue
        pending, resolved, seen = deque(PurePosixPath(alias).parts), [], set()
        while pending:
            part = pending.popleft()
            if part == "..":
                require(resolved, f"source alias escapes repository: {alias}")
                resolved.pop()
                continue
            relative = str(PurePosixPath(*resolved, part))
            path = source / relative
            if tracked.get(relative) == "120000":
                require(relative not in seen, f"source alias cycle: {alias}")
                seen.add(relative)
                require(path.is_symlink(), f"tracked source alias changed: {relative}")
                target = path.readlink()
                require(not target.is_absolute(), f"absolute source alias: {relative}")
                pending.extendleft(reversed(target.parts))
            else:
                resolved.append(part)
                if pending:
                    require(relative in directories and not path.is_symlink() and path.is_dir(),
                            f"source alias crosses an untracked or non-directory parent: {alias}")
        target = str(PurePosixPath(*resolved))
        require(tracked.get(target) in ("100644", "100755"),
                f"source alias must end at a tracked regular file: {alias}")
        regular(source / target)
        aliases[alias] = target
    return aliases


def cargo_inventory(metadata):
    """Cargo, not a hand-maintained crate count, owns workspace membership."""
    members = set(metadata["workspace_members"])
    packages = {p["name"]: p for p in metadata["packages"]
                if p["id"] in members and p["publish"] != []}
    require(packages, "no publishable workspace packages")
    require(len(packages) == len([p for p in metadata["packages"] if p["id"] in members and p["publish"] != []]),
            "duplicate publishable Cargo package")
    for package in packages.values():
        require(package["publish"] in (None, ["crates-io"]), "unsupported Cargo registry")
    dependencies = {}
    for name, package in packages.items():
        local = set()
        for dependency in package["dependencies"]:
            if dependency["kind"] == "dev":
                continue
            if dependency.get("path"):
                require(dependency["name"] in packages, "unpublishable workspace dependency")
                local.add(dependency["name"])
        dependencies[name] = sorted(local)
    ordered = []
    emitted = set()
    while len(emitted) < len(packages):
        ready = sorted(name for name in packages if name not in emitted and set(dependencies[name]) <= emitted)
        require(ready, "cyclic package publication DAG")
        for name in ready:
            ordered.append((packages[name], dependencies[name]))
            emitted.add(name)
    return ordered


def deterministic_archive(root, destination):
    """Only declared regular files; tar metadata carries no host paths or owners."""
    import gzip
    with Path(destination).open("xb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", mtime=0, filename="") as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for path in sorted(Path(root).rglob("*")):
                require(not path.is_symlink(), "source symlink in publication bundle")
                if path.is_dir():
                    continue
                regular(path)
                entry = tarfile.TarInfo(path.relative_to(root).as_posix())
                entry.size = path.stat().st_size
                entry.mode = 0o644
                with path.open("rb") as stream:
                    archive.addfile(entry, stream)


def prepare_packages(source, revision, sdk_payload, output):
    """Runs only in the source-labelled local tool image, never on the signer."""
    from release_publish import crate_metadata, npm_metadata
    source, output = Path(source), Path(output)
    require(not output.exists(), "publication output already exists")
    output.mkdir(parents=True)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"], cwd=source))
    ordered = cargo_inventory(metadata)
    target = output / "cargo-target"
    command = ["cargo", "package", "--locked", "--target-dir", str(target)]
    for package, _ in ordered:
        command += ["-p", package["name"]]
    # Default cargo package verifies every extracted package, including builds.
    # --no-verify/--allow-dirty would silently weaken this local-only gate.
    subprocess.run(command, cwd=source, check=True)
    staged = output / "bundle"
    (staged / "packages").mkdir(parents=True)
    (staged / "evidence/packages").mkdir(parents=True)
    packages, metadata_records = [], []
    for package, dependencies in ordered:
        name, version = package["name"], package["version"]
        filename = f"{name}-{version}.crate"
        payload = staged / "packages" / filename
        shutil.copyfile(regular(target / "package" / filename), payload)
        metadata_path = f"evidence/packages/{name}.json"
        publish_metadata = crate_metadata(payload)
        require(publish_metadata["name"] == name and publish_metadata["vers"] == version, "Cargo packaged another identity")
        save(staged / metadata_path, publish_metadata)
        metadata_records.append(file_record(staged / metadata_path, metadata_path))
        packages.append({**file_record(payload, "packages/" + filename), "registry": "crates.io",
                         "name": name, "version": version, "dependencies": dependencies, "metadata": metadata_path})
    npm = npm_metadata(sdk_payload)
    expected_npm = load_json(source / "typescript/package.json")
    require(npm == expected_npm, "SDK payload differs from committed package metadata")
    filename = f"gripsack-core-{npm['version']}.tgz"
    sdk_destination = staged / "packages" / filename
    shutil.copyfile(regular(sdk_payload), sdk_destination)
    metadata_path = "evidence/packages/gripsack-core.json"
    save(staged / metadata_path, npm)
    metadata_records.append(file_record(staged / metadata_path, metadata_path))
    packages.append({**file_record(sdk_destination, "packages/" + filename), "registry": "npm",
                     "name": npm["name"], "version": npm["version"], "dependencies": [], "metadata": metadata_path})
    package_order(packages)
    from release_inventory import validate_inventory
    validate_inventory(source, packages)
    index = {"format": "gripsack.publication.v1", "source": revision, "packages": packages, "metadata": metadata_records}
    save(output / "index.json", index)
    deterministic_archive(staged, output / "packages.tar.gz")
    # Output is an input archive and index, not cargo's scratch build tree.
    shutil.rmtree(target)
    shutil.rmtree(staged)


def local_gate():
    from local_ci_runtime import ROOT, STATE, container, load_image, source
    from local_ci import profile
    from release_installer import render
    revision = source()
    require((ROOT / "install.sh").read_text() == render(ROOT), "generated installer is stale")
    require((ROOT / "crates/gripsack/src/commands/self_update/installer.sh").read_text() == render(ROOT),
            "packaged updater installer is stale")
    # Cargo exports internal schema aliases as regular package files. Admit only
    # tracked, confined file targets; publication archives still forbid links.
    print("source aliases: " + json.dumps(source_aliases(ROOT), sort_keys=True), flush=True)
    sdk = f"gripsack-core-{profile()['external_inputs']['sdk']['version']}.tgz"
    container(load_image("local-tools"), ["python3", "scripts/release_prepare.py", "packages",
              "--source", "/app", "--commit", revision, "--sdk", "/app/dist/" + sdk,
              "--output", "/app/local-ci/publication"],
              mounts=[(ROOT / "dist", "/app/dist", True), (STATE, "/app/local-ci", False)])
    installer = ROOT / "dist" / "install.sh"
    shutil.copyfile(ROOT / "install.sh", installer)
    source()


def assemble(source, candidate_path, evidence_root, destination):
    from ci_infra.records import admit, load_candidate, passed_attempt
    source, destination = Path(source), Path(destination)
    require(not destination.exists(), "release destination already exists")
    identity = clean_source(source)
    candidate = load_candidate(candidate_path)
    require(identity == {k: candidate["inputs"][k] for k in ("commit", "tree")}, "candidate/source mismatch")
    profile = load_json(source / ".ci/pilot.json")
    require(profile == candidate["inputs"]["profile"], "candidate recipe changed")
    require(profile["external_inputs"]["authority"] == POLICY, "candidate predates standing local authority")
    from release_installer import render
    require((source / "install.sh").read_text() == render(source), "generated installer is stale")
    require((source / "crates/gripsack/src/commands/self_update/installer.sh").read_text() == render(source),
            "packaged updater installer is stale")
    qualification = admit(candidate, Path(evidence_root))
    compatibility = compatible_source(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".release-", dir=destination.parent) as temporary:
        staged = Path(temporary) / "bundle"
        staged.mkdir()
        outputs = {}
        gates = []
        for gate in profile["gates"]:
            attempt = passed_attempt(candidate, Path(evidence_root), gate["id"])
            receipt = load_json(attempt / "receipt.json")
            relative = f"evidence/gates/{gate['id']}.json"
            target = staged / relative
            original_digest = digest(attempt / "receipt.json")
            # Shared with host re-admission: portable bytes never pretend to be
            # original receipts, whose private paths remain on the controller.
            from release_admission import receipt_projection
            save(target, receipt_projection(receipt, original_digest))
            gates.append({"id": gate["id"], "receipt_sha256": original_digest})
            for output in receipt["outputs"]:
                require(output["source_path"] not in outputs, "duplicate gate output")
                outputs[output["source_path"]] = attempt / output["path"]
        save(staged / "evidence/candidate.json", candidate)
        save(staged / "evidence/qualification.json", qualification)
        publication = Path(temporary) / "publication"
        extract_archive(outputs["local-ci/publication/packages.tar.gz"], publication)
        for child in publication.iterdir():
            shutil.copytree(child, staged / child.name, dirs_exist_ok=True)
        shutil.copyfile(outputs["local-ci/publication/index.json"], staged / "evidence/publication-index.json")
        index = load_json(staged / "evidence/publication-index.json")
        artifacts = []
        for original in profile["artifacts"]:
            name = Path(original).name
            target = staged / "artifacts" / name
            target.parent.mkdir(exist_ok=True)
            shutil.copyfile(outputs[original], target)
            role = "auxiliary"
            for component, expected in (("core", f"gripsack-{compatibility['core']}-{TARGET}.tar.gz"),
                                        ("bridge", f"grip-buildkit-bridge-{compatibility['bridge']}-{TARGET}"),
                                        ("conda", f"gripsack-conda-{compatibility['conda']}-{TARGET}")):
                if name == expected:
                    role = component
            artifacts.append({**file_record(target, "artifacts/" + name), "role": role})
        partitions = []
        for original, path in sorted(outputs.items()):
            if original.startswith("local-ci/persistence/reports/"):
                target = staged / "evidence/partitions" / Path(original).name
                target.parent.mkdir(exist_ok=True)
                shutil.copyfile(path, target)
                partitions.append({"id": target.stem, "sha256": digest(target)})
        manifest = {"format": FORMAT, "repository": REPOSITORY, "policy": POLICY, "source": identity,
                    "candidate_id": candidate["candidate_id"], "qualification_sha256": digest(staged / "evidence/qualification.json"),
                    "recipe_sha256": hashlib.sha256(canonical(profile)).hexdigest(), "platform": "linux-x86_64",
                    "signer": {"workflow": WORKFLOW, "ref": "refs/tags/core-v" + compatibility["core"]},
                    "compatible": compatibility, "gates": gates, "partitions": partitions, "exclusions": EXCLUSIONS,
                    "provenance": {"builder": "local", "hosted_build": False}, "artifacts": artifacts,
                    "packages": index["packages"], "evidence": [file_record(path, path.relative_to(staged).as_posix())
                                                               for path in sorted((staged / "evidence").rglob("*")) if path.is_file()]}
        manifest_path = staged / f"gripsack-release-{compatibility['core']}.json"
        save(manifest_path, manifest)
        validate_bundle(staged, source)
        bundle = Path(temporary) / "release.tar.gz"
        deterministic_archive(staged, bundle)
        result = {"candidate_id": candidate["candidate_id"], "source": identity,
                  "manifest_sha256": digest(manifest_path), "bundle_sha256": digest(bundle),
                  "local_qualification": "complete", "signed_release": False}
        # Same-filesystem publish only after all admission, with no replacement.
        destination.mkdir(mode=0o700)
        os.rename(staged, destination / "bundle")
        os.rename(bundle, destination / "release.tar.gz")
        save(destination / "nomination.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("gate")
    packages = commands.add_parser("packages")
    packages.add_argument("--source", type=Path, required=True)
    packages.add_argument("--commit", required=True)
    packages.add_argument("--sdk", type=Path, required=True)
    packages.add_argument("--output", type=Path, required=True)
    assembly = commands.add_parser("assemble")
    assembly.add_argument("--source", type=Path, required=True)
    assembly.add_argument("--candidate", type=Path, required=True)
    assembly.add_argument("--evidence", type=Path, required=True)
    assembly.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "gate":
        local_gate()
    elif args.command == "packages":
        prepare_packages(args.source, args.commit, args.sdk, args.output)
    else:
        print(json.dumps(assemble(args.source, args.candidate, args.evidence, args.destination)))


if __name__ == "__main__":
    main()
