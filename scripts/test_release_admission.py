"""Boundary tests only: synthetic controller records are NOT release qualification.

Private-receipt tests exercise the real ci_infra.records admission implementation;
run with the reviewed ci-infra checkout on PYTHONPATH. No fake gate counters,
network writes, credential reads, compilation or signature claims are involved.
"""
from __future__ import annotations

import copy
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import release_admission as witness
import release_manifest as schema
from test_release_manifest import put


def binding():
    return {"repository": schema.REPOSITORY, "workflow": schema.WORKFLOW,
            "bundle_url": "https://example.invalid/" + "b" * 64 + "/release.tar.gz",
            "bundle_sha256": "b" * 64, "manifest_sha256": "c" * 64,
            "source_commit": "d" * 40, "tag": "core-v0.47.0", "candidate_id": "e" * 64}


def status(identity=None, identifier=10):
    identity = identity or binding()
    return {**witness.status_payload(identity), "id": identifier,
            "url": f"https://api.github.com/repos/{schema.REPOSITORY}/statuses/{identity['source_commit']}",
            "creator": {"id": witness.CREATOR_ID}}


class WitnessStatusTests(unittest.TestCase):
    def setUp(self):
        self.repository = {"id": witness.REPOSITORY_ID, "full_name": schema.REPOSITORY}

    def test_exact_success_requires_fixed_numeric_identities(self):
        self.assertEqual(witness.validate_status(binding(), self.repository, [status()]), status())
        self.assertLessEqual(len(status()["description"]), 140)
        for field, value in (("id", 42), ("id", str(witness.REPOSITORY_ID)), ("full_name", "other/repo")):
            with self.subTest(field=field, value=value), self.assertRaisesRegex(ValueError, "repository"):
                witness.validate_status(binding(), {**self.repository, field: value}, [status()])

    def test_missing_foreign_mismatched_pending_failure_and_wrong_source(self):
        with self.assertRaisesRegex(ValueError, "missing"):
            witness.validate_status(binding(), self.repository, [])
        changes = [("creator", {"id": 42}), ("creator", {"id": str(witness.CREATOR_ID)}),
                   ("state", "pending"), ("state", "failure"), ("state", "error"),
                   ("description", "manifest-sha256:" + "f" * 64),
                   ("target_url", "https://example.invalid/" + "f" * 64),
                   ("url", "https://api.github.com/repos/foreign/repo/statuses/" + "d" * 40)]
        for field, value in changes:
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                witness.validate_status(binding(), self.repository, [{**status(), field: value}])

    def test_superseding_status_never_searches_back_to_success(self):
        for field, value in (("state", "failure"), ("state", "pending"), ("creator", {"id": 42}),
                             ("description", "manifest-sha256:" + "f" * 64)):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                witness.validate_status(binding(), self.repository,
                                        [{**status(identifier=11), field: value}, status()])
        with self.assertRaisesRegex(ValueError, "stale"):
            witness.validate_status(binding(), self.repository, [status(), status(identifier=11)])

    def test_fetch_uses_exact_commit_and_first_context_not_combined_status(self):
        calls = []
        def api(endpoint):
            calls.append(endpoint)
            if endpoint == f"repos/{schema.REPOSITORY}":
                return self.repository
            return [{"context": "test", "id": 12}, {**status(identifier=11), "state": "failure"}, status()]
        with self.assertRaises(ValueError):
            witness.fetch_status(binding(), api)
        self.assertEqual(calls, [f"repos/{schema.REPOSITORY}",
            f"repos/{schema.REPOSITORY}/commits/{binding()['source_commit']}/statuses?per_page=100&page=1"])

    def test_transport_url_must_bind_exact_bundle_digest(self):
        with self.assertRaisesRegex(ValueError, "SHA256"):
            witness.status_payload({**binding(), "bundle_sha256": "a" * 64})

    def test_local_readmission_refuses_worker_before_reading_any_input(self):
        with patch.dict("os.environ", {"CI_CANDIDATE_ID": "e" * 64}), \
                patch.object(schema, "digest", side_effect=AssertionError("worker read private input")):
            with self.assertRaisesRegex(ValueError, "host-only"):
                witness.admit_local("unread-source", "unread-state", "unread-archive", binding()["bundle_url"])


