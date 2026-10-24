"""Source-only package inventory regression tests; no Cargo/build execution."""
import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from release_inventory import source_inventory, validate_inventory
from test_release_manifest import put


class SourceInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.source = Path(self.temporary.name)
        (self.source / "Cargo.toml").write_text('''[workspace]
members = ["crates/*", "fuzz"]
[workspace.package]
version = "0.47.0"
[workspace.dependencies]
base = { version = "0.47.0", path = "crates/base" }
''')
        self.crate("base")
        self.crate("leaf", '[dependencies]\nbase.workspace = true\n')
        self.crate("target", '[target.\'cfg(unix)\'.build-dependencies]\nrenamed = { package = "base", path = "../base", version = "0.47.0" }\n')
        (self.source / "fuzz").mkdir()
        (self.source / "fuzz/Cargo.toml").write_text('[package]\nname="fuzz"\nversion="0.0.0"\npublish=false\n')
        put(self.source / "typescript/package.json", {"name": "@gripsack/core", "version": "0.47.0"})
        self.expected = [
            {"registry": "crates.io", "name": "base", "version": "0.47.0", "dependencies": []},
            {"registry": "crates.io", "name": "leaf", "version": "0.47.0", "dependencies": ["base"]},
            {"registry": "crates.io", "name": "target", "version": "0.47.0", "dependencies": ["base"]},
            {"registry": "npm", "name": "@gripsack/core", "version": "0.47.0", "dependencies": []}]

    def crate(self, name, extra=""):
        path = self.source / "crates" / name
        path.mkdir(parents=True)
        (path / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion.workspace=true\n' + extra)

    def test_exact_source_inventory_includes_leaf_target_and_sdk_without_execution(self):
        with patch("subprocess.run", side_effect=AssertionError("no source execution")), \
                patch("subprocess.check_output", side_effect=AssertionError("no Cargo execution")):
            self.assertEqual(source_inventory(self.source), self.expected)
            validate_inventory(self.source, self.expected)

    def test_omitted_leaf_all_crates_extra_version_and_order_fail(self):
        wrong_version = copy.deepcopy(self.expected)
        wrong_version[0]["version"] = "0.46.0"
        wrong_dependencies = copy.deepcopy(self.expected)
        wrong_dependencies[1]["dependencies"] = []
        cases = [self.expected[:1] + self.expected[2:], self.expected[-1:],
                 self.expected[:-1] + [{"registry": "crates.io", "name": "extra", "version": "0.47.0", "dependencies": []}] + self.expected[-1:],
                 wrong_version, wrong_dependencies,
                 [self.expected[0], self.expected[2], self.expected[1], self.expected[3]]]
        for packages in cases:
            with self.subTest(packages=packages), self.assertRaisesRegex(ValueError, "exact source workspace"):
                validate_inventory(self.source, packages)

    def test_new_publishable_leaf_is_discovered_not_hand_counted(self):
        self.crate("added")
        with self.assertRaisesRegex(ValueError, "exact source workspace"):
            validate_inventory(self.source, self.expected)
        self.assertEqual(source_inventory(self.source)[0]["name"], "added")

    def test_unsupported_workspace_shapes_refuse_instead_of_guessing(self):
        cargo = self.source / "Cargo.toml"
        original = cargo.read_text()
        for suffix in ('\n[patch.crates-io]\nbase = { path="elsewhere" }\n',
                       '\n[replace]\n"base:0.47.0" = { path="elsewhere" }\n'):
            cargo.write_text(original + suffix)
            with self.subTest(suffix=suffix), self.assertRaisesRegex(ValueError, "overrides"):
                source_inventory(self.source)
        cargo.write_text(original)
        path = self.source / "crates/leaf/Cargo.toml"
        path.write_text(path.read_text().replace('base.workspace = true', 'base = { path="../../unlisted", version="0.47.0" }'))
        with self.assertRaisesRegex(ValueError, "implicit or external"):
            source_inventory(self.source)

    def test_source_manifest_symlink_and_private_sdk_refuse(self):
        sdk = self.source / "typescript/package.json"
        put(sdk, {"name": "@gripsack/core", "version": "0.47.0", "private": True})
        with self.assertRaisesRegex(ValueError, "SDK"):
            source_inventory(self.source)
        path = self.source / "crates/leaf/Cargo.toml"
        path.unlink()
        path.symlink_to(self.source / "crates/base/Cargo.toml")
        with self.assertRaisesRegex(ValueError, "symlink"):
            source_inventory(self.source)


if __name__ == "__main__":
    unittest.main()
