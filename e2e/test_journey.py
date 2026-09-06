"""The journey harness (plan/0038): seeded random user journeys —
declare/apply/drift/take-over/undeclare/redeclare/rollback/update —
against the real binary, with an expectation model of what the
semantics SAY the filesystem should hold after every step.

This covers the seam the scenario tests don't: state evolving across
TIME. A red run prints its seed; replay it, then promote the failing
sequence to a named regression test.
"""

import random

import pytest
from conftest import grip, make_env_repo, make_tarball, refresh_host

SEEDS = [7, 42, 1337, 2026, 31337]
STEPS = 30

COPY_MOD = """
import { module, trackedCopy } from "@gripsack/core";

export default module("copy", {
  config: { "configs/copy/a.conf": trackedCopy("%(copy_to)s") },
});
"""

LINK_MOD = """
import {{ fileFetch, module, symlink }} from "@gripsack/core";

export default module("tool", {{
  fetch: fileFetch("{payload}"),
  install: {{ "bin/tool": symlink("%(link_to)s") }},
}});
"""

MERGE_MOD = """
import { module, merge } from "@gripsack/core";

export default module("shell", {
  config: { "configs/shell/block.sh": merge("~/.bashrc") },
});
"""

HOOK_MOD = """
import { module, customHook } from "@gripsack/core";

export default module("hook", {
  activate: [customHook("echo tick >> ~/" + "hook.log")],
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


def module_source(repo_root, kind, to, content):
    if kind == "copy":
        return COPY_MOD % {"copy_to": to}
    if kind == "tool":
        payload = repo_root / "tool.tar.gz"
        return LINK_MOD.format(payload=payload) % {"link_to": to}
    if kind == "shell":
        return MERGE_MOD
    if kind == "hook":
        return HOOK_MOD
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

    copy_dest = sandbox / ".config/demo/a.conf"
    link_dest = sandbox / ".local/bin/tool"
    copy_abs = str(copy_dest)
    link_abs = str(link_dest)

    (repo_root / "configs" / "copy").mkdir(parents=True)
    (repo_root / "configs" / "shell").mkdir(parents=True)
    (repo_root / "configs" / "hook").mkdir(parents=True)
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

    present = {"copy": True, "tool": True, "shell": True, "hook": True}

    def write_repo():
        mods = {}
        if present["copy"]:
            mods["copy"] = module_source(repo_root, "copy", copy.to, None)
        if present["tool"]:
            mods["tool"] = module_source(repo_root, "tool", tool.to, None)
        if present["shell"]:
            mods["shell"] = MERGE_MOD
        if present["hook"]:
            mods["hook"] = HOOK_MOD
        (repo_root / "configs" / "copy" / "a.conf").write_text(copy.repo)
        (repo_root / "configs" / "shell" / "block.sh").write_text(merge.repo)
        # re-entrant: removed modules' files are DELETED (an
        # undeclared module is gone, not just unlisted) and the host
        # is refreshed — make_env_repo never cleans, by design
        moddir = repo_root / "modules"
        moddir.mkdir(parents=True, exist_ok=True)
        for stale in moddir.glob("*.ts"):
            if stale.stem not in mods:
                stale.unlink()
        for name, src in mods.items():
            (moddir / f"{name}.ts").write_text(src)
        refresh_host(repo_root)
        return mods

    gen = 0

    def apply(takeover=False):
        nonlocal gen
        args = ["apply", "--host", "testhost"] + (["--take-over"] if takeover else [])
        out = grip(*args, cwd=repo_root)
        assert out.returncode == 0, f"seed={seed} trace: {' '.join(trace)}\napply failed: {out.stderr}"
        # satisfied runs cut no generation — read the truth, never
        # count by hand
        import re

        m = re.search(r"generation (\d+)", out.stdout)
        assert m, f"seed={seed}: apply output has no generation: {out.stdout}"
        gen = int(m.group(1))
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
        # the link: present iff gripsack last deployed it
        if tool.managed is not None:
            assert link_dest.is_symlink(), f"seed={seed}: tool link missing"
            target = link_dest.readlink()
            assert "gripsack" in str(target) or "store" in str(target)
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
    tool.declare("tool-v0\n")
    make_env_repo(repo_root, {
        "copy": module_source(repo_root, "copy", copy.to, None),
        "tool": module_source(repo_root, "tool", tool.to, None),
        "shell": module_source(repo_root, "shell", None, None),
        "hook": module_source(repo_root, "hook", None, None),
    })
    trace = ["init+apply"]

    write_repo()
    apply()

    gen_content = {gen: (copy.managed, copy.declared, merge.managed, merge.declared)}

    for step in range(STEPS):
        op = rng.choice(
            [
                "edit_copy", "edit_merge", "edit_payload",
                "apply", "apply", "drift_copy", "drift_merge",
                "apply_takeover", "toggle_copy", "toggle_merge",
                "respell_copy", "update_payload", "rollback",
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
            make_tarball(payload, {b"bin/tool".decode(): f"#!/bin/sh\necho tool-v{rng.randint(1,9999)}\n".encode()})
            out = grip("update", "--host", "testhost", cwd=repo_root)
            assert out.returncode == 0, f"seed={seed} update: {out.stderr}"
            write_repo()
            gen = apply()
            gen_content[gen] = (copy.managed, copy.declared, merge.managed, merge.declared)
            check_world()
            continue
        elif op == "drift_copy" and copy_dest.exists():
            copy.drift(f"user-{rng.randint(1, 9999)}\n")
            copy_dest.write_text(copy.live)
        elif op == "drift_merge":
            text = bashrc.read_text() if bashrc.exists() else ""
            bashrc.write_text(text + f"# note {rng.randint(1,9999)}\n")
            merge.live = bashrc.read_text()
        elif op == "apply_takeover":
            write_repo()
            gen = apply(takeover=True)
            gen_content[gen] = (copy.managed, copy.declared, merge.managed, merge.declared)
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
            make_tarball(payload, {"bin/tool": f"#!/bin/sh\necho tool-v{rng.randint(1,9999)}\n".encode()})
            out = grip("update", "--host", "testhost", cwd=repo_root)
            assert out.returncode == 0, f"seed={seed} update: {out.stderr}"
            write_repo()
            gen = apply()
            gen_content[gen] = (copy.managed, copy.declared, merge.managed, merge.declared)
            check_world()
            continue
        elif op == "rollback" and gen > 1:
            target = rng.randint(1, gen - 1)
            out = grip("rollback", str(target), cwd=repo_root)
            assert out.returncode == 0, f"seed={seed} rollback {target}: {out.stderr}"
            if target in gen_content:
                cm, cd, mm, md = gen_content[target]
                copy.rolled_back(cm, cd)
                merge.rolled_back(mm, md)
            gen = target
            check_world()
            continue
        else:
            write_repo()
            gen = apply()
            gen_content[gen] = (copy.managed, copy.declared, merge.managed, merge.declared)
            check_world()
            continue
        check_world()
