"""The journey harness (plan/0038): seeded random user journeys —
declare/apply/drift/take-over/undeclare/redeclare/rollback/update —
against the real binary, with an expectation model of what the
semantics SAY the filesystem should hold after every step.

This covers the seam the scenario tests don't: state evolving across
TIME. A red run prints its seed; replay it, then promote the failing
sequence to a named regression test.
"""

import json
import random

import pytest
from conftest import (
    grip,
    make_workspace_repo,
    make_tarball,
    refresh_workspace,
)

SEEDS = [7, 42, 1337, 2026, 31337]
STEPS = 30

COPY_OUTPUT = """
import { profile, file, repoFile, identity, trackedCopyTo } from "@gripsack/core";
export default profile("copy", {
  files: [file({source: repoFile("configs/copy/a.conf"), content: identity(),
    destination: trackedCopyTo("%(copy_to)s")})],
});
"""

LINK_OUTPUT = """
import {{ pkg, provider, tarball, profile, file, artifactFile, identity, symlinkTo }} from "@gripsack/core";
export default [
  pkg("tool-payload", {{producer: provider(tarball("{payload}")),
    commands: {{}}, target: {{os:"linux",arch:"x86_64"}}, layout: {{kind:"relocatable"}}}}),
  profile("tool", {{files: [file({{source: artifactFile("tool-payload", "bin/tool"),
    content: identity(), destination: symlinkTo("%(link_to)s")}})]}}),
];
"""

MERGE_OUTPUT = """
import { profile, file, repoFile, identity, managedBlock } from "@gripsack/core";
export default profile("shell", {
  files: [file({source: repoFile("configs/shell/block.sh"), content: identity(),
    destination: managedBlock("~/.bashrc", "shell")})],
});
"""


class Dest:
    """The expectation model for one destination: what the semantics
    SAY the live object should be after each step."""

    def __init__(self, to):
        self.to = to
        self.repo = ""
        self.declared = False
        self.managed = None  # what gripsack last deployed (content)
        self.drifted = False  # live diverged from managed
        self.live = None  # expected live content (None = absent)
        self.origin = None  # pre-adoption content (restored on prune)

    def declare(self, content):
        self.declared = True
        self.repo = content

    def undeclare(self):
        self.declared = False

    def drift(self, content):
        assert self.live is not None
        self.live = content
        self.drifted = True

    def applied(self, takeover=False):
        """The apply rule: preserved drift stands FOREVER (0029 §2 —
        until take-over or the user converges by hand); otherwise the
        repo content lands. Prune restores the origin (or absence)."""
        if not self.declared:
            if self.drifted:
                # preserved drift is never pruned (0029 §2) — the
                # user's file stands even after undeclare
                self.managed = None
                return
            self.live = self.origin
            self.managed = None
            self.drifted = False
            return
        if self.live is None:
            self.live = self.repo
        elif self.drifted and not takeover:
            return  # preserved: drift stands, the record authorizes nothing
        else:
            self.live = self.repo
        self.managed = self.repo
        self.drifted = False

    def rolled_back(self, managed_then, declared_then):
        """The rollback rule: the target generation's managed content
        lands; a destination undeclared there is restored to origin.
        A drifted destination is KEPT (0029 §2 — preserved records are
        never touched, and a drifted live object is not the current
        generation's intact content)."""
        if self.drifted:
            # kept; the record is now the target's — the next apply
            # still reads drift (live != target's record)
            self.managed = managed_then if declared_then else None
            return
        if declared_then:
            self.live = managed_then
            self.managed = managed_then
        else:
            self.live = self.origin
            self.managed = None
        self.drifted = False


def output_source(repo_root, kind, to):
    if kind == "copy":
        return COPY_OUTPUT % {"copy_to": to}
    if kind == "tool":
        return LINK_OUTPUT.format(payload=(repo_root / "tool.tar.gz").as_uri()) % {"link_to": to}
    if kind == "shell":
        return MERGE_OUTPUT
    raise AssertionError(f"unknown kind {kind}")


