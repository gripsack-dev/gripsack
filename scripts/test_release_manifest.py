"""Behavioral admission fixtures: synthetic bytes are never release evidence."""
from __future__ import annotations

import copy
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import release_manifest as release
from release_prepare import cargo_inventory, deterministic_archive, source_aliases
from check_ci_gate import PERSISTENCE_SCENARIOS, PERSISTENCE_SHARDS

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "a" * 40
TREE = "b" * 40


def put(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(release.canonical(value) + b"\n")


def fixture(root):
    profile = release.load_json(ROOT / ".ci/pilot.json")
    profile["external_inputs"]["authority"] = release.POLICY
    core = profile["version"]
    sdk = profile["external_inputs"]["sdk"]["version"]
    inputs = {"repository": "https://github.com/gripsack-dev/gripsack.git", "commit": SOURCE, "tree": TREE,
              "bundle_sha256": "c" * 64, "profile": profile, "worker_image": "sha256:" + "d" * 64,
              "adapter_sha256": "e" * 64, "policy": "fixture-local-controller"}
    candidate_id = hashlib.sha256(release.canonical(inputs)).hexdigest()
    put(root / "evidence/candidate.json", {"schema": 1, "candidate_id": candidate_id, "inputs": inputs})
    package_path = f"packages/gripsack-core-{sdk}.tgz"
    package = root / package_path
    package.parent.mkdir(parents=True)
    package.write_bytes(b"synthetic SDK fixture, never published")
    metadata_path = "evidence/packages/gripsack-core.json"
    put(root / metadata_path, {"name": "@gripsack/core", "version": sdk})
    packages = [{**release.file_record(package, package_path), "registry": "npm", "name": "@gripsack/core",
                 "version": sdk, "dependencies": [], "metadata": metadata_path}]
    put(root / "evidence/publication-index.json", {"format": "gripsack.publication.v1", "source": SOURCE,
        "packages": packages, "metadata": [release.file_record(root / metadata_path, metadata_path)]})
    reports = {}
    partitions = []
    for scenario in PERSISTENCE_SCENARIOS:
        for fault in ("error", "kill"):
            for shard in range(PERSISTENCE_SHARDS):
                name = f"Linux-{scenario}-{fault}-{shard}"
                report = {"system": "Linux", "machine": "x86_64", "scenario": scenario, "fault": fault,
                          "shard": shard, "shards": PERSISTENCE_SHARDS, "source": SOURCE,
                          "inventory": [["before", "FilePublish"]] * 19,
                          "completed_cuts": list(range(shard + 1, 20, PERSISTENCE_SHARDS)),
                          "drift_states": [False, True], "seconds": 1.0}
                path = root / f"evidence/partitions/{name}.json"
                put(path, report)
                reports[f"local-ci/persistence/reports/{name}/{name}.json"] = path
                partitions.append({"id": name, "sha256": release.digest(path)})
    artifacts = []
    pins = {}
    for original in profile["artifacts"]:
        path = root / "artifacts" / Path(original).name
        path.parent.mkdir(exist_ok=True)
        path.write_bytes(("synthetic artifact " + original).encode())
        role = "auxiliary"
        for component, expected in (("core", f"gripsack-{core}-{release.TARGET}.tar.gz"),
                                    ("bridge", f"grip-buildkit-bridge-{core}-{release.TARGET}"),
                                    ("conda", f"gripsack-conda-{core}-{release.TARGET}")):
            if path.name == expected:
                role = component
                if component != "core":
                    pins[component] = release.digest(path)
        artifacts.append({**release.file_record(path, "artifacts/" + path.name), "role": role})
    gates, qualified_gates, qualified_artifacts = [], [], []
    for gate in profile["gates"]:
        receipt_outputs = []
        for source_path in gate["outputs"]:
            if source_path in profile["artifacts"]:
                file = root / "artifacts" / Path(source_path).name
            elif source_path in reports:
                file = reports[source_path]
            elif source_path == "local-ci/publication/index.json":
                file = root / "evidence/publication-index.json"
            else:
                file = package
            item = {**release.file_record(file, "outputs/" + source_path), "source_path": source_path}
            receipt_outputs.append(item)
        receipt = {"schema": 1, "candidate_id": candidate_id, "source_commit": SOURCE,
                   "worker_image": inputs["worker_image"], "adapter_sha256": inputs["adapter_sha256"],
                   "gate": gate["id"], "argv": gate["argv"], "attempt": "fixture-attempt", "outcome": "passed",
                   "returncode": 0, "outputs": receipt_outputs}
        path = root / f"evidence/gates/{gate['id']}.json"
        original_digest = hashlib.sha256(release.canonical(receipt)).hexdigest()
        receipt.pop("schema")
        receipt.update(format="gripsack.gate-evidence.v1", receipt_sha256=original_digest)
        put(path, receipt)
        attribution = {"gate": gate["id"], "attempt": "fixture-attempt", "receipt_sha256": original_digest}
        qualified_gates.append(attribution)
        gates.append({"id": gate["id"], "receipt_sha256": original_digest})
        qualified_artifacts.extend({"source_path": item["source_path"], "sha256": item["sha256"],
                                    "size_bytes": item["size_bytes"], **attribution}
                                   for item in receipt_outputs if item["source_path"] in profile["artifacts"])
    put(root / "evidence/qualification.json", {"schema": 1, "kind": "trusted-local-pilot-qualification",
        "candidate_id": candidate_id, "policy": inputs["policy"], "platform": "linux-x86_64",
        "signed_release": False, "publication_authorized": False, "gates": qualified_gates, "artifacts": qualified_artifacts})
    manifest = {"format": release.FORMAT, "repository": release.REPOSITORY, "policy": release.POLICY,
        "source": {"commit": SOURCE, "tree": TREE}, "candidate_id": candidate_id,
        "qualification_sha256": release.digest(root / "evidence/qualification.json"),
        "recipe_sha256": hashlib.sha256(release.canonical(profile)).hexdigest(), "platform": "linux-x86_64",
        "signer": {"workflow": release.WORKFLOW, "ref": "refs/tags/core-v" + core},
        "compatible": {"core": core, "sdk": sdk, "bridge": core, "conda": core,
                       "ir": {"writer": 9, "readers": [9]}, "helper_pins": pins},
        "gates": gates, "partitions": partitions, "exclusions": release.EXCLUSIONS,
        "provenance": {"builder": "local", "hosted_build": False}, "artifacts": artifacts,
        "packages": packages, "evidence": []}
    refresh(root, manifest)
    return manifest


def refresh(root, manifest):
    manifest["evidence"] = [release.file_record(path, path.relative_to(root).as_posix())
                            for path in sorted((root / "evidence").rglob("*")) if path.is_file()]
    manifest["qualification_sha256"] = release.digest(root / "evidence/qualification.json")
    put(root / f"gripsack-release-{manifest['compatible']['core']}.json", manifest)


class ManifestAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "bundle"
        self.root.mkdir()
        self.manifest = fixture(self.root)

    def test_synthetic_bundle_is_internally_consistent_not_controller_admitted(self):
        checked = release.validate_bundle(self.root)
        self.assertEqual(checked["candidate_id"], self.manifest["candidate_id"])
        self.assertEqual(checked["provenance"], {"builder": "local", "hosted_build": False})

    def test_rejects_legacy_ir_compatibility(self):
        for compatibility in ({"writer": 8, "readers": [3, 4, 5, 6, 7, 8]},
                              {"writer": 9, "readers": [8, 9]},
                              {"writer": 8, "readers": [9]}):
            with self.subTest(compatibility=compatibility):
                value = copy.deepcopy(self.manifest)
                value["compatible"]["ir"] = compatibility
                with self.assertRaisesRegex(ValueError, "unsupported IR compatibility contract"):
                    release.validate_manifest(value)

    def test_source_bound_admission_rejects_self_consistent_bundle_omitting_all_crates(self):
        # The synthetic hierarchy is internally hashed, but not a release proof.
        # Source inventory must independently reject its SDK-only package list.
        with patch.object(release, "clean_source", return_value=self.manifest["source"]), \
                patch.object(release, "compatible_source", return_value=self.manifest["compatible"]):
            with self.assertRaisesRegex(ValueError, "exact source workspace"):
                release.validate_bundle(self.root, ROOT)

    def test_missing_foreign_duplicate_gate_partition_and_artifact(self):
        for inventory in ("gates", "partitions", "artifacts"):
            for change in ("missing", "duplicate", "foreign"):
                with self.subTest(inventory=inventory, change=change):
                    value = copy.deepcopy(self.manifest)
                    if change == "missing":
                        value[inventory].pop(0)
                    elif change == "duplicate":
                        value[inventory].append(value[inventory][0])
                    elif inventory == "artifacts":
                        value[inventory][0]["path"] = "artifacts/foreign.tar.gz"
                    else:
                        value[inventory][0]["id"] = "foreign"
                    with self.assertRaises(ValueError):
                        release.validate_manifest(value)

    def test_foreign_source_and_changed_recipe_are_refused(self):
        for field in ("source", "recipe_sha256"):
            with self.subTest(field=field):
                value = copy.deepcopy(self.manifest)
                value[field] = {"commit": "f" * 40, "tree": TREE} if field == "source" else "f" * 64
                refresh(self.root, value)
                with self.assertRaisesRegex(ValueError, "foreign source|changed gate recipe"):
                    release.validate_bundle(self.root)

    def test_receipt_rehashed_with_foreign_source_still_fails(self):
        gate = self.manifest["gates"][0]
        path = self.root / f"evidence/gates/{gate['id']}.json"
        receipt = release.load_json(path)
        receipt["source_commit"] = "f" * 40
        put(path, receipt)
        refresh(self.root, self.manifest)
        with self.assertRaisesRegex(ValueError, "source_commit"):
            release.validate_bundle(self.root)

    def test_wrong_sdk_and_helper_pin_fail_before_consumption(self):
        for component in ("sdk", "helper_pins"):
            value = copy.deepcopy(self.manifest)
            if component == "sdk":
                value["compatible"]["sdk"] = "0.0.1"
            else:
                value["compatible"]["helper_pins"]["conda"] = "f" * 64
            with self.assertRaises(ValueError):
                release.validate_manifest(value)

    def test_changed_missing_or_unlisted_payload_fails(self):
        artifact = self.root / self.manifest["artifacts"][0]["path"]
        artifact.write_bytes(b"changed bytes")
        with self.assertRaisesRegex(ValueError, "changed release file"):
            release.validate_bundle(self.root)
        artifact.unlink()
        with self.assertRaisesRegex(ValueError, "missing or unlisted"):
            release.validate_bundle(self.root)

    def test_source_dirty_and_hidden_index_are_not_frozen(self):
        with patch.object(release, "git", return_value=" M Cargo.toml"):
            with self.assertRaisesRegex(ValueError, "source dirty"):
                release.clean_source(self.root)
        with patch.object(release, "git", side_effect=["", "S Cargo.toml"]):
            with self.assertRaisesRegex(ValueError, "hidden index"):
                release.clean_source(self.root)

    def test_bundle_round_trip_has_exact_inventory(self):
        archive = Path(self.temporary.name) / "release.tar.gz"
        deterministic_archive(self.root, archive)
        extracted = Path(self.temporary.name) / "extracted"
        release.extract_archive(archive, extracted)
        self.assertEqual(release.validate_bundle(extracted), self.manifest)


class BoundedInputTests(unittest.TestCase):
    def test_bounded_json_duplicate_depth_nonfinite(self):
        for data in (b'{"a":1,"a":2}', b'{"a":NaN}', b"[" * 40 + b"0" + b"]" * 40,
                     b" " * (release.MAX_JSON_BYTES + 1)):
            with self.subTest(prefix=data[:30]), self.assertRaises(ValueError):
                release.decode_json(data)

    def test_archive_paths_links_duplicate_and_expansion_bounds(self):
        for name, kind in (("../escape", "file"), ("/absolute", "file"), ("a/../../escape", "file"),
                           ("a\\b", "file"), ("link", "symlink"), ("duplicate", "duplicate"),
                           ("huge", "oversized"), ("metadata", "extended")):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                archive = Path(temporary) / "bad.tar.gz"
                with tarfile.open(archive, "w:gz", format=tarfile.USTAR_FORMAT) as bundle:
                    member = tarfile.TarInfo(name)
                    if kind == "symlink":
                        member.type = tarfile.SYMTYPE
                        member.linkname = "/tmp/escape"
                        bundle.addfile(member)
                    else:
                        if kind == "extended":
                            member.type = tarfile.XHDTYPE
                        member.size = 4
                        bundle.addfile(member, io.BytesIO(b"data"))
                        if kind == "duplicate":
                            bundle.addfile(member, io.BytesIO(b"data"))
                destination = Path(temporary) / "out"
                with self.assertRaises(ValueError):
                    release.extract_archive(archive, destination, maximum=1 if kind == "oversized" else release.MAX_BUNDLE_BYTES)
                self.assertFalse(destination.exists())

    def test_package_dag_comes_from_metadata_not_name_order(self):
        def package(name, dependencies=(), publish=None):
            return {"id": name, "name": name, "publish": publish,
                    "dependencies": [{"name": dep, "kind": None, "path": "/source/" + dep} for dep in dependencies]}
        metadata = {"workspace_members": ["core", "ir", "policy", "fuzz"],
                    "packages": [package("core", ["ir"]), package("ir", ["policy"]), package("policy"), package("fuzz", publish=[])]}
        self.assertEqual([p["name"] for p, _ in cargo_inventory(metadata)], ["policy", "ir", "core"])
        metadata["packages"][2]["dependencies"] = [{"name": "core", "kind": None, "path": "/source/core"}]
        with self.assertRaisesRegex(ValueError, "cyclic"):
            cargo_inventory(metadata)
        metadata["packages"][2]["dependencies"] = [{"name": "fuzz", "kind": None, "path": "/source/fuzz"}]
        with self.assertRaisesRegex(ValueError, "unpublishable"):
            cargo_inventory(metadata)


class SourceAliasTests(unittest.TestCase):
    def fixture(self, directory):
        source = Path(directory) / "source"
        source.mkdir()
        environment = {"PATH": os.environ["PATH"], "HOME": directory,
                       "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}

        def git(*arguments):
            subprocess.run(["git", "-c", "user.name=fixture",
                            "-c", "user.email=fixture@example.invalid",
                            "-c", "commit.gpgsign=false", *arguments],
                           cwd=source, env=environment, capture_output=True, check=True)

        git("init", "-q")
        (source / "schema").mkdir()
        (source / "schema/value.json").write_text('{"type":"object"}\n')
        return source, git

    def test_schema_alias_chain_resolves_to_its_tracked_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source, git = self.fixture(directory)
            (source / "first.json").symlink_to("schema/value.json")
            (source / "crate").mkdir()
            (source / "crate/export.json").symlink_to("../first.json")
            git("add", "schema", "first.json", "crate")
            git("commit", "-qm", "fixture")
            self.assertEqual(source_aliases(source), {
                "first.json": "schema/value.json", "crate/export.json": "schema/value.json"})

    def test_unsafe_or_uncaptured_alias_targets_refuse(self):
        for kind in ("escape", "absolute", "dangling", "cycle", "untracked", "untracked-parent", "directory", "reentry"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                source, git = self.fixture(directory)
                outside = Path(directory) / "outside"
                outside.mkdir()
                (outside / "value.json").write_text("foreign\n")
                if kind == "escape":
                    target = "../outside/value.json"
                elif kind == "absolute":
                    target = str(source / "schema/value.json")
                elif kind == "dangling":
                    target = "missing.json"
                elif kind == "cycle":
                    target = "alias.json"
                elif kind == "untracked":
                    (source / "untracked.json").write_text("untracked\n")
                    target = "untracked.json"
                elif kind == "untracked-parent":
                    (source / "untracked-parent").symlink_to("schema")
                    target = "untracked-parent/value.json"
                elif kind == "directory":
                    target = "schema"
                else:
                    (outside / "return").symlink_to(source / "schema/value.json")
                    target = "../outside/return"
                (source / "alias.json").symlink_to(target)
                git("add", "schema", "alias.json")
                git("commit", "-qm", "fixture")
                with self.assertRaises(ValueError):
                    source_aliases(source)


if __name__ == "__main__":
    unittest.main()
