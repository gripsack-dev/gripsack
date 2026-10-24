"""Exact-byte transport and resumable-publication tests; no live registry access."""
import base64
import copy
import hashlib
import io
import json
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import Mock, patch
import urllib.error

import release_publish as publish


class PublishTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "bundle"
        (self.root / "packages").mkdir(parents=True)
        (self.root / "evidence/packages").mkdir(parents=True)
        self.result = self.root.parent / "publication.json"
        self.manifest = {"source": {"commit": "a" * 40, "tree": "b" * 40},
                         "candidate_id": "c" * 64, "compatible": {"core": "1.2.3"},
                         "packages": []}
        self.manifest_path = self.root / "gripsack-release-1.2.3.json"
        self.manifest_path.write_bytes(b"manifest identity fixture")

    def archive(self, path, files):
        with tarfile.open(path, "w:gz") as archive:
            for name, data in files.items():
                member = tarfile.TarInfo(name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def crate(self, name="gripsack-a", dependencies=(), extra=""):
        version = "1.2.3"
        filename = f"{name}-{version}.crate"
        path = self.root / "packages" / filename
        toml = f'[package]\nname = "{name}"\nversion = "{version}"\nlicense = "MIT"\n'
        toml += 'description = "publish fixture"\n' + extra
        if dependencies:
            toml += "\n[dependencies]\n" + "".join(f'{dep} = "1.2.3"\n' for dep in dependencies)
        self.archive(path, {f"{name}-{version}/Cargo.toml": toml.encode(),
                            f"{name}-{version}/src/lib.rs": b'compile_error!("must never build");'})
        metadata = publish.crate_metadata(path)
        return self.package("crates.io", name, version, path, metadata, dependencies)

    def npm(self):
        path = self.root / "packages" / "gripsack-core-1.2.3.tgz"
        metadata = {"name": "@gripsack/core", "version": "1.2.3",
                    "scripts": {"prepublishOnly": "exit 99", "prepare": "exit 98"},
                    "description": "exact tar fixture"}
        self.archive(path, {"package/package.json": json.dumps(metadata).encode(),
                            "package/index.js": b"throw new Error('never execute');"})
        return self.package("npm", metadata["name"], metadata["version"], path, metadata, ())

    def package(self, registry, name, version, path, metadata, dependencies):
        metadata_path = self.root / "evidence/packages" / (path.name + ".json")
        metadata_path.write_bytes(publish.canonical(metadata))
        item = {"registry": registry, "name": name, "version": version,
                "path": str(path.relative_to(self.root)), "size_bytes": path.stat().st_size,
                "sha256": publish.digest(path), "dependencies": list(dependencies),
                "metadata": str(metadata_path.relative_to(self.root))}
        self.manifest["packages"].append(item)
        return item

    def run_bundle(self, registry, registries=None):
        # Full admission belongs to release_manifest's tests; this boundary must
        # still be invoked before any transport even with already-verified state.
        with patch.object(publish, "validate_bundle", return_value=self.manifest) as bundle, \
                patch.object(publish, "validate_manifest", side_effect=lambda value: value):
            result = publish.publish_bundle(self.root, self.result, registries, registry)
            bundle.assert_called_once_with(self.root)
            return result

    def crate_response(self, package):
        return publish.canonical({"version": {"crate": package["name"], "num": package["version"],
                                              "checksum": package["sha256"], "yanked": False}})

    def crate_index(self, package):
        return publish.canonical({"name": package["name"], "vers": package["version"],
                                  "cksum": package["sha256"], "yanked": False}) + b"\n"

    def npm_response(self, package, payload):
        return publish.canonical({"name": package["name"], "version": package["version"],
                                  "dist": {"tarball": "https://registry.npmjs.org/@gripsack/core/-/core-1.2.3.tgz",
                                           "integrity": "sha512-" + base64.b64encode(hashlib.sha512(payload).digest()).decode(),
                                           "shasum": hashlib.sha1(payload).hexdigest()}})

    def test_crate_upload_frames_exact_payload_and_normalized_metadata(self):
        package = self.crate()
        path = self.root / package["path"]
        metadata = publish.crate_metadata(path)
        client = publish.Registry({"CARGO_REGISTRY_TOKEN": "test-token"})
        client.request = Mock(return_value=b'{"warnings":{}}')
        client.upload(package, path, metadata)
        args, kwargs = client.request.call_args
        self.assertEqual(args, ("https://crates.io/api/v1/crates/new",))
        self.assertEqual(kwargs["method"], "PUT")
        self.assertEqual(kwargs["token"], "test-token")
        body = kwargs["data"]
        length = struct.unpack("<I", body[:4])[0]
        self.assertEqual(json.loads(body[4:4 + length]), metadata)
        crate_length = struct.unpack("<I", body[4 + length:8 + length])[0]
        self.assertEqual(crate_length, path.stat().st_size)
        self.assertEqual(body[8 + length:], path.read_bytes())

    def test_npm_attachment_is_exact_tarball_without_lifecycle_execution(self):
        package = self.npm()
        path = self.root / package["path"]
        metadata = publish.npm_metadata(path)
        client = publish.Registry({"NPM_TOKEN": "test-token"})
        client.request = Mock(return_value=b'{"ok":true}')
        client.upload(package, path, metadata)
        args, kwargs = client.request.call_args
        self.assertEqual(args, ("https://registry.npmjs.org/%40gripsack%2Fcore",))
        body = json.loads(kwargs["data"])
        attachment = body["_attachments"]["core-1.2.3.tgz"]
        self.assertEqual(base64.b64decode(attachment["data"]), path.read_bytes())
        self.assertEqual(attachment["length"], path.stat().st_size)
        self.assertEqual(body["versions"]["1.2.3"]["scripts"], metadata["scripts"])
        self.assertEqual(kwargs["token"], "Bearer test-token")

    def test_crate_existing_version_requires_actual_downloaded_bytes(self):
        package = self.crate()
        payload = (self.root / package["path"]).read_bytes()
        wrong = bytes([payload[0] ^ 1]) + payload[1:]
        client = publish.Registry(sleep=Mock())
        client.request = Mock(side_effect=[self.crate_response(package), self.crate_index(package), wrong])
        with self.assertRaises(publish.DigestMismatch):
            client.remote_digest(package)
        self.assertEqual(client.request.call_count, 3)
        client.sleep.assert_not_called()

    def test_npm_existing_version_requires_actual_downloaded_bytes(self):
        package = self.npm()
        payload = (self.root / package["path"]).read_bytes()
        client = publish.Registry(sleep=Mock())
        client.request = Mock(side_effect=[self.npm_response(package, payload), b"x" * len(payload)])
        with self.assertRaises(publish.DigestMismatch):
            client.remote_digest(package)
        client.sleep.assert_not_called()

    def test_existing_exact_bytes_are_reconciled_without_upload(self):
        package = self.crate()
        client = publish.Registry()
        client.request = Mock(side_effect=[self.crate_response(package), self.crate_index(package),
                                          (self.root / package["path"]).read_bytes()])
        client.upload = Mock()
        self.assertEqual(client.publish(package, self.root / package["path"], {}), package["sha256"])
        client.upload.assert_not_called()

    def test_partial_publication_is_durable_and_resume_rechecks_remote(self):
        first = self.crate()
        second = self.crate("gripsack-b", ("gripsack-a",))
        client = Mock()
        client.publish.side_effect = [first["sha256"], publish.AuthenticationError("auth")]
        with self.assertRaises(publish.AuthenticationError):
            self.run_bundle(client)
        partial = publish.load_json(self.result)
        self.assertEqual(partial["status"], "partial")
        self.assertFalse(partial["signed_release_complete"])
        self.assertEqual(partial["packages"], [publish._record(first, first["sha256"])])
        self.assertEqual(partial["identity"], {"source": self.manifest["source"],
                                              "candidate_id": self.manifest["candidate_id"],
                                              "manifest_sha256": publish.digest(self.manifest_path)})
        resumed = Mock()
        resumed.remote_digest.return_value = first["sha256"]
        resumed.publish.return_value = second["sha256"]
        result = self.run_bundle(resumed)
        resumed.remote_digest.assert_called_once_with(first)
        self.assertEqual(resumed.publish.call_args.args[0], second)
        self.assertEqual(result["status"], "publication_complete")
        self.assertFalse(result["signed_release_complete"])
        self.assertEqual([p["name"] for p in result["packages"]], ["gripsack-a", "gripsack-b"])

    def test_resume_refuses_changed_candidate_source_and_manifest_digest(self):
        package = self.crate()
        client = Mock()
        client.publish.return_value = package["sha256"]
        self.run_bundle(client)
        original = copy.deepcopy(self.manifest)
        for field in ("candidate_id", "source", "manifest_digest"):
            with self.subTest(field=field):
                self.manifest = copy.deepcopy(original)
                self.manifest_path.write_bytes(b"manifest identity fixture")
                if field == "source":
                    self.manifest["source"]["tree"] = "f" * 40
                elif field == "manifest_digest":
                    self.manifest_path.write_bytes(b"different manifest bytes")
                else:
                    self.manifest["candidate_id"] = "d" * 64
                resumed = Mock()
                with self.assertRaisesRegex(publish.PublicationError, "identity/schema mismatch"):
                    self.run_bundle(resumed)
                self.assertEqual(resumed.mock_calls, [])

    def test_resume_rejects_forged_remote_checksum(self):
        package = self.crate()
        client = Mock()
        client.publish.return_value = package["sha256"]
        self.run_bundle(client)
        state = publish.load_json(self.result)
        state["packages"][0]["remote_sha256"] = "0" * 64
        self.result.write_bytes(publish.canonical(state))
        with self.assertRaisesRegex(publish.PublicationError, "package/digest mismatch"):
            self.run_bundle(Mock())

    def test_previously_published_missing_version_is_not_reuploaded(self):
        package = self.crate()
        client = Mock()
        client.publish.return_value = package["sha256"]
        self.run_bundle(client)
        resumed = Mock()
        resumed.remote_digest.return_value = None
        with self.assertRaisesRegex(publish.PublicationError, "previously verified.*missing"):
            self.run_bundle(resumed)
        resumed.publish.assert_not_called()
        self.assertEqual(publish.load_json(self.result)["status"], "partial")

    def test_dependency_order_is_checked_before_network(self):
        self.crate()
        self.crate("gripsack-b", ("gripsack-a",))
        self.manifest["packages"].reverse()
        client = Mock()
        with self.assertRaisesRegex(publish.PublicationError, "dependency order"):
            self.run_bundle(client)
        self.assertEqual(client.mock_calls, [])

    def test_hidden_dependency_edge_is_rejected(self):
        self.crate()
        package = self.crate("gripsack-b", ("gripsack-a",))
        package["dependencies"] = []
        with self.assertRaisesRegex(publish.PublicationError, "dependency inventory"):
            self.run_bundle(Mock())

    def test_missing_dependency_package_is_rejected(self):
        package = self.crate("gripsack-b", ("gripsack-a",))
        # An attacker cannot remove both the package and its declared edge.
        package["dependencies"] = []
        with self.assertRaisesRegex(publish.PublicationError, "dependency missing"):
            self.run_bundle(Mock())

    def test_missing_payload_fails_before_any_upload(self):
        self.crate()
        second = self.crate("gripsack-b")
        (self.root / second["path"]).unlink()
        client = Mock()
        with self.assertRaisesRegex(publish.PublicationError, "missing publication payload"):
            self.run_bundle(client)
        self.assertEqual(client.mock_calls, [])

    def test_full_bundle_admission_is_required_even_for_existing_state(self):
        client = Mock()
        with patch.object(publish, "validate_bundle", side_effect=ValueError("invalid qualification")):
            with self.assertRaises(ValueError):
                publish.publish_bundle(self.root, self.result, registry=client)
        self.assertEqual(client.mock_calls, [])

    def test_metadata_must_equal_prepared_evidence(self):
        package = self.crate()
        path = self.root / package["metadata"]
        value = publish.load_json(path)
        value["description"] = "different metadata"
        path.write_bytes(publish.canonical(value))
        with self.assertRaises(publish.DigestMismatch):
            self.run_bundle(Mock())

    def test_authentication_is_not_retried(self):
        for code in (401, 403):
            with self.subTest(code=code):
                client = publish.Registry(sleep=Mock())
                client.opener = Mock()
                client.opener.open.side_effect = urllib.error.HTTPError(
                    "https://crates.io/api/v1/crates/new", code, "auth", {}, None)
                with self.assertRaises(publish.AuthenticationError):
                    client.request("https://crates.io/api/v1/crates/new", method="PUT", data=b"bytes", token="secret")
                self.assertEqual(client.opener.open.call_count, 1)
                client.sleep.assert_not_called()

    def test_authentication_does_not_trigger_reconciliation_retry(self):
        package = self.crate()
        client = publish.Registry(sleep=Mock())
        client.remote_digest = Mock(return_value=None)
        client.upload = Mock(side_effect=publish.AuthenticationError("auth"))
        with self.assertRaises(publish.AuthenticationError):
            client.publish(package, self.root / package["path"], {})
        self.assertEqual(client.remote_digest.call_count, 1)
        self.assertEqual(client.upload.call_count, 1)
        client.sleep.assert_not_called()

    def test_uncertain_put_is_observed_without_blind_reupload(self):
        package = self.crate()
        client = publish.Registry(sleep=Mock())
        client.remote_digest = Mock(side_effect=[None, None, package["sha256"]])
        client.upload = Mock(side_effect=publish.TransportError("connection lost after send"))
        self.assertEqual(client.publish(package, self.root / package["path"], {}), package["sha256"])
        client.upload.assert_called_once()
        self.assertEqual(client.remote_digest.call_count, 3)

    def test_transport_get_retry_is_bounded(self):
        client = publish.Registry(sleep=Mock())
        client.opener = Mock()
        client.opener.open.side_effect = urllib.error.URLError("offline")
        with self.assertRaises(publish.TransportError):
            client.request("https://crates.io/api/v1/crates/gripsack/1.2.3")
        self.assertEqual(client.opener.open.call_count, publish.ATTEMPTS)
        self.assertEqual(client.sleep.call_count, publish.ATTEMPTS - 1)

    def test_registry_urls_and_tarball_redirect_targets_are_closed(self):
        client = publish.Registry()
        client.opener = Mock()
        for url in ("https://evil.example/upload", "http://registry.npmjs.org/pkg",
                    "https://secret@crates.io/api/v1/crates/new", "https://crates.io/path?token=x"):
            with self.subTest(url=url), self.assertRaises(publish.PublicationError):
                client.request(url)
        client.opener.open.assert_not_called()
        package = self.npm()
        payload = (self.root / package["path"]).read_bytes()
        info = json.loads(self.npm_response(package, payload))
        info["dist"]["tarball"] = "https://evil.example/payload"
        client.request = Mock(return_value=publish.canonical(info))
        with self.assertRaisesRegex(publish.PublicationError, "fixed registry"):
            client.remote_digest(package)
        client.request.assert_called_once()

    def test_payload_mutation_between_admission_and_upload_is_not_retried(self):
        package = self.crate()
        path = self.root / package["path"]
        metadata = publish.crate_metadata(path)
        path.write_bytes(path.read_bytes() + b"changed")
        client = publish.Registry({"CARGO_REGISTRY_TOKEN": "test-token"})
        client.request = Mock()
        with self.assertRaises(publish.DigestMismatch):
            client.upload(package, path, metadata)
        client.request.assert_not_called()

    def test_archive_link_and_traversal_are_rejected_without_extraction(self):
        path = self.root / "packages" / "bad-1.2.3.crate"
        for name, kind in (("bad-1.2.3/../../escape", tarfile.REGTYPE),
                           ("bad-1.2.3/Cargo.toml", tarfile.SYMTYPE)):
            with self.subTest(name=name):
                with tarfile.open(path, "w:gz") as archive:
                    member = tarfile.TarInfo(name)
                    member.type = kind
                    member.linkname = "/etc/passwd" if kind == tarfile.SYMTYPE else ""
                    archive.addfile(member)
                with self.assertRaises(publish.PublicationError):
                    publish.crate_metadata(path)
        self.assertFalse((self.root.parent / "escape").exists())

    def test_npm_payload_cannot_override_registry(self):
        path = self.root / "packages" / "foreign.tgz"
        self.archive(path, {"package/package.json": json.dumps({
            "name": "@gripsack/core", "version": "1.2.3",
            "publishConfig": {"registry": "https://evil.example"}}).encode()})
        with self.assertRaisesRegex(publish.PublicationError, "public npm registry"):
            publish.npm_metadata(path)

    def test_index_observation_is_bounded_and_does_not_reupload(self):
        package = self.crate()
        client = publish.Registry(sleep=Mock())
        client.request = Mock(side_effect=[self.crate_response(package)] + [None] * publish.ATTEMPTS)
        client.upload = Mock()
        with self.assertRaises(publish.TransportError):
            client.publish(package, self.root / package["path"], {})
        client.upload.assert_not_called()
        self.assertEqual(client.request.call_count, 1 + publish.ATTEMPTS)
        self.assertEqual(client.sleep.call_count, publish.ATTEMPTS - 1)

    def test_index_checksum_mismatch_is_not_retried(self):
        package = self.crate()
        client = publish.Registry(sleep=Mock())
        wrong = json.loads(self.crate_index(package))
        wrong["cksum"] = "0" * 64
        client.request = Mock(side_effect=[self.crate_response(package), publish.canonical(wrong)])
        with self.assertRaises(publish.DigestMismatch):
            client.remote_digest(package)
        self.assertEqual(client.request.call_count, 2)
        client.sleep.assert_not_called()

    def test_normalized_crate_alias_target_and_readme_are_preserved(self):
        path = self.root / "packages" / "gripsack-a-1.2.3.crate"
        toml = b'''[package]
name = "gripsack-a"
version = "1.2.3"
readme = "README.md"
rust-version = "1.88"
[dependencies.renamed]
package = "serde"
version = "1"
default-features = false
features = ["derive"]
optional = true
[target.'cfg(unix)'.build-dependencies]
cc = "1"
[dev-dependencies]
test-only = "2"
'''
        self.archive(path, {"gripsack-a-1.2.3/Cargo.toml": toml,
                            "gripsack-a-1.2.3/README.md": b"actual README\n"})
        value = publish.crate_metadata(path)
        self.assertEqual(value["readme"], "actual README\n")
        self.assertEqual(value["readme_file"], "README.md")
        self.assertEqual(value["rust_version"], "1.88")
        alias = next(dep for dep in value["deps"] if dep["name"] == "serde")
        self.assertEqual(alias, {"name": "serde", "version_req": "1", "features": ["derive"],
                                 "optional": True, "default_features": False, "target": None,
                                 "kind": "normal", "registry": None, "explicit_name_in_toml": "renamed"})
        target = next(dep for dep in value["deps"] if dep["name"] == "cc")
        self.assertEqual((target["kind"], target["target"]), ("build", "cfg(unix)"))
        self.assertEqual(publish._payload_dependencies({"registry": "crates.io"}, value), {"serde", "cc"})

    def test_duplicate_and_overdeep_package_json_are_rejected(self):
        path = self.root / "packages" / "invalid.tgz"
        for payload in (b'{"name":"@gripsack/core","name":"different"}',
                        b'{"nested":' * 40 + b"null" + b"}" * 40):
            with self.subTest(payload=payload):
                self.archive(path, {"package/package.json": payload})
                with self.assertRaises(publish.PublicationError):
                    publish.npm_metadata(path)


    def test_compressed_archive_has_a_global_expansion_bound(self):
        package = self.crate()
        with patch.object(publish, "MAX_EXPANDED", 128):
            with self.assertRaisesRegex(publish.PublicationError, "decompression bound"):
                publish.crate_metadata(self.root / package["path"])

    def test_npm_existing_exact_bytes_and_integrity_are_verified(self):
        package = self.npm()
        payload = (self.root / package["path"]).read_bytes()
        client = publish.Registry()
        client.request = Mock(side_effect=[self.npm_response(package, payload), payload])
        self.assertEqual(client.remote_digest(package), package["sha256"])
        self.assertEqual(client.request.call_count, 2)

    def test_selecting_one_registry_cannot_claim_complete_publication(self):
        crate = self.crate()
        npm = self.npm()
        client = Mock()
        client.publish.return_value = crate["sha256"]
        partial = self.run_bundle(client, ["crates.io"])
        self.assertEqual(partial["status"], "partial")
        self.assertEqual(len(partial["packages"]), 1)
        resumed = Mock()
        resumed.remote_digest.return_value = crate["sha256"]
        resumed.publish.return_value = npm["sha256"]
        complete = self.run_bundle(resumed, ["npm"])
        resumed.remote_digest.assert_called_once_with(crate)
        self.assertEqual(resumed.publish.call_args.args[0], npm)
        self.assertEqual(complete["status"], "publication_complete")
        self.assertFalse(complete["signed_release_complete"])


if __name__ == "__main__":
    unittest.main()
