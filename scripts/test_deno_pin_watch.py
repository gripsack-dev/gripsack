"""A Deno proposal must be usable on the supported Linux checkout without hosted CI."""
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import check_pins
from pin_pr import apply_edits


class DenoPinWatch(unittest.TestCase):
    def test_proposal_moves_the_image_and_both_runtime_consumers_together(self):
        source = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name in ("Dockerfile", check_pins.HOST_RS, "scripts/check_examples.py"):
                (root / name).parent.mkdir(exist_ok=True, parents=True)
                shutil.copyfile(source / name, root / name)
            sites = check_pins.PinSites(temp)
            major, minor, revision = map(int, sites.dockerfile_arg("DENO_VERSION").split("."))
            newer = f"{major}.{minor}.{revision + 1}"
            with (patch.object(check_pins, "github_latest", return_value=f"v{newer}"),
                  patch.object(check_pins, "docker_hub_digest", return_value="a" * 64),
                  patch.object(check_pins, "deno_sidecar_sha", return_value=None) as sidecar):
                blocked = check_pins.unit_deno(sites)
                self.assertEqual((blocked.status, blocked.edits), ("blocked", []))
                sidecar.return_value = "b" * 64
                proposal = check_pins.unit_deno(sites)
                self.assertEqual(proposal.status, "ready", proposal.note)
                sidecar.assert_called_with(newer, "x86_64-unknown-linux-gnu")
            apply_edits(proposal.edits, temp)
            moved = check_pins.PinSites(temp)
            self.assertEqual(moved.dockerfile_froms()["denoland/deno"], (newer, "a" * 64))
            self.assertEqual(moved.dockerfile_arg("DENO_VERSION"), newer)
            self.assertEqual(moved.dockerfile_arg("DENO_SHA256"), "b" * 64)
            self.assertEqual(moved.host_deno(), (newer, {"LinuxX86_64Musl": "b" * 64}))