class PrivateAdmissionTests(unittest.TestCase):
    def setUp(self):
        from ci_infra import records
        self.records = records
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        base = Path(self.temporary.name)
        self.state, self.root, self.source = base / "selected", base / "portable", base / "source"
        self.state.mkdir(mode=0o700)
        self.root.mkdir()
        self.source.mkdir()
        profile = {"schema": 1, "project": "gripsack", "profile": "linux-x64-trusted-pilot",
                   "platform": "linux-x86_64", "policy": records.PLATFORM_POLICY, "version": "0.47.0",
                   "external_inputs": {"authority": schema.POLICY}, "artifacts": ["dist/fixture"],
                   "gates": [{"id": "fixture", "argv": ["not-executed-test-fixture"], "timeout_seconds": 1,
                              "inputs": [], "outputs": ["dist/fixture"]}]}
        source_bytes = b"synthetic frozen archive, never submitted or executed"
        inputs = {"repository": f"https://github.com/{schema.REPOSITORY}.git", "commit": "a" * 40,
                  "tree": "b" * 40, "bundle_sha256": hashlib.sha256(source_bytes).hexdigest(),
                  "profile": profile, "worker_image": "sha256:" + "c" * 64,
                  "adapter_sha256": "d" * 64, "policy": records.POLICY}
        identifier = hashlib.sha256(schema.canonical(inputs)).hexdigest()
        self.candidate = {"schema": 1, "candidate_id": identifier, "inputs": inputs}
        self.frozen = self.state / "inbox" / identifier
        put(self.frozen / "candidate.json", self.candidate)
        (self.frozen / "source.bundle").write_bytes(source_bytes)
        settings = {"schema": 2, "scope": "trusted-local-nonpublishing", "installation": "1" * 16,
                    "worker_root": str(self.state / "worker"), "image": inputs["worker_image"],
                    "adapter_sha256": inputs["adapter_sha256"], "service_recipe": "fixture recipe",
                    "service_recipe_sha256": hashlib.sha256(b"fixture recipe").hexdigest()}
        put(self.state / "config.json", settings)
        (self.state / "config.json").chmod(0o600)
        put(self.source / ".ci/pilot.json", profile)
        put(self.root / "evidence/candidate.json", self.candidate)
        evidence = self.state / "archive" / identifier / "evidence"
        self.attempt = evidence / "fixture" / ("2" * 32)
        (self.attempt / "artifacts/dist").mkdir(parents=True)
        (self.attempt / "artifacts/dist/fixture").write_bytes(b"unit boundary fixture output")
        (self.attempt / "stdout.log").write_bytes(b"not executed; unit boundary fixture only")
        receipt = {"schema": 1, "candidate_id": identifier, "gate": "fixture", "attempt": self.attempt.name,
                   "source_commit": inputs["commit"], "worker_image": inputs["worker_image"],
                   "adapter_sha256": inputs["adapter_sha256"], "argv": profile["gates"][0]["argv"],
                   "environment": {}, "worker": {"fixture": "not a real CI execution"}, "outcome": "passed",
                   "returncode": 0, "started_at": "2026-10-23T00:00:00Z", "finished_at": "2026-10-23T00:00:01Z",
                   "elapsed_seconds": 1, "log": schema.file_record(self.attempt / "stdout.log", "stdout.log"),
                   "outputs": [{**schema.file_record(self.attempt / "artifacts/dist/fixture", "artifacts/dist/fixture"),
                                "source_path": "dist/fixture"}],
                   "execution": {"buildkit_cache": "not_observed", "fresh_gate_execution_claimed": False}}
        self.receipt = receipt
        put(self.attempt / "receipt.json", receipt)
        qualification = records.admit(self.candidate, evidence)
        put(self.root / "evidence/qualification.json", qualification)
        self.projection = self.root / "evidence/gates/fixture.json"
        put(self.projection, witness.receipt_projection(receipt, schema.digest(self.attempt / "receipt.json")))
        self.manifest = {"candidate_id": identifier, "source": {"commit": inputs["commit"], "tree": inputs["tree"]},
                         "recipe_sha256": hashlib.sha256(schema.canonical(profile)).hexdigest()}
        mocked_source = patch.object(schema, "clean_source", return_value=self.manifest["source"])
        mocked_source.start()
        self.addCleanup(mocked_source.stop)

    def admit(self):
        return witness.admit_private(self.root, self.source, self.state, self.manifest)

    def test_original_relationship_is_checked_without_leaking_private_fields(self):
        result = self.admit()
        self.assertEqual(result, schema.load_json(self.root / "evidence/qualification.json"))
        projection = schema.load_json(self.projection)
        self.assertNotIn("worker", projection)
        self.assertNotIn("environment", projection)
        self.assertNotIn(str(self.state), schema.canonical(projection).decode())

    def test_forged_projection_preserving_original_digest_is_refused(self):
        original = schema.load_json(self.projection)
        forged = copy.deepcopy(original)
        forged["outputs"][0]["sha256"] = "f" * 64
        put(self.projection, forged)
        self.assertEqual(forged["receipt_sha256"], original["receipt_sha256"])
        with self.assertRaisesRegex(ValueError, "projection differs"):
            self.admit()

    def test_private_output_or_receipt_tampering_is_refused_by_real_admit(self):
        (self.attempt / "artifacts/dist/fixture").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "digest or size"):
            self.admit()
        (self.attempt / "artifacts/dist/fixture").write_bytes(b"unit boundary fixture output")
        put(self.attempt / "receipt.json", {**self.receipt, "source_commit": "f" * 40})
        with self.assertRaisesRegex(ValueError, "foreign receipt"):
            self.admit()

    def test_wrong_private_source_profile_runtime_and_source_archive(self):
        original = copy.deepcopy(self.candidate)
        for field, value in (("commit", "f" * 40), ("worker_image", "sha256:" + "f" * 64),
                             ("profile", {**original["inputs"]["profile"], "version": "0.0.1"})):
            altered = copy.deepcopy(original)
            altered["inputs"][field] = value
            altered["candidate_id"] = hashlib.sha256(schema.canonical(altered["inputs"])).hexdigest()
            put(self.frozen / "candidate.json", altered)
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "selected candidate"):
                self.admit()
        put(self.frozen / "candidate.json", original)
        (self.frozen / "source.bundle").write_bytes(b"wrong archive")
        with self.assertRaisesRegex(ValueError, "source archive"):
            self.admit()

    def test_selected_installation_runtime_and_source_profile_are_independent(self):
        settings_path = self.state / "config.json"
        settings = schema.load_json(settings_path)
        put(settings_path, {**settings, "image": "sha256:" + "f" * 64})
        with self.assertRaisesRegex(ValueError, "selected runtime"):
            self.admit()
        put(settings_path, settings)
        profile = copy.deepcopy(self.candidate["inputs"]["profile"])
        profile["gates"][0]["argv"] = ["another-command"]
        put(self.source / ".ci/pilot.json", profile)
        with self.assertRaisesRegex(ValueError, "private candidate profile"):
            self.admit()

    def test_portable_qualification_cannot_replace_private_admission(self):
        path = self.root / "evidence/qualification.json"
        value = schema.load_json(path)
        value["gates"][0]["receipt_sha256"] = "f" * 64
        put(path, value)
        with self.assertRaisesRegex(ValueError, "private admission"):
            self.admit()

    def test_missing_candidate_missing_receipt_and_fake_installation_fail(self):
        (self.attempt / "receipt.json").unlink()
        with self.assertRaisesRegex(ValueError, "missing passed gate"):
            self.admit()
        (self.frozen / "candidate.json").unlink()
        with self.assertRaises((ValueError, FileNotFoundError)):
            self.admit()
        (self.state / "config.json").chmod(0o644)
        with self.assertRaisesRegex(ValueError, "private and operator-owned"):
            self.admit()


if __name__ == "__main__":
    unittest.main()
