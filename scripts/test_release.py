"""Pure-logic tests for release.py (no docker, network or GitHub)."""
import json
from pathlib import Path
import struct
import unittest
import zlib

import release

ROOT = Path(__file__).resolve().parent.parent


def elf(*, segments=(1,), dynamic_needed=0, sections=None):
    """A minimal x86_64 ELF64 image with the given program headers and sections."""
    sections = sections or {}
    names = b"\0" + b"".join(name.encode() + b"\0" for name in [*sections, ".shstrtab"])
    header_size, phentsize, shentsize = 64, 56, 64
    segments = list(segments) + ([2] if dynamic_needed else [])
    dynamic = b"".join(struct.pack("<qQ", 1, 0) for _ in range(dynamic_needed)) + struct.pack("<qQ", 0, 0)
    blobs = list(sections.values()) + [names]
    phoff = header_size
    data_offset = phoff + phentsize * len(segments)
    payload, offsets = bytearray(), []
    dynamic_offset = data_offset
    payload += dynamic
    for blob in blobs:
        offsets.append(data_offset + len(payload))
        payload += blob
    shoff = data_offset + len(payload)
    header = bytearray(b"\x7fELF\x02\x01\x01" + bytes(9))
    header += struct.pack("<HHIQQQIHHHHHH", 2, 0x3E, 1, 0, phoff, shoff, 0,
                          header_size, phentsize, len(segments), shentsize, len(blobs) + 1, len(blobs))
    programs = b"".join(
        struct.pack("<IIQQQQQQ", kind, 0, dynamic_offset if kind == 2 else 0, 0, 0,
                    len(dynamic) if kind == 2 else 0, 0, 0) for kind in segments)
    table = bytes(shentsize)  # SHN_UNDEF
    position = 1
    for blob, offset, name in zip(blobs, offsets, [*sections, ".shstrtab"]):
        table += struct.pack("<IIQQQQIIQQ", position, 1, 0, 0, offset, len(blob), 0, 0, 1, 0)
        position += len(name) + 1
    return bytes(header) + programs + bytes(payload) + table


SBOM = zlib.compress(json.dumps({"packages": [
    {"name": "gripsack", "version": "0.51.0", "root": True},
    {"name": "serde", "version": "1.0.0"}]}).encode())


class PureLogic(unittest.TestCase):
    def test_versions_must_be_coordinated(self):
        self.assertEqual(release.coordinated_version("1.2.3", "1.2.3", "1.2.3"), "1.2.3")
        with self.assertRaises(release.ReleaseError):
            release.coordinated_version("1.2.3", "1.2.3", "1.2.4")

    def test_asset_set_matches_last_release_minus_evidence(self):
        self.assertEqual(release.asset_names("0.51.0"), [
            "gripsack-0.51.0-x86_64-unknown-linux-musl.tar.gz",
            "gripsack-0.51.0-x86_64-unknown-linux-musl.tar.gz.sha256",
            "gripsack-conda-0.51.0-x86_64-unknown-linux-musl",
            "gripsack-conda-0.51.0-x86_64-unknown-linux-musl.sha256",
            "gripsack-conda-0.51.0-x86_64-unknown-linux-musl.manifest.json",
            "gripsack-conda-0.51.0-x86_64-unknown-linux-musl.LICENSE-pixi",
            "gripsack-conda-0.51.0-x86_64-unknown-linux-musl.LICENSE-UV-MIT",
            "gripsack-conda-0.51.0-x86_64-unknown-linux-musl.LICENSE-rattler",
            "gripsack-core-0.51.0.tgz", "install.sh"])
        self.assertEqual(release.tag_for("0.51.0"), "core-v0.51.0")

    def test_profile_declares_the_release_assets(self):
        profile = json.loads((ROOT / ".ci/pilot.json").read_text())
        expected = ["dist/" + name for name in release.asset_names(profile["version"])]
        gates = {gate["id"]: gate for gate in profile["gates"]}
        self.assertEqual(profile["artifacts"], expected)
        self.assertEqual(gates["package"]["outputs"], expected)
        self.assertEqual(gates["smoke"]["inputs"], expected)
        self.assertEqual(profile["version"], release.workspace_version())

    def test_sidecar(self):
        self.assertTrue(release.sidecar_ok("ab  x.tar.gz\n", "x.tar.gz", "ab"))
        self.assertFalse(release.sidecar_ok("ab x.tar.gz\n", "x.tar.gz", "ab"))
        self.assertFalse(release.sidecar_ok("ab  y.tar.gz\n", "x.tar.gz", "ab"))

    def test_static_binary_with_sbom(self):
        facts = release.check_static(elf(sections={".dep-v0": SBOM}), "grip")
        self.assertEqual(len(release.embedded_sbom(facts)["packages"]), 2)

    def test_static_pie_without_needed_libraries_is_static(self):
        release.check_static(elf(segments=(1,), dynamic_needed=0), "grip")

    def test_dynamic_binaries_are_rejected(self):
        with self.assertRaises(release.ReleaseError):
            release.check_static(elf(segments=(1, 3)), "grip")
        with self.assertRaises(release.ReleaseError):
            release.check_static(elf(dynamic_needed=2), "grip")
        with self.assertRaises(release.ReleaseError):
            release.check_static(b"MZ" + bytes(100), "grip")

    def test_missing_or_foreign_sbom_is_rejected(self):
        with self.assertRaises(release.ReleaseError):
            release.embedded_sbom(release.elf_facts(elf()))
        foreign = zlib.compress(json.dumps({"packages": [{"name": "other", "root": True}]}).encode())
        with self.assertRaises(release.ReleaseError):
            release.embedded_sbom(release.elf_facts(elf(sections={".dep-v0": foreign})))

    def qualified(self, version="0.51.0"):
        candidate_id = "c" * 64
        gates = [{"id": name} for name in ("test", "package", "smoke")]
        candidate = {"candidate_id": candidate_id, "inputs": {"commit": "a" * 40, "profile": {
            "project": "gripsack", "version": version, "gates": gates}}}
        record = {"kind": "trusted-local-pilot-qualification", "candidate_id": candidate_id,
                  "gates": [{"gate": gate["id"]} for gate in gates],
                  "artifacts": [{"source_path": "dist/" + name, "gate": "package", "attempt": "0" * 32,
                                 "sha256": "0" * 64, "size_bytes": 1}
                                for name in release.asset_names(version)]}
        return record, candidate, candidate_id

    def test_qualification_must_cover_gates_and_assets(self):
        record, candidate, candidate_id = self.qualified()
        self.assertEqual(len(release.qualification_artifacts(record, candidate, candidate_id)), 10)
        with self.assertRaises(release.ReleaseError):
            release.qualification_artifacts(record, candidate, "d" * 64)
        record["gates"] = record["gates"][:-1]
        with self.assertRaises(release.ReleaseError):
            release.qualification_artifacts(record, candidate, candidate_id)
        record, candidate, candidate_id = self.qualified()
        record["artifacts"] = record["artifacts"][1:]
        with self.assertRaises(release.ReleaseError):
            release.qualification_artifacts(record, candidate, candidate_id)


if __name__ == "__main__":
    unittest.main()
