"""The managed-block splice (plan/0056 K5, F2 step 6).

The kernel renders, scans and splices a managed block. These flows pin what
the Lean proofs claim at the product boundary: a block gripsack wrote is
recognized as written (a second apply changes nothing), and rollback restores
a block only from a retained payload that reproduces the target's record."""

import hashlib
import json
from pathlib import Path

from conftest import grip, make_workspace_repo

STATE = Path(".local/share/gripsack")

BLOCK_REPO = """
import { file, identity, managedBlock, profile, repoFile } from "@gripsack/core";

export default profile("shell", { files: [
    file({ source: repoFile("block.sh"), content: identity(), destination: managedBlock("~/.bashrc", "shell") })
  ] });
"""


def apply(repo: Path):
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr
    return out


def selected(sandbox: Path) -> int:
    return json.loads((sandbox / STATE / "current/manifest.json").read_text())["number"]


def bytes_hash(data: bytes) -> str:
    """`canonical_bytes_hash`: the managed-block manifest domain."""
    return hashlib.sha256(b"file\0" + b"\x00" + data).hexdigest()


def test_a_payload_with_carriage_returns_before_a_newline_settles(sandbox):
    """FND-4: a carriage-return run before a line break used to normalize to
    bytes the rendered block never reproduces, so the block never matched its
    own sha. The first apply wrote a stray CRLF into an LF block; the second
    rewrote the block with CRLF line ends, called it "hand-edited" and cut a
    generation; every later apply cut another generation for a no-op."""
    repo = make_workspace_repo(sandbox / "env", BLOCK_REPO)
    (repo / "block.sh").write_bytes(b"export A=1\r\r\r\nexport B=2\n")
    bashrc = sandbox / ".bashrc"
    bashrc.write_bytes(b"# mine\n")
    apply(repo)
    first = bashrc.read_bytes()
    generation = selected(sandbox)
    assert first.startswith(b"# mine\n") and b"\r" not in first

    for _ in range(2):
        out = apply(repo)
        assert "hand-edited" not in out.stdout, out.stdout
        assert bashrc.read_bytes() == first
        assert selected(sandbox) == generation


def test_rollback_restores_a_managed_block_and_keeps_foreign_bytes(sandbox):
    repo = make_workspace_repo(sandbox / "env", BLOCK_REPO)
    (repo / "block.sh").write_text("export A=1\n")
    bashrc = sandbox / ".bashrc"
    bashrc.write_bytes(b"# mine\r\n")
    apply(repo)
    first = bashrc.read_bytes()
    (repo / "block.sh").write_text("export A=2\n")
    apply(repo)
    assert b"export A=2" in bashrc.read_bytes()
    bashrc.write_bytes(bashrc.read_bytes() + b"# appended later\r\n")

    out = grip("rollback", cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr
    assert selected(sandbox) == 1
    assert bashrc.read_bytes() == first + b"# appended later\r\n"


def test_rollback_refuses_a_block_payload_that_does_not_reproduce_its_record(sandbox):
    """plan/0056 §16.7 deviation 2: rollback spliced the retained block payload
    without comparing it with the block's recorded hash. A record its retained
    payload does not reproduce (an older receipt in another hash domain, here a
    rewritten manifest standing in for one) is now refused before the flip."""
    repo = make_workspace_repo(sandbox / "env", BLOCK_REPO)
    (repo / "block.sh").write_text("export A=1\n")
    bashrc = sandbox / ".bashrc"
    bashrc.write_bytes(b"# mine\n")
    apply(repo)
    (repo / "block.sh").write_text("export A=2\n")
    apply(repo)
    second = bashrc.read_bytes()

    path = sandbox / STATE / "generations/1/manifest.json"
    manifest = json.loads(path.read_text())
    (entry,) = [e for m in manifest["modules"].values() for e in m["entries"] if e["to"] == "~/.bashrc"]
    entry["hash"] = bytes_hash(b"export A=something-else")
    path.write_text(json.dumps(manifest))

    out = grip("rollback", cwd=repo)
    assert out.returncode != 0, out.stdout + out.stderr
    assert "cannot restore" in out.stderr and ".bashrc" in out.stderr
    assert selected(sandbox) == 2
    assert bashrc.read_bytes() == second
