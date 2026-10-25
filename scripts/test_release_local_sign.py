"""Owner-key local signing authority: admission behavior, never crypto proof.

The cosign process boundary is a test double. Success cases exercise the
document/binding contracts and the exact recorded command surface; they are
not claims that synthetic keys or signatures are cryptographically valid.
"""
from __future__ import annotations

import argparse
import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import release_consumer as consumer
import release_evidence as evidence
import release_local_sign as signer
import release_manifest as schema
from test_release_evidence import EvidenceTests
from test_release_manifest import fixture

PUBLIC_KEY = ("-----BEGIN PUBLIC KEY-----\n"
              "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEZm9vdm9vdm9vdm9vdm9vdm9vdm9v\n"
              "dm9vdm9vdm9vdm9vdm9vdm9vdm9vdm9vdm9vdm9vdm9vdm9vdm9vdm9vdm9v\n"
              "-----END PUBLIC KEY-----")
OTHER_KEY = ("-----BEGIN PUBLIC KEY-----\n"
             "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEb3RoZXJrZXlvdGhlcmtleW90aGVy\n"
             "a2V5b3RoZXJrZXlvdGhlcmtleW90aGVya2V5b3RoZXJrZXlvdGhlcg==\n"
             "-----END PUBLIC KEY-----")
SIGNATURE = base64.b64encode(b"\x21" * 64).decode()
FINGERPRINT = hashlib.sha256(PUBLIC_KEY.encode()).hexdigest()


def document(manifest, manifest_path, *, public_key=PUBLIC_KEY, signature=SIGNATURE, payload=None):
    fingerprint = schema.key_fingerprint(public_key)
    return {"format": schema.LOCAL_SIGNATURE_FORMAT,
            "payload": payload if payload is not None
            else schema.signature_payload(manifest, schema.digest(manifest_path), fingerprint),
            "public_key": public_key, "key_fingerprint": fingerprint, "signature": signature}


def write_document(root, value, name="signature.local-signature.json"):
    path = Path(root) / name
    path.write_bytes(schema.canonical(schema.validate_signature_document(value)) + b"\n")
    return path


class CosignDouble:
    """Records the exact command surface; never a cryptographic oracle."""

    def __init__(self, *, outputs=None, refuse=False):
        self.calls = []
        self.outputs = outputs or {}
        self.refuse = refuse

    def __call__(self, args, **kwargs):
        args = list(map(str, args))
        self.calls.append(args)
        if self.refuse:
            raise ValueError(f"cosign failed: injected boundary refusal for {args[1]}")
        if args[:2] == ["cosign", "public-key"]:
            return self.outputs.get("public-key", PUBLIC_KEY + "\n")
        if args[:2] == ["cosign", "sign-blob"]:
            return self.outputs.get("sign-blob", SIGNATURE + "\n")
        if args[:2] == ["cosign", "verify-blob"]:
            return self.outputs.get("verify-blob", "Verified OK\n")
        if args[:2] == ["cosign", "version"]:
            return self.outputs.get("version", "v2.4.1\n")
        raise AssertionError(f"unexpected command: {args}")


class SignatureDocumentTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.manifest = fixture(self.root)
        self.path = self.root / f"gripsack-release-{self.manifest['compatible']['core']}.json"
        self.document = document(self.manifest, self.path)

    def test_valid_document_binds_every_identity_field(self):
        value = schema.validate_signature_document(copy.deepcopy(self.document))
        self.assertEqual(value["payload"], schema.signature_payload(
            self.manifest, schema.digest(self.path), FINGERPRINT))

    def test_wrong_key_fingerprint_is_refused(self):
        for mutation in (lambda d: d.update(key_fingerprint="0" * 64),
                         lambda d: d["payload"].update(key_fingerprint="0" * 64),
                         lambda d: d.update(public_key=OTHER_KEY),
                         lambda d: d.update(payload={**d["payload"], "repository": "evil/example"}),
                         lambda d: d.update(payload={**d["payload"], "policy": "other-authority"}),
                         lambda d: d.update(payload={**d["payload"], "format": "other.v1"}),
                         lambda d: d["payload"].pop("candidate_id"),
                         lambda d: d.update(extra=True),
                         lambda d: d.update(signature="not base64!!"),
                         lambda d: d.update(signature=base64.b64encode(b"short").decode()),
                         lambda d: d.update(format="gripsack.local-signature.v2"),
                         lambda d: d["payload"].update(source_commit="f" * 41),
                         lambda d: d["payload"].update(tag="refs/tags/core-v1.2.3")):
            with self.subTest(mutation=mutation.__name__):
                with self.assertRaises(ValueError):
                    schema.validate_signature_document(mutation(copy.deepcopy(self.document)))

    def test_payload_is_never_a_digest_only_claim(self):
        payload = schema.signature_payload(self.manifest, schema.digest(self.path), FINGERPRINT)
        self.assertIn("source_commit", payload)
        self.assertIn("tag", payload)
        self.assertIn("candidate_id", payload)
        self.assertIn("key_fingerprint", payload)


class SigningTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bundle = self.root / "bundle"
        self.bundle.mkdir()
        self.manifest = fixture(self.bundle)
        self.version = self.manifest["compatible"]["core"]
        self.path = self.bundle / f"gripsack-release-{self.version}.json"
        self.key = self.root / "signer" / "owner.key"
        self.key.parent.mkdir()
        self.key.write_bytes(b"private key bytes live only on the signer host")
        self.identity = {"manifest_sha256": schema.digest(self.path),
                         "source_commit": self.manifest["source"]["commit"],
                         "tag": "core-v" + self.version,
                         "candidate_id": self.manifest["candidate_id"]}
        self.out = self.root / "out" / f"gripsack-release-{self.version}.json{consumer.LOCAL_SIGNATURE_SUFFIX}"
        self.out.parent.mkdir()
        self.args = argparse.Namespace(bundle=self.bundle, key=self.key, out=self.out, **self.identity)
        self.double = CosignDouble()

    def sign(self):
        with patch.object(consumer, "command", self.double):
            return signer.sign(self.args)

    def test_signed_document_is_deterministic_and_fully_bound(self):
        self.sign()
        first = self.out.read_bytes()
        self.sign()
        self.assertEqual(self.out.read_bytes(), first)
        value = schema.validate_signature_document(schema.load_json(self.out))
        self.assertEqual(value["payload"], schema.signature_payload(
            self.manifest, self.identity["manifest_sha256"], FINGERPRINT))
        signed = next(args for args in self.double.calls if args[1] == "sign-blob")
        self.assertIn("--tlog-upload=false", signed)
        self.assertIn(str(self.key), signed)
        verified = [args for args in self.double.calls if args[1] == "verify-blob"]
        self.assertEqual(len(verified), 2)  # emitted document and self-check
        self.assertIn("--insecure-ignore-tlog", verified[0])

    def test_wrong_manifest_identity_refuses_before_the_key(self):
        self.args.manifest_sha256 = "0" * 64
        with self.assertRaisesRegex(ValueError, "nominated identity"):
            self.sign()
        self.assertFalse(self.double.calls)
        self.assertFalse(self.out.exists())

    def test_unadmitted_bundle_never_reaches_the_key(self):
        artifact = self.bundle / self.manifest["artifacts"][0]["path"]
        artifact.write_bytes(b"tampered bytes")
        with self.assertRaisesRegex(ValueError, "changed release file"):
            self.sign()
        self.assertFalse(self.double.calls)

    def test_key_inside_the_bundle_or_relative_is_refused(self):
        # The admitted-bundle inventory independently refuses any unlisted
        # key file; the path rule is exercised directly here.
        inside = self.bundle / "owner.key"
        inside.write_bytes(b"private key bytes")
        self.args.key = inside
        with self.assertRaisesRegex(ValueError, "inside the release bundle"):
            signer.signer_key_path(self.args)
        self.args.key = Path("relative.key")
        with self.assertRaisesRegex(ValueError, "absolute"):
            signer.signer_key_path(self.args)
        self.args.key = None
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(ValueError, "GRIPSACK_SIGNER_KEY"):
                signer.signer_key_path(self.args)
        self.assertFalse(self.double.calls)

    def test_cosign_failure_produces_no_document(self):
        self.double.refuse = True
        with self.assertRaisesRegex(ValueError, "boundary refusal"):
            self.sign()
        self.assertFalse(self.out.exists())


class VerificationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.manifest = fixture(self.root)
        self.version = self.manifest["compatible"]["core"]
        self.path = self.root / f"gripsack-release-{self.version}.json"
        self.signature = write_document(self.root, document(self.manifest, self.path))
        self.double = CosignDouble()

    def verify(self, **kwargs):
        with patch.object(consumer, "command", self.double):
            return consumer.verify_local_signature(self.path, self.manifest, self.signature, **kwargs)

    def test_accepted_signature_invokes_the_offline_verifier(self):
        self.verify(owner_key=PUBLIC_KEY)
        argv = self.double.calls[0]
        self.assertEqual(argv[:2], ["cosign", "verify-blob"])
        self.assertIn("--insecure-ignore-tlog", argv)
        payload_file = Path(argv[-1])
        self.assertEqual(payload_file.name, "payload.json")

    def test_wrong_key_refuses_before_cryptography(self):
        with self.assertRaisesRegex(ValueError, "not by the pinned owner key"):
            self.verify(owner_key=OTHER_KEY)
        self.assertFalse(self.double.calls)

    def test_wrong_manifest_refuses_before_cryptography(self):
        payload = schema.signature_payload(self.manifest, "0" * 64, FINGERPRINT)
        self.signature = write_document(self.root, document(self.manifest, self.path, payload=payload))
        with self.assertRaisesRegex(ValueError, "does not bind"):
            self.verify(owner_key=PUBLIC_KEY)
        self.assertFalse(self.double.calls)

    def test_tampered_manifest_bytes_refuse_before_cryptography(self):
        original = self.path.read_bytes()
        self.path.write_bytes(original + b"\n")
        self.addCleanup(self.path.write_bytes, original)
        with self.assertRaisesRegex(ValueError, "does not bind"):
            self.verify(owner_key=PUBLIC_KEY)

    def test_missing_or_relative_pinned_key_refuses(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(ValueError, consumer.OWNER_KEY_ENV):
                self.verify()
        with patch.dict(os.environ, {consumer.OWNER_KEY_ENV: "owner.pub"}):
            with self.assertRaisesRegex(ValueError, "absolute"):
                self.verify()

    def test_recorded_command_is_replayable_from_a_fixed_scratch(self):
        scratch = self.root / "cosign-local"
        self.verify(owner_key=PUBLIC_KEY, scratch=scratch)
        argv = self.double.calls[0]
        self.verify(owner_key=PUBLIC_KEY, scratch=scratch)
        self.assertEqual(argv, self.double.calls[1])


class AdmissionAuthorityTests(unittest.TestCase):
    """Dual-authority admission; network and cryptography are patched bounds."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bundle = self.root / "bundle"
        self.bundle.mkdir()
        self.manifest = fixture(self.bundle)
        self.version = self.manifest["compatible"]["core"]
        self.release = {"id": 5, "tag_name": "core-v" + self.version, "draft": False, "prerelease": False}
        self.sources = {self.bundle.rglob("*") and path.name: path
                        for path in self.bundle.rglob("*") if path.is_file()}
        self.hosted_name = f"gripsack-release-{self.version}.json.sigstore.json"
        self.local_name = f"gripsack-release-{self.version}.json{consumer.LOCAL_SIGNATURE_SUFFIX}"
        self.asset_names = set()
        self.attestations = []
        self.locals = []
        patched = (patch.object(consumer, "platform_check"),
                   patch.object(consumer, "select_release", return_value=self.release),
                   patch.object(consumer, "release_assets", side_effect=self.assets),
                   patch.object(consumer, "tag_source", return_value=self.manifest["source"]),
                   patch.object(consumer, "download", side_effect=self.download),
                   patch.object(consumer, "verifier"),
                   patch.object(consumer, "local_verifier"),
                   patch.object(consumer, "verify_attestation",
                                side_effect=lambda *a: self.attestations.append(a)),
                   patch.object(consumer, "verify_local_signature",
                                side_effect=lambda *a, **k: self.locals.append((a, k))))
        for item in patched:
            item.start()
            self.addCleanup(item.stop)
        self.downloads = []

    def assets(self, release):
        base = {Path(item["path"]).name for item in self.manifest["artifacts"]}
        base.add("gripsack-release-" + self.version + ".json")
        self.asset_names |= base
        return {name: {"name": name, "size": 1} for name in self.asset_names}

    def download(self, url, path, maximum):
        self.downloads.append(Path(path).name)
        source = self.sources.get(Path(path).name)
        Path(path).write_bytes(source.read_bytes() if source else b'{"synthetic": true}')

    def admit(self):
        stage = self.root / "stage"
        stage.mkdir(exist_ok=True)
        return consumer.admit(stage, check_only=True)

    def test_hosted_only_release_keeps_the_frozen_attestation_path(self):
        self.asset_names.add(self.hosted_name)
        result = self.admit()
        self.assertEqual(result["version"], self.version)
        self.assertEqual(len(self.attestations), 1)
        self.assertFalse(self.locals)

    def test_local_only_release_uses_the_owner_key_authority(self):
        self.asset_names.add(self.local_name)
        result = self.admit()
        self.assertEqual(result["version"], self.version)
        self.assertEqual(len(self.locals), 1)
        self.assertFalse(self.attestations)

    def test_both_authorities_coexist_and_both_are_fully_validated(self):
        self.asset_names |= {self.hosted_name, self.local_name}
        self.admit()
        self.assertEqual(len(self.attestations), 1)
        self.assertEqual(len(self.locals), 1)
        self.assertEqual(sorted(self.downloads),
                         sorted(["gripsack-release-" + self.version + ".json", self.hosted_name, self.local_name]))

    def test_missing_both_signatures_refuses_before_any_payload_transport(self):
        with self.assertRaisesRegex(ValueError, "no signed qualification manifest"):
            self.admit()
        self.assertFalse(self.downloads)
        self.assertFalse(self.attestations)
        self.assertFalse(self.locals)

    def test_failed_local_authority_never_falls_back_to_checksums(self):
        self.asset_names.add(self.local_name)
        consumer.verify_local_signature.side_effect = ValueError("owner key refused")
        with self.assertRaisesRegex(ValueError, "owner key refused"):
            self.admit()

    def test_failed_hosted_attestation_still_refuses(self):
        self.asset_names.add(self.hosted_name)
        consumer.verify_attestation.side_effect = ValueError("attestation refused")
        with self.assertRaisesRegex(ValueError, "attestation refused"):
            self.admit()

    def test_foreign_extra_asset_is_refused(self):
        self.asset_names |= {self.hosted_name, "unqualified.bin"}
        with self.assertRaisesRegex(ValueError, "missing or extra public release assets"):
            self.admit()


class LocalAuthorityEvidenceTests(EvidenceTests):
    """The retained-evidence observer admits the local owner-key authority."""

    def cosign(self, args, **kwargs):
        args = list(map(str, args))
        if args[:2] == ["cosign", "version"]:
            return "v2.4.1\n"
        if args[:2] == ["cosign", "verify-blob", "--help"]:
            return "usage: --key --signature --insecure-ignore-tlog\n"
        if args[:2] == ["cosign", "verify-blob"]:
            return "Verified OK\n"
        raise AssertionError(args)

    def setup_signature(self):
        self.attestation_output = "Verified OK\n"
        # Local authority: no hosted signing run; the pinned owner key is the
        # signature authority and the retained command surface is cosign.
        self.run = None
        self.ident = evidence.identity(self.record, None, 101)
        for surface in self.surfaces.values():
            surface["identity"] = self.ident
        self.receipt["identity"] = self.ident
        self.owner_pub = self.put("owner.pub", PUBLIC_KEY.encode())
        name = self.manifest_path.name + consumer.LOCAL_SIGNATURE_SUFFIX
        bundle = self.put("downloads/" + name,
                          schema.canonical(document(self.manifest, self.manifest_path)) + b"\n")
        signed = self.surfaces["signed_manifest"]
        signed["data"] = {"bundle": evidence.reference(self.root, bundle), "source_proof": self.source_proof,
                          "release": self.ref("release.json", schema.canonical(
                              {"id": 101, "tag_name": "core-v" + self.version, "draft": False, "prerelease": False})),
                          "tag_source": self.ref("tag-source.json", schema.canonical(self.manifest["source"]))}
        self.surfaces["transport_setup"] = {"identity": self.ident, "surface": "transport_setup", "commands": []}
        self.log("transport_setup", "manifest", evidence.download_argv(
            evidence.public_asset_url(self.version, self.manifest_path.name), self.manifest_path, schema.MAX_JSON_BYTES))
        self.log("transport_setup", "signature", evidence.download_argv(
            evidence.public_asset_url(self.version, bundle.name), bundle, consumer.MAX_JSON))
        self.log("transport_setup", "verifier_version", ["cosign", "--version"], "v2.4.1")
        signed["data"]["transport"] = self.ref("transport-setup.json", schema.canonical(self.surfaces.pop("transport_setup")))
        # Replay the real verifier: the recorded command must be regenerated
        # byte-identically from the pinned key and stable scratch directory.
        names = iter(("local_signature",))
        def boundary(args):
            self.log("signed_manifest", next(names), args, "Verified OK\n")
            return "Verified OK\n"
        consumer.verify_local_signature(self.manifest_path, self.manifest, bundle, scratch=self.root / "cosign-local",
            owner_key=PUBLIC_KEY, command_runner=boundary)

    def setup_site(self):
        value = {"format": "gripsack.site-release.v1", "repository": self.manifest["repository"],
                 "source": self.manifest["source"], "candidate_id": self.manifest["candidate_id"],
                 "manifest_sha256": schema.digest(self.manifest_path), "tag": "core-v" + self.version,
                 "run_id": None, "release_id": 101, "version": self.version}
        binding = self.put("site-release.json", schema.canonical(value))
        page = self.put("site.html", f"<html>gripsack {self.version}</html>".encode())
        self.surfaces["site"]["data"] = {"url": evidence.SITE_URL, "binding": evidence.reference(self.root, binding),
                                         "page": evidence.reference(self.root, page)}
        self.log("site", "site_binding", evidence.download_argv(evidence.SITE_URL, binding, schema.MAX_JSON_BYTES))
        self.log("site", "site_page", evidence.download_argv("https://gripsack.dev/", page, evidence.MAX_LOG))

    def setUp(self):
        super().setUp()
        environment = {consumer.OWNER_KEY_ENV: str(self.owner_pub)}
        for item in (patch.dict(os.environ, environment),
                     patch.object(consumer, "command", side_effect=self.cosign),
                     patch.object(consumer, "local_verifier")):
            item.start()
            self.addCleanup(item.stop)

    def verify(self):
        path = self.save()
        return evidence.verify_completion(path, schema.digest(path), self.record, None)

    def test_local_authority_completion_admits_real_verifier_replay(self):
        receipt = self.verify()
        self.assertEqual(receipt["status"], "complete")
        self.assertIsNone(receipt["identity"]["run_id"])

    def test_local_authority_rejects_a_claimed_hosted_run(self):
        path = self.save()
        run = {"id": 77, "status": "completed", "conclusion": "success"}
        with self.assertRaisesRegex(ValueError, "no hosted signing run"):
            evidence.verify_completion(path, schema.digest(path), self.record, run)

    def test_local_authority_rejects_a_boundary_refusal(self):
        with patch.object(consumer, "command", side_effect=ValueError("real cosign refused")):
            with self.assertRaisesRegex(ValueError, "real cosign refused"):
                self.verify()


if __name__ == "__main__":
    unittest.main()

# Hosted-only inherited assertions (attestation command counts, run mutation)
# do not apply to the local authority fixture.
for _hosted_only in ("test_complete_synthetic_observations_reinvoke_signature_boundary",
                     "test_recorded_signature_json_never_substitutes_for_real_verifier",
                     "test_full_identity_mismatches_are_rejected"):
    setattr(LocalAuthorityEvidenceTests, _hosted_only,
            unittest.skip("hosted-only attestation assertion")(lambda self: None))
