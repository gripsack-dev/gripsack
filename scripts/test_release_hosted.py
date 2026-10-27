#!/usr/bin/env python3
"""Offline finalization/identity tests; signature boundary is mocked, not proven."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import release_hosted as hosted


class Publication(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bundle = self.root / "candidate"
        (self.bundle / "artifacts").mkdir(parents=True)
        payload = self.bundle / "artifacts/grip.tar.gz"
        payload.write_bytes(b"qualified bytes, never executed")
        self.manifest = {
            "source": {"commit": "a" * 40, "tree": "b" * 40}, "candidate_id": "c" * 64,
            "compatible": {"core": "1.2.3"}, "signer": {"ref": "refs/tags/core-v1.2.3"},
            "artifacts": [{"path": "artifacts/grip.tar.gz", "sha256": hosted.schema.digest(payload)}],
            "packages": [{"registry": "npm", "name": "@gripsack/core", "version": "1.2.3",
                          "size_bytes": 42, "sha256": "d" * 64}],
        }
        self.path = self.bundle / "gripsack-release-1.2.3.json"
        self.path.write_text(json.dumps(self.manifest))
        self.signature = self.root / "signature.json"
        self.signature.write_bytes(b"TEST DOUBLE NOT A SIGNATURE")
        self.result = self.root / "publication.json"
        self.record = {
            "status": "publication_complete", "signed_release_complete": False,
            "identity": {"manifest_sha256": hosted.schema.digest(self.path),
                         "source": self.manifest["source"], "candidate_id": self.manifest["candidate_id"]},
            "packages": [{"registry": "npm", "name": "@gripsack/core", "version": "1.2.3",
                          "size_bytes": 42, "remote_sha256": "d" * 64}],
        }
        self.result.write_text(json.dumps(self.record))
        self.args = argparse.Namespace(bundle=self.bundle, signature=self.signature, publication=self.result)
        self.assets = {}
        self.created = False
        self.draft = True
        self.uploads = []
        self.fail_upload = None
        self.verifications = []
        self.addCleanup(patch.stopall)
        patch.object(hosted.schema, "validate_bundle", return_value=self.manifest).start()
        patch.object(hosted.consumer, "verify_attestation", side_effect=lambda *args: self.verifications.append(args)).start()
        patch.object(hosted, "gh", side_effect=self.gh).start()

    def gh(self, *args):
        args = tuple(map(str, args))
        if args[:1] == ("api",):
            endpoint = args[1]
            release = {"id": 1, "tag_name": "core-v1.2.3", "draft": self.draft}
            if endpoint.endswith("releases?per_page=100"):
                return json.dumps([[release] if self.created else []])
            if "/releases/tags/" in endpoint:
                return json.dumps(release)
            if "/assets?" in endpoint:
                return json.dumps([[{"name": name} for name in self.assets]])
        if args[:2] == ("release", "create"):
            self.created = True
        elif args[:2] == ("release", "upload"):
            path = Path(args[3])
            if path.name == self.fail_upload:
                raise RuntimeError("injected upload interruption")
            self.uploads.append(path.name)
            self.assets[path.name] = path.read_bytes()
        elif args[:2] == ("release", "download"):
            name = args[args.index("--pattern") + 1]
            directory = Path(args[args.index("--dir") + 1])
            (directory / name).write_bytes(self.assets[name])
        elif args[:2] == ("release", "edit"):
            self.draft = False
        else:
            raise AssertionError(args)
        return ""

    def test_exact_resume_never_replaces_assets(self):
        hosted.publish(self.args)
        self.assertFalse(self.draft)
        uploads = list(self.uploads)
        hosted.publish(self.args)
        self.assertEqual(self.uploads, uploads)
        self.assertGreaterEqual(len(self.verifications), 4)

    def test_partial_upload_stays_draft_and_resumes(self):
        self.fail_upload = self.path.name
        with self.assertRaisesRegex(RuntimeError, "interruption"):
            hosted.publish(self.args)
        self.assertTrue(self.draft)
        self.assertEqual(self.uploads, ["grip.tar.gz"])
        self.fail_upload = None
        hosted.publish(self.args)
        self.assertFalse(self.draft)
        self.assertEqual(self.uploads.count("grip.tar.gz"), 1)

    def test_same_name_different_bytes_refuses_finalization(self):
        self.created = True
        self.assets["grip.tar.gz"] = b"different bytes"
        with self.assertRaisesRegex(ValueError, "identity differs"):
            hosted.publish(self.args)
        self.assertTrue(self.draft)
        self.assertFalse(self.uploads)

    def test_extra_asset_refuses_finalization(self):
        self.created = True
        self.assets["unqualified"] = b"extra"
        with self.assertRaisesRegex(ValueError, "unexpected"):
            hosted.publish(self.args)
        self.assertTrue(self.draft)

    def test_partial_or_foreign_registry_result_blocks_native_release(self):
        for mutation in (lambda r: r.update(status="partial"),
                         lambda r: r["identity"].update(manifest_sha256="0" * 64),
                         lambda r: r["packages"][0].update(remote_sha256="0" * 64)):
            record = json.loads(json.dumps(self.record))
            mutation(record)
            self.result.write_text(json.dumps(record))
            with self.assertRaises(ValueError):
                hosted.publish(self.args)
            self.assertFalse(self.created)


class HostedWitnessAdmission(unittest.TestCase):
    def setUp(self):
        from release_operator import request_id
        from test_release_admission import binding, status
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.args = argparse.Namespace(archive=root / "archive.tar.gz", bundle=root / "bundle",
                                       predicate=root / "predicate.json")
        self.args.bundle.mkdir()
        self.binding = binding()
        self.manifest = {"source": {"commit": self.binding["source_commit"]},
                         "candidate_id": self.binding["candidate_id"],
                         "signer": {"ref": "refs/tags/" + self.binding["tag"]}}
        self.path = self.args.bundle / "gripsack-release-0.47.0.json"
        self.path.write_text(json.dumps(self.manifest))
        self.binding["manifest_sha256"] = hosted.schema.digest(self.path)
        self.rows = [status(self.binding)]
        self.status_reads = 0
        self.supersede = False
        environment = {"BUNDLE_URL": self.binding["bundle_url"], "BUNDLE_SHA256": self.binding["bundle_sha256"],
                       "MANIFEST_SHA256": self.binding["manifest_sha256"], "SOURCE_COMMIT": self.binding["source_commit"],
                       "RELEASE_TAG": self.binding["tag"], "CANDIDATE_ID": self.binding["candidate_id"],
                       "REQUEST_ID": request_id(self.binding), "GITHUB_REPOSITORY": self.binding["repository"],
                       "GITHUB_REF": self.manifest["signer"]["ref"], "GITHUB_SHA": self.binding["source_commit"],
                       "GITHUB_OUTPUT": str(root / "outputs")}
        for mocked in (patch.dict(os.environ, environment),
                       patch.object(hosted, "gh", side_effect=self.gh),
                       patch.object(hosted.schema, "validate_bundle", return_value=self.manifest),
                       patch.object(hosted.consumer, "command"),
                       patch.object(hosted.consumer, "expected_predicate", return_value={"test_boundary": "not a signature"})):
            mocked.start()
            self.addCleanup(mocked.stop)
        download = patch.object(hosted.consumer, "download")
        self.download = download.start()
        self.addCleanup(download.stop)

    def gh(self, *arguments):
        from release_admission import REPOSITORY_ID, CREATOR_ID
        self.assertEqual(arguments[:3], ("api", "--hostname", "github.com"))
        endpoint = arguments[3]
        if endpoint == f"repos/{hosted.schema.REPOSITORY}":
            return json.dumps({"id": REPOSITORY_ID, "full_name": hosted.schema.REPOSITORY})
        self.assertIn("/commits/" + self.binding["source_commit"] + "/statuses?", endpoint)
        self.status_reads += 1
        rows = self.rows
        if self.supersede and self.status_reads > 1:
            rows = [{**rows[0], "id": 11, "state": "failure", "creator": {"id": CREATOR_ID}}, *rows]
        return json.dumps(rows)

    def test_no_witness_blocks_before_download_or_predicate(self):
        self.rows = []
        with self.assertRaisesRegex(ValueError, "missing"):
            hosted.admit(self.args)
        self.download.assert_not_called()
        self.assertFalse(self.args.predicate.exists())

    def test_superseding_failure_before_signing_blocks_predicate(self):
        self.supersede = True
        with self.assertRaisesRegex(ValueError, "latest"):
            hosted.admit(self.args)
        self.assertEqual(self.status_reads, 2)
        self.assertFalse(self.args.predicate.exists())

    def test_exact_independent_witness_allows_predicate_not_signature(self):
        hosted.admit(self.args)
        self.assertEqual(self.status_reads, 2)
        self.assertEqual(json.loads(self.args.predicate.read_text()), {"test_boundary": "not a signature"})


if __name__ == "__main__":
    unittest.main()
