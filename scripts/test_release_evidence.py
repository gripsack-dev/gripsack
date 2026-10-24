"""Synthetic process/network fixtures test admission, never real release success."""
from __future__ import annotations

import base64
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import release_consumer as consumer
import release_evidence as evidence
import release_manifest as schema
import release_verify as verifier
from test_release_manifest import fixture


def archive(path, files):
    with tarfile.open(path, "w:gz", format=tarfile.USTAR_FORMAT) as tar:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size, info.mode = len(data), 0o755
            tar.addfile(info, io.BytesIO(data))


def proof(root, name, files):
    directory = root / name
    directory.mkdir()
    result = {"trees": {}, "files": {}}
    def store(kind, data):
        digest = evidence.object_digest(kind, data)
        path = directory / digest
        path.write_bytes(data)
        return digest, evidence.reference(root, path)
    structure = {}
    for relative, data in files.items():
        current = structure
        for part in relative.split("/")[:-1]:
            current = current.setdefault(part, {})
        current[relative.split("/")[-1]] = data
    def tree(current, prefix=""):
        rows = []
        for key, value in sorted(current.items()):
            if isinstance(value, dict):
                digest = tree(value, prefix + key + "/")
                mode = "40000"
            else:
                digest, ref = store("blob", value)
                result["files"][prefix + key] = ref
                mode = "100644"
            rows.append(f"{mode} {key}\0".encode() + bytes.fromhex(digest))
        digest, ref = store("tree", b"".join(rows))
        result["trees"][digest] = ref
        return digest
    tree_digest = tree(structure)
    commit, result["commit"] = store("commit", f"tree {tree_digest}\nauthor Fixture <fixture@example.invalid> 1 +0000\n\nfixture\n".encode())
    return result, {"commit": commit, "tree": tree_digest}


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.manifest = fixture(self.root)
        self.version = self.manifest["compatible"]["core"]
        self.environment = verifier.clean_environment(self.root)
        self.counter = 0
        self.core = b"synthetic core bytes, not executed"
        self.installed = self.put("home/.local/bin/grip", self.core)
        self.updated = self.put("updater/grip", self.core)
        core_item = next(a for a in self.manifest["artifacts"] if a["role"] == "core")
        archive(self.root / core_item["path"], {f"gripsack-{self.version}-{schema.TARGET}/grip": self.core})
        for item in self.manifest["artifacts"]:
            item.update(schema.file_record(self.root / item["path"], item["path"]))
        package = self.manifest["packages"][0]
        self.sdk_files = {"package.json": schema.canonical({"name": "@gripsack/core", "version": package["version"]}),
                          "index.js": b"export const fixture = true;\n"}
        archive(self.root / package["path"], {"package/" + p: b for p, b in self.sdk_files.items()})
        package.update(schema.file_record(self.root / package["path"], package["path"]))
        self.recipes = {p: ("synthetic qualified recipe " + p).encode() for p in evidence.CORE_RECIPES}
        self.source_proof, self.manifest["source"] = proof(self.root, "source-proof", self.recipes)
        self.external_files = {"hosts/laptop.ts": b"synthetic representative source", "modules/a.ts": b"module source",
                               "workspaces/conda/gripsack.ts": b"synthetic conda source", "workspaces/conda/gripsack.lock": b"frozen lock"}
        self.external_proof, external_source = proof(self.root, "example-proof", self.external_files)
        external_patch = patch.dict(evidence.EXTERNAL, {"example": ("https://example.invalid/fixture", external_source["commit"])})
        external_patch.start()
        self.addCleanup(external_patch.stop)
        self.manifest_path = self.put("downloads/gripsack-release-" + self.version + ".json", schema.canonical(self.manifest))
        self.record = {"request_id": "fixture-request", "run_id": 77, "binding": {
            "manifest_sha256": schema.digest(self.manifest_path), "candidate_id": self.manifest["candidate_id"],
            "source_commit": self.manifest["source"]["commit"], "tag": "core-v" + self.version,
            "bundle_url": "https://example.invalid/bundle", "bundle_sha256": "d" * 64}}
        self.run = {"id": 77, "status": "completed", "conclusion": "success",
                    "display_title": "release-request-fixture-request", "head_sha": self.manifest["source"]["commit"],
                    "head_branch": "core-v" + self.version, "event": "workflow_dispatch", "path": schema.WORKFLOW,
                    "repository": {"full_name": schema.REPOSITORY}}
        self.ident = evidence.identity(self.record, 77, 101)
        self.surfaces = {name: {"format": evidence.SURFACE_FORMAT, "surface": name, "identity": self.ident,
                               "status": "verified", "commands": [], "data": {}, "detail": None}
                         for name in evidence.required_surfaces(self.manifest)}
        self.receipt = {"format": evidence.FORMAT, "identity": self.ident, "status": "complete",
                        "manifest": evidence.reference(self.root, self.manifest_path), "surfaces": {}}
        self.setup_signature()
        self.setup_assets()
        self.setup_registries()
        self.setup_installer()
        self.setup_helpers()
        self.setup_sdk()
        self.setup_site()
        check = patch.object(consumer, "verifier")
        check.start()
        self.addCleanup(check.stop)
        command = patch.object(consumer, "command", return_value=self.attestation_output)
        self.real_boundary = command.start()
        self.addCleanup(command.stop)

    def put(self, relative, data):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def ref(self, relative, data):
        return evidence.reference(self.root, self.put(relative, data))

    def log(self, name, step, args, output="", cwd=None):
        number = self.counter
        self.counter += 1
        result = {"step": step, "argv": list(map(str, args)), "cwd": str(cwd or self.root),
                  "environment": dict(self.environment), "returncode": 0, "termination": "exited", "duration_seconds": 0.1,
                  "stdout": self.ref(f"logs/{number}.stdout", output.encode()), "stderr": self.ref(f"logs/{number}.stderr", b""), "stdin": None}
        self.surfaces[name]["commands"].append(result)
        return result

    def approve(self, name, prefix, repo):
        self.log(name, prefix + "_inspect", [self.installed, "trust", "inspect", repo, "--json"],
                 json.dumps({"repository": str(repo), "bundle_digest": "a" * 64, "policy_digest": "b" * 64}), cwd=repo)
        self.log(name, prefix + "_approve", [self.installed, "trust", "add", repo, "--bundle", "a" * 64, "--policy", "b" * 64], cwd=repo)

    def setup_signature(self):
        bundle = self.put("downloads/signature.json", b'{"synthetic":true}')
        signed = self.surfaces["signed_manifest"]
        signed["data"] = {"bundle": evidence.reference(self.root, bundle), "source_proof": self.source_proof,
                          "release": self.ref("release.json", schema.canonical({"id": 101, "tag_name": "core-v" + self.version, "draft": False, "prerelease": False})),
                          "tag_source": self.ref("tag-source.json", schema.canonical(self.manifest["source"])),
                          "workflow_run": self.ref("workflow-run.json", schema.canonical(self.run))}
        self.surfaces["transport_setup"] = {"identity": self.ident, "surface": "transport_setup", "commands": []}
        self.log("transport_setup", "manifest", evidence.download_argv(evidence.public_asset_url(self.version, self.manifest_path.name),
                                                                     self.manifest_path, schema.MAX_JSON_BYTES))
        self.log("transport_setup", "signature", evidence.download_argv(evidence.public_asset_url(self.version, bundle.name),
                                                                      bundle, consumer.MAX_JSON))
        self.log("transport_setup", "verifier_version", ["gh", "--version"], "gh version 2.102.0")
        signed["data"]["transport"] = self.ref("transport-setup.json", schema.canonical(self.surfaces.pop("transport_setup")))
        statement = {"predicateType": consumer.PREDICATE, "predicate": consumer.expected_predicate(self.manifest),
                     "subject": [{"digest": {"sha256": schema.digest(self.manifest_path)}}]}
        self.attestation_output = json.dumps([{"verificationResult": {"statement": statement}}])
        names = iter(("attestation_bundle", "attestation_repository"))
        def boundary(args):
            self.log("signed_manifest", next(names), args, self.attestation_output)
            return self.attestation_output
        consumer.verify_attestation(self.manifest_path, self.manifest, bundle, command_runner=boundary)

    def setup_assets(self):
        surface = self.surfaces["downloaded_assets"]
        surface["data"] = {"artifacts": {}, "core": self.ref("downloads/grip", self.core)}
        for item in self.manifest["artifacts"]:
            path = self.root / item["path"]
            surface["data"]["artifacts"][item["path"]] = evidence.reference(self.root, path)
            self.log("downloaded_assets", item["path"], evidence.download_argv(evidence.public_asset_url(self.version, path.name), path, item["size_bytes"]))

    def setup_registries(self):
        package = self.manifest["packages"][0]
        payload = (self.root / package["path"]).read_bytes()
        url = f"https://registry.npmjs.org/@gripsack/core/-/core-{package['version']}.tgz"
        metadata = {"name": package["name"], "version": package["version"], "dist": {"tarball": url,
                    "integrity": "sha512-" + base64.b64encode(hashlib.sha512(payload).digest()).decode(),
                    "shasum": hashlib.sha1(payload).hexdigest()}}
        self.surfaces["registries"]["data"] = {"responses": [
            {"url": f"https://registry.npmjs.org/%40gripsack%2Fcore/{package['version']}", "body": self.ref("registry/metadata", schema.canonical(metadata))},
            {"url": url, "body": evidence.reference(self.root, self.root / package["path"])}],
            "packages": {package["path"]: evidence.reference(self.root, self.root / package["path"])}}

    def setup_installer(self):
        installer = self.put("install.sh", self.recipes["install.sh"])
        self.surfaces["installer"]["data"] = {"installer": evidence.reference(self.root, installer),
            "installed": evidence.reference(self.root, self.installed), "before_update": self.ref("baseline/grip", b"old signed synthetic core"),
            "after_update": evidence.reference(self.root, self.updated), "previous_version": "0.1.0", "mode": "upgrade"}
        self.log("installer", "install", ["sh", installer, "--version", self.version])
        self.log("installer", "installed_version", [self.installed, "--version"], f"grip {self.version}\n")
        self.log("installer", "update", [self.updated, "self-update"], f"updated 0.1.0 → {self.version}\n")
        self.log("installer", "updated_version", [self.updated, "--version"], f"grip {self.version}\n")

    def setup_helpers(self):
        conda, bridge = self.root / "conda-consumer", self.root / "bridge-consumer"
        for relative in ("gripsack.ts", "gripsack.lock"):
            self.put("conda-consumer/" + relative, self.external_files["workspaces/conda/" + relative])
        for relative in ("gripsack.ts", "source/greeter.c"):
            self.put("bridge-consumer/" + relative, self.recipes["examples/workspaces/03-source-built/" + relative])
        self.surfaces["default_helpers"]["data"] = {"installed": {},
            "conda_lock": evidence.reference(self.root, conda / "gripsack.lock"),
            "conda_output": self.ref("home/.gs/store/conda/bin/rg", b"synthetic ripgrep output"),
            "bridge_output": self.ref("home/.gs/store/bridge/bin/greeter", b"synthetic compiled output")}
        for role, filename in (("conda", "gripsack-conda"), ("bridge", "bridge")):
            original = next(a for a in self.manifest["artifacts"] if a["role"] == role)
            namespace = "conda-helper" if role == "conda" else "buildkit-bridge"
            self.surfaces["default_helpers"]["data"]["installed"][role] = self.ref(
                f"home/.gs/tools/{namespace}-{self.version}/{filename}", (self.root / original["path"]).read_bytes())
        self.approve("default_helpers", "conda", conda)
        report = {"ok": True, "host": {"os": "linux", "arch": "x86_64"}, "outputs": [{"name": "ripgrep", "kind": "package"}]}
        self.log("default_helpers", "conda_check", [self.installed, "check", "--json"], json.dumps(report), cwd=conda)
        self.log("default_helpers", "conda_build", [self.installed, "build", "ripgrep", "--json"], json.dumps({"outputs": [{"name": "ripgrep", "kind": "package", "path": str(self.root / "home/.gs/store/conda")}]}), cwd=conda)
        self.log("default_helpers", "conda_run", [self.installed, "run", "--env", "search", "--", "rg", "^needle$"], "needle\n", cwd=conda)
        self.approve("default_helpers", "bridge", bridge)
        self.log("default_helpers", "bridge_lock", [self.installed, "update"], cwd=bridge)
        self.log("default_helpers", "bridge_build", [self.installed, "build", "greeter", "--json"], json.dumps({"outputs": [{"name": "greeter", "kind": "package", "path": str(self.root / "home/.gs/store/bridge")}]}), cwd=bridge)
        self.log("default_helpers", "bridge_run", [self.installed, "task", "run", "smoke"], "hello from greeter\n", cwd=bridge)
        self.log("default_helpers", "bridge_stop", [self.installed, "builder", "stop"], cwd=bridge)

    def setup_sdk(self):
        package = self.manifest["packages"][0]
        project = self.root / "sdk-consumer"
        for relative, data in self.external_files.items():
            self.put("sdk-consumer/" + relative, data)
        installed_files = {relative: self.ref("sdk-consumer/node_modules/@gripsack/core/" + relative, data)
                           for relative, data in self.sdk_files.items()}
        self.surfaces["sdk"]["data"] = {"tarball": evidence.reference(self.root, self.root / package["path"]),
            "installed_files": installed_files, "package_root": "sdk-consumer/node_modules/@gripsack/core",
            "recipe": evidence.reference(self.root, project / "hosts/laptop.ts"), "source_proof": self.external_proof}
        self.log("sdk", "npm_install", ["npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--package-lock=false", "--save-exact", self.root / package["path"]], cwd=project)
        self.approve("sdk", "sdk", project)
        self.log("sdk", "sdk_check", [self.installed, "check", "--host", "laptop", "--json"],
                 json.dumps({"ok": True, "host": {"os": "linux", "arch": "x86_64"}, "modules": ["real-source"]}), cwd=project)

    def setup_site(self):
        value = {"format": "gripsack.site-release.v1", "repository": self.manifest["repository"],
                 "source": self.manifest["source"], "candidate_id": self.manifest["candidate_id"],
                 "manifest_sha256": schema.digest(self.manifest_path), "tag": "core-v" + self.version,
                 "run_id": 77, "release_id": 101, "version": self.version}
        binding = self.put("site-release.json", schema.canonical(value))
        page = self.put("site.html", f"<html>gripsack {self.version}</html>".encode())
        self.surfaces["site"]["data"] = {"url": evidence.SITE_URL, "binding": evidence.reference(self.root, binding), "page": evidence.reference(self.root, page)}
        self.log("site", "site_binding", evidence.download_argv(evidence.SITE_URL, binding, schema.MAX_JSON_BYTES))
        self.log("site", "site_page", evidence.download_argv("https://gripsack.dev/", page, evidence.MAX_LOG))

    def save(self):
        self.receipt["surfaces"] = {name: self.ref(name + ".json", schema.canonical(value)) for name, value in self.surfaces.items()}
        return self.put("completion.json", schema.canonical(self.receipt))

    def verify(self):
        path = self.save()
        return evidence.verify_completion(path, schema.digest(path), self.record, self.run)

    def test_complete_synthetic_observations_reinvoke_signature_boundary(self):
        self.assertEqual(self.verify()["status"], "complete")
        self.assertEqual(self.real_boundary.call_count, 2)

    def test_recorded_signature_json_never_substitutes_for_real_verifier(self):
        self.real_boundary.side_effect = ValueError("real cryptographic verifier refused")
        with self.assertRaisesRegex(ValueError, "real cryptographic"):
            self.verify()

    def test_status_booleans_and_unreferenced_hashes_are_rejected(self):
        path = self.save()
        self.receipt["surfaces"]["sdk"] = {"status": "verified", "evidence_sha256": "e" * 64}
        path.write_bytes(schema.canonical(self.receipt))
        with self.assertRaises(ValueError):
            evidence.verify_completion(path, schema.digest(path), self.record, self.run)

    def test_full_identity_mismatches_are_rejected(self):
        for key in ("source_commit", "candidate_id", "manifest_sha256", "tag"):
            with self.subTest(key=key):
                previous = self.record["binding"][key]
                self.record["binding"][key] = "mismatched"
                with self.assertRaises(ValueError):
                    self.verify()
                self.record["binding"][key] = previous
        self.run["id"] = 78
        with self.assertRaises(ValueError):
            self.verify()

    def test_surface_identity_mismatch_is_rejected(self):
        self.surfaces["sdk"]["identity"] = dict(self.ident, run_id=78)
        with self.assertRaises(ValueError):
            self.verify()

    def test_missing_extra_and_caller_waived_surfaces_are_rejected(self):
        saved = copy.deepcopy(self.surfaces)
        for name in saved:
            with self.subTest(name=name):
                self.surfaces = copy.deepcopy(saved)
                del self.surfaces[name]
                with self.assertRaises(ValueError):
                    self.verify()
        self.surfaces = copy.deepcopy(saved)
        self.surfaces["extra"] = copy.deepcopy(saved["site"])
        with self.assertRaises(ValueError):
            self.verify()
        for name in ("site", "registries"):
            self.surfaces = copy.deepcopy(saved)
            self.surfaces[name]["status"] = "not_applicable"
            with self.assertRaises(ValueError):
                self.verify()

    def test_wrong_installed_bytes_are_rejected_even_with_refreshed_hash(self):
        self.installed.write_bytes(b"wrong installed core")
        self.surfaces["installer"]["data"]["installed"] = evidence.reference(self.root, self.installed)
        with self.assertRaisesRegex(ValueError, "wrong installed"):
            self.verify()

    def test_wrong_registry_payload_and_wrong_packed_sdk_are_rejected(self):
        package = self.manifest["packages"][0]
        path = self.root / package["path"]
        original = path.read_bytes()
        path.write_bytes(b"not the published package")
        updated = evidence.reference(self.root, path)
        registry = self.surfaces["registries"]["data"]
        registry["responses"][-1]["body"] = updated
        registry["packages"][package["path"]] = updated
        with self.assertRaisesRegex(ValueError, "registry bytes differ"):
            self.verify()
        path.write_bytes(original)
        registry["responses"][-1]["body"] = evidence.reference(self.root, path)
        registry["packages"][package["path"]] = evidence.reference(self.root, path)
        self.surfaces["sdk"]["data"]["tarball"] = self.ref("wrong-sdk.tgz", b"wrong packed SDK")
        with self.assertRaisesRegex(ValueError, "exact published tarball"):
            self.verify()

    def test_failed_installed_byte_admission_never_reaches_candidate_execution(self):
        from types import SimpleNamespace
        from unittest.mock import Mock
        source = self.root / "source-fixture"
        source.mkdir()
        (source / "install.sh").write_bytes(self.recipes["install.sh"])
        host = verifier.Verification.__new__(verifier.Verification)
        host.root, host.manifest, host.surfaces = self.root, self.manifest, self.surfaces
        host.args = SimpleNamespace(source=source)
        host.installed = None
        host.runner = Mock(environment=self.environment)
        self.installed.write_bytes(b"wrong installed bytes")
        with patch.object(schema, "clean_source", return_value=self.manifest["source"]):
            with self.assertRaisesRegex(ValueError, "wrong executable"):
                host.installer({"commands": [], "data": {}})
        self.assertIsNone(host.installed)
        self.assertEqual(host.runner.run.call_count, 1)
        with self.assertRaisesRegex(ValueError, "installer has not produced"):
            host.sdk({"commands": [], "data": {}})
        self.assertEqual(host.runner.run.call_count, 1)

    def test_process_failure_and_nonexecution_are_rejected(self):
        entry = self.surfaces["sdk"]["commands"][-1]
        entry["returncode"] = 9
        with self.assertRaisesRegex(ValueError, "execute successfully"):
            self.verify()
        entry["returncode"] = 0
        entry["termination"] = "not_started"
        with self.assertRaisesRegex(ValueError, "execute successfully"):
            self.verify()

    def test_absent_helper_and_packed_sdk_execution_are_rejected(self):
        self.surfaces["default_helpers"]["data"]["installed"].pop("bridge")
        with self.assertRaisesRegex(ValueError, "absent default helper"):
            self.verify()
        self.setup_helpers_reset()
        self.surfaces["sdk"]["commands"] = [c for c in self.surfaces["sdk"]["commands"] if c["step"] != "npm_install"]
        with self.assertRaisesRegex(ValueError, "executed consumer steps"):
            self.verify()

    def setup_helpers_reset(self):
        self.surfaces["default_helpers"]["commands"] = []
        self.setup_helpers()

    def test_stale_site_binding_is_pending_not_complete(self):
        ref = self.surfaces["site"]["data"]["binding"]
        path = self.root / ref["path"]
        value = schema.load_json(path)
        value["version"] = "0.0.1"
        path.write_bytes(schema.canonical(value))
        self.surfaces["site"]["data"]["binding"] = evidence.reference(self.root, path)
        with self.assertRaisesRegex(ValueError, "site is stale"):
            self.verify()

    def test_noop_updater_and_echo_sdk_are_rejected(self):
        data = self.surfaces["installer"]["data"]
        original = data["before_update"]
        data["before_update"] = data["after_update"]
        with self.assertRaisesRegex(ValueError, "no-op"):
            self.verify()
        data["before_update"] = original
        self.surfaces["sdk"]["commands"][-1]["argv"] = ["echo", "verified"]
        with self.assertRaisesRegex(ValueError, "required recipe"):
            self.verify()

    def test_reinstall_requires_actual_replacement_not_version_probe(self):
        data = self.surfaces["installer"]["data"]
        data.update(mode="reinstall", previous_version=self.version,
                    before_update=self.ref("baseline/grip", self.core))
        command = next(c for c in self.surfaces["installer"]["commands"] if c["step"] == "update")
        command["argv"] = [str(self.updated), "self-update", "--reinstall", "--version", self.version]
        command["stdout"] = self.ref("reinstall.stdout", f"reinstalled {self.version}\n".encode())
        self.assertEqual(self.verify()["status"], "complete")
        command["stdout"] = self.ref("reinstall.stdout", f"grip {self.version} is current\n".encode())
        with self.assertRaisesRegex(ValueError, "did not perform replacement"):
            self.verify()

    def test_credential_exposure_and_external_home_are_rejected(self):
        entry = self.surfaces["installer"]["commands"][0]
        entry["environment"]["GH_TOKEN"] = "synthetic-secret"
        with self.assertRaisesRegex(ValueError, "credentials"):
            self.verify()
        entry["environment"].pop("GH_TOKEN")
        entry["environment"]["HOME"] = "/home/operator"
        with self.assertRaisesRegex(ValueError, "privately owned"):
            self.verify()

    def test_modified_evidence_and_symlink_are_rejected(self):
        path = self.save()
        (self.root / "site.json").write_text("{}")
        with self.assertRaisesRegex(ValueError, "referenced evidence"):
            evidence.verify_completion(path, schema.digest(path), self.record, self.run)
        target = self.put("foreign", b"payload")
        link = self.root / "link"
        link.symlink_to(target)
        with self.assertRaisesRegex(ValueError, "symlink"):
            evidence.resolve(self.root, {"path": "link", "size_bytes": 7, "sha256": schema.digest(target)})

    def test_recipe_inclusion_proof_rejects_changed_source(self):
        file = self.root / self.source_proof["files"]["install.sh"]["path"]
        file.write_bytes(b"echo not a real installer")
        self.source_proof["files"]["install.sh"] = evidence.reference(self.root, file)
        with self.assertRaisesRegex(ValueError, "recipe bytes"):
            self.verify()


