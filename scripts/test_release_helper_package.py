"""Synthetic helper reuse fixtures; no build or native execution in unit tests."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("conda_package", ROOT / "tools/conda-helper/package.py")
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class HelperReuseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.here = self.root / "tools/conda-helper"
        self.source = self.root / "measured"
        self.dist = self.root / "output"
        self.source.mkdir()
        self.here.mkdir(parents=True)
        for relative in ("Cargo.toml", "Cargo.lock", "package.py", "src/main.rs",
                         "vendor/pixi_git/LICENSE", "vendor/pixi_git/LICENSE-UV-MIT",
                         "vendor/rattler_package_streaming/LICENSE"):
            path = self.here / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(relative)
        (self.root / "LICENSE").write_text("fixture project MIT license\n")
        embedded = self.root / "crates/gripsack-exec/src/workspace/sharing/notices"
        embedded.mkdir(parents=True)
        for name, source in package.portable_notice_sources(self.here).items():
            (embedded / name).write_bytes(source.read_bytes())
        (self.root / "Cargo.toml").write_text(
            '[workspace.package]\nversion = "0.51.0"\n'
            '[workspace.dependencies]\ngripsack-policy = { path = "crates/gripsack-policy" }\n'
            'unused-core = { path = "crates/gripsack" }\n'
        )
        (self.here / "Cargo.toml").write_text(
            '[package]\nname = "helper"\nversion = "0.51.0"\n'
            '[dependencies]\ngripsack-fetch = { path = "../../crates/gripsack-fetch" }\n'
        )
        for name in ("gripsack-fetch", "gripsack-policy", "gripsack"):
            directory = self.root / "crates" / name
            (directory / "src").mkdir(parents=True)
            (directory / "src/lib.rs").write_text(f"// {name}\n")
            manifest = f'[package]\nname = "{name}"\nversion.workspace = true\n'
            if name == "gripsack-fetch":
                manifest += '[dependencies]\ngripsack-policy.workspace = true\n'
            (directory / "Cargo.toml").write_text(manifest)
        (self.root / "crates/gripsack-fetch/src/conda_pins.rs").write_text("// independent pin\n")
        self.version = "0.51.0"
        self.target = "x86_64-unknown-linux-musl"
        self.name = f"gripsack-conda-{self.version}-{self.target}"
        binary = self.source / self.name
        binary.write_bytes(b"synthetic helper, not executable release evidence")
        self.digest = package.sha(binary)
        (self.source / f"{self.name}.sha256").write_text(f"{self.digest}  {self.name}\n")
        licenses = {}
        for suffix, relative in (("LICENSE-pixi", "pixi_git/LICENSE"),
                                 ("LICENSE-UV-MIT", "pixi_git/LICENSE-UV-MIT"),
                                 ("LICENSE-rattler", "rattler_package_streaming/LICENSE")):
            path = self.source / f"{self.name}.{suffix}"
            path.write_bytes((self.here / "vendor" / relative).read_bytes())
            licenses[path.name] = package.sha(path)
        self.manifest = {
            "schema": 1, "version": self.version, "target": self.target, "asset": self.name,
            "sha256": self.digest, "bytes": binary.stat().st_size,
            "lock_sha256": package.sha(self.here / "Cargo.lock"),
            "source_sha256": package.source_digest(self.here), "build_image": package.LINUX_IMAGE,
            "compiler": f"rustc {package.COMPILER} fixture", "reproducible_rounds": 1,
            "licenses": licenses,
        }
        self.save()

    def save(self):
        (self.source / f"{self.name}.manifest.json").write_text(json.dumps(self.manifest))

    def reuse(self):
        with patch.object(package, "read_pins", return_value=(self.version, {"LinuxX86_64Musl": self.digest})), \
                patch.object(package.subprocess, "run") as native, patch.object(package, "smoke") as smoke:
            package.reuse(self.here, self.root, self.source, self.dist, self.version, self.target, [])
            native.assert_called_once()
            smoke.assert_called_once_with(self.dist / self.name)

    def test_unchanged_measurements_reuse_exact_bytes_without_build(self):
        self.reuse()
        self.assertEqual(package.sha(self.dist / self.name), self.digest)
        self.assertEqual(json.loads((self.dist / f"{self.name}.manifest.json").read_text()), self.manifest)

    def test_portable_notices_must_match_canonical_sources(self):
        package.check_portable_notices(self.here)
        embedded = self.root / "crates/gripsack-exec/src/workspace/sharing/notices"
        for name in package.portable_notice_sources(self.here):
            with self.subTest(notice=name):
                path = embedded / name
                original = path.read_bytes()
                path.write_bytes(original + b"changed")
                with self.assertRaisesRegex(SystemExit, "notice differs"):
                    package.check_portable_notices(self.here)
                with self.assertRaisesRegex(SystemExit, "build inputs"):
                    self.reuse()
                path.write_bytes(original)
        original = (self.root / "LICENSE").read_bytes()
        (self.root / "LICENSE").write_bytes(original + b"changed")
        with self.assertRaisesRegex(SystemExit, "build inputs"):
            self.reuse()

    def test_changed_source_lock_packager_or_vendor_requires_remeasurement(self):
        for relative in ("Cargo.lock", "package.py", "src/main.rs", "vendor/pixi_git/LICENSE"):
            with self.subTest(relative=relative):
                path = self.here / relative
                original = path.read_bytes()
                path.write_bytes(original + b"changed")
                with self.assertRaisesRegex(SystemExit, "build inputs"):
                    self.reuse()
                path.write_bytes(original)
                self.assertFalse(self.dist.exists())

    def test_changed_transitive_crate_inputs_require_remeasurement(self):
        for relative in ("crates/gripsack-fetch/src/lib.rs", "crates/gripsack-policy/src/lib.rs",
                         "crates/gripsack-policy/Cargo.toml", "Cargo.toml"):
            with self.subTest(relative=relative):
                path = self.root / relative
                original = path.read_bytes()
                if relative == "Cargo.toml":
                    path.write_bytes(original.replace(b'0.51.0', b'0.51.1'))
                else:
                    path.write_bytes(original + b"\n# changed\n" if path.suffix == ".toml"
                                     else original + b"// changed\n")
                with self.assertRaisesRegex(SystemExit, "build inputs"):
                    self.reuse()
                path.write_bytes(original)
                self.assertFalse(self.dist.exists())

    def test_unrelated_core_sdk_and_pin_table_do_not_create_measurement_cycle(self):
        expected = package.source_digest(self.here)
        for relative in ("crates/gripsack/src/lib.rs", "crates/gripsack/Cargo.toml",
                         "typescript/src/emit.ts", "crates/gripsack-fetch/src/conda_pins.rs"):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("unrelated to helper measurement")
        self.assertEqual(package.source_digest(self.here), expected)
        workspace = self.root / "Cargo.toml"
        workspace.write_text(workspace.read_text().replace("crates/gripsack\"", "crates/unused\""))
        self.assertEqual(package.source_digest(self.here), expected)

    def test_changed_bytes_fail_compiled_pin(self):
        (self.source / self.name).write_bytes(b"wrong helper")
        with self.assertRaisesRegex(SystemExit, "pin mismatch"):
            self.reuse()
        self.assertFalse(self.dist.exists())

    def test_old_or_false_measurement_metadata_is_not_reused(self):
        for key, value in (("source_sha256", None), ("reproducible_rounds", True),
                           ("reproducible_rounds", 0), ("build_image", "foreign")):
            with self.subTest(key=key, value=value):
                original = self.manifest[key]
                self.manifest[key] = value
                self.save()
                with self.assertRaises(SystemExit):
                    self.reuse()
                self.manifest[key] = original
                self.assertFalse(self.dist.exists())