@pytest.mark.parametrize("seed", SEEDS)
def test_random_journey(sandbox, seed):
    """A seeded random journey. Failure prints the seed; replay it,
    then promote the failing sequence to a named regression test."""
    rng = random.Random(seed)
    repo_root = sandbox / "myenv"
    copy_to = "~/.config/demo/a.conf"
    link_to = "~/.local/bin/tool"
    bashrc = sandbox / ".bashrc"
    bashrc.write_text("# my bashrc\n")
    foreign_lines = ["# my bashrc\n"]

    copy_dest = sandbox / ".config/demo/a.conf"
    link_dest = sandbox / ".local/bin/tool"
    copy_abs = str(copy_dest)

    (repo_root / "configs" / "copy").mkdir(parents=True)
    (repo_root / "configs" / "shell").mkdir(parents=True)
    (repo_root / "configs" / "copy" / "a.conf").write_text("copy-v0\n")
    (repo_root / "configs" / "shell" / "block.sh").write_text("export V=0\n")
    payload = repo_root / "tool.tar.gz"
    make_tarball(payload, {"bin/tool": b"#!/bin/sh\necho tool-v0\n"})

    # destination models: copy and merge hold CONTENT; the link holds
    # a marker = the payload bytes it points at
    copy = Dest(copy_to)
    merge = Dest("~/.bashrc")
    merge.origin = "# my bashrc\n"
    merge.live = "# my bashrc\n"
    tool = Dest(link_to)

    present = {"copy": True, "tool": True, "shell": True}

    def write_repo():
        outputs = {}
        if present["copy"]:
            outputs["copy"] = output_source(repo_root, "copy", copy.to)
        if present["tool"]:
            outputs["tool"] = output_source(repo_root, "tool", tool.to)
        if present["shell"]:
            outputs["shell"] = MERGE_OUTPUT
        (repo_root / "configs" / "copy" / "a.conf").write_text(copy.repo)
        (repo_root / "configs" / "shell" / "block.sh").write_text(merge.repo)
        output_dir = repo_root / "outputs"
        output_dir.mkdir(parents=True, exist_ok=True)
        for stale in output_dir.glob("*.ts"):
            if stale.stem not in outputs:
                stale.unlink()
        for name, source in outputs.items():
            (output_dir / f"{name}.ts").write_text(source)
        refresh_workspace(repo_root)
        return outputs

    gen = 0

    def apply(takeover=False):
        nonlocal gen
        args = ["apply", "--host", "testhost"] + (["--take-over"] if takeover else [])
        out = grip(*args, cwd=repo_root)
        assert out.returncode == 0, f"seed={seed} trace: {' '.join(trace)}\napply failed: {out.stderr}"
        wire = json.loads((sandbox / ".local/share/gripsack/current/manifest.json").read_text())
        gen = wire.get("generation", wire)["number"]
        copy.applied(takeover)
        merge.applied(takeover)
        if present["tool"]:
            tool.declared = True
            tool.applied()
        else:
            tool.undeclare()
            tool.applied()
        return gen

    def check_world():
        try:
            _check_world()
        except AssertionError as e:
            raise AssertionError(f"seed={seed} trace: {' '.join(trace)}\n{e}")

    def _check_world():
        # content destinations
        if copy.live is None:
            assert not copy_dest.exists(), f"seed={seed}: copy should be absent"
        else:
            assert copy_dest.read_text() == copy.live, (
                f"seed={seed}: copy holds {copy_dest.read_text()!r}, expected {copy.live!r}"
            )
        if merge.live is None:
            assert not bashrc.exists() or "gripsack" not in bashrc.read_text()
        else:
            text = bashrc.read_text()
            # the MANAGED block (what gripsack last deployed) — repo
            # edits and undeclares change nothing on disk until apply
            block = (merge.managed or "").rstrip("\n")
            if block:
                assert block in text, (
                    f"seed={seed}: bashrc missing our block: {text!r}"
                )
            else:
                assert "gripsack" not in text, f"seed={seed}: merge block should be gone"
            assert all(line in text for line in foreign_lines), "foreign shell content was lost"
        # the link: present iff gripsack last deployed it
        if tool.managed is not None:
            assert link_dest.is_symlink(), f"seed={seed}: tool link missing"
            target = link_dest.readlink()
            assert "gripsack" in str(target) or "store" in str(target)
            assert link_dest.read_text() == tool.managed, "link points at the wrong retained revision"
        else:
            assert not link_dest.exists(), f"seed={seed}: tool link should be gone"
        # system health
        out = grip("check", "--host", "testhost", cwd=repo_root)
        assert out.returncode == 0, f"seed={seed}: check failed: {out.stderr}"
        out = grip("store-verify", cwd=repo_root)
        assert out.returncode == 0, f"seed={seed}: store verify: {out.stdout}"
        out = grip("generations", cwd=repo_root)
        assert out.returncode == 0

    copy.declare("copy-v0\n")
    merge.declare("export V=0\n")
    tool.declare("#!/bin/sh\necho tool-v0\n")
    make_workspace_repo(repo_root, {
        "copy": output_source(repo_root, "copy", copy.to),
        "tool": output_source(repo_root, "tool", tool.to),
        "shell": output_source(repo_root, "shell", None),
    })
    trace = ["init+update+apply"]

    write_repo()
    # tarball(file://...) is an external source even inside the repository.
    # Establish its pin explicitly before exercising frozen apply and rollback.
    out = grip("update", "--host", "testhost", cwd=repo_root)
    assert out.returncode == 0, f"seed={seed} initial update: {out.stderr}"
    apply()

    gen_content = {gen: (copy.managed, copy.declared, merge.managed, merge.declared, tool.managed)}

    for step in range(STEPS):
        op = rng.choice(
            [
                "edit_copy", "edit_merge", "edit_payload",
                "apply", "apply", "drift_copy", "drift_merge",
                "apply_takeover", "toggle_copy", "toggle_merge",
                "respell_copy", "update_payload",
                "rollback",
            ]
        )
        trace.append(f"{step}:{op}")
        if op == "edit_copy" and present["copy"]:
            copy.declare(f"copy-v{rng.randint(1, 9999)}\n")
            write_repo()
        elif op == "edit_merge" and present["shell"]:
            merge.declare(f"export V={rng.randint(1, 9999)}\n")
            write_repo()
        elif op == "edit_payload" and present["tool"]:
            tool.declare(f"#!/bin/sh\necho tool-v{rng.randint(1,9999)}\n")
            make_tarball(payload, {"bin/tool": tool.repo.encode()})
            out = grip("update", "--host", "testhost", cwd=repo_root)
            assert out.returncode == 0, f"seed={seed} update: {out.stderr}"
            write_repo()
            gen = apply()
            gen_content[gen] = (copy.managed, copy.declared, merge.managed, merge.declared, tool.managed)
            check_world()
            continue
        elif op == "drift_copy" and copy_dest.exists():
            copy.drift(f"user-{rng.randint(1, 9999)}\n")
            copy_dest.write_text(copy.live)
        elif op == "drift_merge":
            text = bashrc.read_text() if bashrc.exists() else ""
            note = f"# note {rng.randint(1,9999)}\n"
            foreign_lines.append(note)
            bashrc.write_text(text + note)
            merge.live = bashrc.read_text()
        elif op == "apply_takeover":
            write_repo()
            gen = apply(takeover=True)
            gen_content[gen] = (copy.managed, copy.declared, merge.managed, merge.declared, tool.managed)
            check_world()
            continue
        elif op == "toggle_copy":
            present["copy"] = not present["copy"]
            if present["copy"]:
                copy.declare(getattr(copy, "repo", "copy-v0\n"))
            else:
                copy.undeclare()
            write_repo()
        elif op == "toggle_merge":
            present["shell"] = not present["shell"]
            if present["shell"]:
                merge.declare(getattr(merge, "repo", "export V=0\n"))
            else:
                merge.undeclare()
            write_repo()
        elif op == "respell_copy" and present["copy"]:
            # the 0035 F1 class: tilde <-> absolute spelling
            copy.to = copy_abs if copy.to.startswith("~") else "~/.config/demo/a.conf"
            write_repo()
        elif op == "update_payload" and present["tool"]:
            tool.declare(f"#!/bin/sh\necho tool-v{rng.randint(1,9999)}\n")
            make_tarball(payload, {"bin/tool": tool.repo.encode()})
            out = grip("update", "--host", "testhost", cwd=repo_root)
            assert out.returncode == 0, f"seed={seed} update: {out.stderr}"
            write_repo()
            gen = apply()
            gen_content[gen] = (copy.managed, copy.declared, merge.managed, merge.declared, tool.managed)
            check_world()
            continue
        elif op == "rollback" and gen > 1:
            target = rng.randint(1, gen - 1)
            out = grip("rollback", str(target), cwd=repo_root)
            assert out.returncode == 0, f"seed={seed} rollback {target}: {out.stderr}"
            if target in gen_content:
                cm, cd, mm, md, tm = gen_content[target]
                copy.rolled_back(cm, cd)
                merge.rolled_back(mm, md)
                tool.rolled_back(tm, True)
            gen = target
            check_world()
            continue
        else:
            write_repo()
            gen = apply()
            gen_content[gen] = (copy.managed, copy.declared, merge.managed, merge.declared, tool.managed)
            check_world()
            continue
        check_world()