class ProcessRunnerTests(unittest.TestCase):
    def test_environment_is_an_allowlist_not_secret_name_filter(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {
                "GH_TOKEN": "synthetic", "AWS_SECRET_ACCESS_KEY": "synthetic", "UNKNOWN_SECRET": "synthetic",
                "GRIPSACK_CONDA_HELPER": "/foreign/helper", "NODE_OPTIONS": "--require=/foreign/script"}):
            env = verifier.clean_environment(Path(temporary))
            self.assertFalse(set(env) & {"GH_TOKEN", "AWS_SECRET_ACCESS_KEY", "UNKNOWN_SECRET", "GRIPSACK_CONDA_HELPER", "NODE_OPTIONS"})
            self.assertLessEqual(set(env), evidence.ENV_KEYS)

    def test_actual_private_process_logs_failure_and_clears_credentials(self):
        import sys
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"GH_TOKEN": "synthetic-secret"}):
            root = Path(temporary)
            runner = verifier.Runner(root)
            surface = {"commands": []}
            # This test executes only the host Python interpreter, not a release
            # artifact. The fixture cannot be offered as signed release evidence.
            with self.assertRaises(ValueError):
                runner.run(surface, "failure", [sys.executable, "-c",
                    "import os,sys; print(os.getenv('GH_TOKEN', 'absent')); sys.exit(7)"])
            record = surface["commands"][0]
            self.assertEqual(record["returncode"], 7)
            self.assertEqual(evidence.resolve(root, record["stdout"]).read_text().strip(), "absent")
            self.assertNotIn("GH_TOKEN", record["environment"])

    def test_missing_program_preserves_nonexecution_logs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            runner = verifier.Runner(root)
            surface = {"commands": []}
            with self.assertRaises(FileNotFoundError):
                runner.run(surface, "missing", [root / "absent"])
            record = surface["commands"][0]
            self.assertEqual(record["termination"], "not_started")
            self.assertTrue(evidence.resolve(root, record["stdout"]).is_file())
            self.assertTrue(evidence.resolve(root, record["stderr"]).is_file())

    def test_output_and_time_limits_retain_failure_evidence(self):
        import sys
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            runner = verifier.Runner(root)
            surface = {"commands": []}
            with patch.object(evidence, "MAX_LOG", 32), self.assertRaisesRegex(ValueError, "log bound"):
                runner.run(surface, "too-much-output", [sys.executable, "-c", "print('x' * 4096)"])
            record = surface["commands"][-1]
            self.assertEqual(record["termination"], "output_limit")
            self.assertEqual(evidence.resolve(root, record["stdout"]).stat().st_size, 32)
            with self.assertRaises(TimeoutError):
                runner.run(surface, "timeout", [sys.executable, "-c", "import time; time.sleep(10)"], timeout=0.05)
            record = surface["commands"][-1]
            self.assertEqual(record["termination"], "timeout")
            self.assertIsNotNone(record["returncode"])
            self.assertTrue(evidence.resolve(root, record["stderr"]).is_file())

    def test_recorded_registry_tarball_remains_installable_file_spec(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data = {"responses": [], "packages": {}}
            registry = verifier.RecordingRegistry(root, data)
            with patch.object(verifier.publish.Registry, "request", return_value=b"synthetic packed bytes"):
                registry.request("https://registry.npmjs.org/@gripsack/core/-/core-1.2.3.tgz", limit=1024)
            path = evidence.resolve(root, data["responses"][0]["body"])
            self.assertEqual(path.suffix, ".tgz")
            self.assertEqual(path.read_bytes(), b"synthetic packed bytes")


if __name__ == "__main__":
    unittest.main()
