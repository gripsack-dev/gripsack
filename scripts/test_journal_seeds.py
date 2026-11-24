"""Exercise the generator only, in fresh directories. No saved inputs or decoder."""
from collections import Counter
import importlib.util
from itertools import product
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("journal_seeds", ROOT / "fuzz/seeds.py")
seeds = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(seeds)


class JournalSeeds(unittest.TestCase):
    def test_semantic_matrix_and_byte_determinism(self):
        with tempfile.TemporaryDirectory() as tmp:
            def generate(name):
                root = Path(tmp) / name
                seeds.build(root)
                return {p.name: p.read_bytes() for p in (root / "journal").iterdir()}

            generated = generate("first")
            self.assertEqual(generated, generate("second"))

        actual = Counter()
        for data in generated.values():
            marker, raw = data.split(b"\0")
            prefix = raw[:len(raw) - len(raw.lstrip())]
            actual[(json.dumps(json.loads(marker), sort_keys=True), prefix,
                    json.dumps(json.loads(raw), sort_keys=True))] += 1

        expected = Counter()
        for prior, (prev, target, op), prefix, fields in product(
            ({"kind": "absent"}, {"kind": "file", "hash": "../../outside", "mode": 384},
             {"kind": "symlink", "target": "/outside"}),
            ((None, 1, "apply"), (3, 1, "rollback"), (1, 2, "apply"), (2, 3, "rollback")),
            (b"", b" ", b"\n", b"\t"),
            ({"after": "gripsack:removed"}, {"v": 1, "after": {"kind": "removed"}},
             {"v": 1, "after": {"kind": "link", "target": "gripsack:removed"}},
             {"v": 1, "after": {"kind": "file", "identity": "ab" * 32}},
             {"v": 1, "after": {"kind": "link", "target": "ab" * 32}}),
        ):
            marker = {"previous_generation": prev, "target_generation": target, "op": op}
            entry = {"dest": "/outside", "prior": prior, **fields}
            expected[(json.dumps(marker, sort_keys=True), prefix, json.dumps(entry, sort_keys=True))] = 1
        self.assertEqual(actual, expected)

    def test_existing_name_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            journal = root / "journal"
            journal.mkdir()
            existing = journal / "absent-None-1-apply-none-legacy-removed.json"
            existing.write_bytes(b"preexisting")
            with self.assertRaises(FileExistsError):
                seeds.build(root)
            self.assertEqual(existing.read_bytes(), b"preexisting")
