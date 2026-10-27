"""0042: stateful execution/source/retention journeys, separate from 0038.

Every seed visits the contract families, but their order, pending source edits
and payloads vary. The oracle records commits, not declarations: update and
failed apply are never permission to advance the deployed-content model.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import random
import shutil
import stat
import subprocess
from typing import Any, Literal

import pytest
from conftest import grip, make_workspace_repo, make_tarball


Fault = Literal["none", "source", "rendered", "post_link"]
SEEDS = (42, 2026, 31337)


@dataclass(frozen=True)
class Commit:
    contents: tuple[bytes, bytes]
    current: Path
    manifest: bytes


@dataclass
class Model:
    desired: bytes
    pinned: bytes
    committed: Commit | None = None


def declaration(payload: Path, fault: Fault = "none") -> str:
    """Native acquired content must pass explicit workspace file checks."""
    subject = "deployed" if fault == "post_link" else "source" if fault == "source" else "rendered"
    stage = "post_link" if fault == "post_link" else "pre_flip"
    command = "/usr/bin/true" if fault == "none" else "/usr/bin/false"
    return f'''import {{pkg,provider,tarball,profile,file,artifactFile,identity,
  trackedCopyTo,check,exec,lit}} from "@gripsack/core";
export default [
  pkg("recipe-payload", {{producer:provider(tarball({json.dumps(payload.as_uri())})),
    commands:{{}},target:{{os:"linux",arch:"x86_64"}},layout:{{kind:"relocatable"}}}}),
  check("validate", {{subject:"recipe",run:exec({{argv:[lit({json.dumps(command)})]}})}}),
  profile("recipe", {{files:[file({{source:artifactFile("recipe-payload","input"),
    content:identity(),destination:trackedCopyTo("~/.journey/recipe"),
    checks:[{{check:"validate",subject:{json.dumps(subject)},stage:{json.dumps(stage)}}}]}})]}}),
];
'''


def read_lock(repo: Path) -> tuple[Path, dict[str, Any]]:
    path = repo / "gripsack.lock"
    data = json.loads(path.read_text())
    assert data["lock_version"] == 2
    resolutions = list(data["resolutions"].values())
    assert len(resolutions) == 1
    pins = {pin["output"]: pin for pin in resolutions[0]["pins"]}
    assert set(pins) == {"source-payload", "recipe-payload"}
    return path, pins


def _make_removable(path: Path) -> None:
    """Published store trees are immutable; make one removable first."""
    for entry in [path, *path.rglob("*")]:
        if not entry.is_symlink():
            entry.chmod(entry.stat().st_mode | stat.S_IWUSR | stat.S_IRUSR
                        | (stat.S_IXUSR if entry.is_dir() else 0))


def remove_store(store: Path) -> None:
    """Evict the whole store. Legal ONLY while no generation roots it:
    the update-time source cache is unrooted until the first apply, and
    0042 C permits its eviction; once generations exist, GC must retain
    every generation-rooted path (store-verify enforces exactly that).
    """
    if not store.exists():
        return
    _make_removable(store)
    shutil.rmtree(store)


def evict_generation_paths(home: Path) -> list[Path]:
    """Evict current profile artifacts, never distinct historical roots."""
    wire = json.loads((home / "current/manifest.json").read_bytes())
    generation = wire.get("generation", wire)
    paths = list({Path(state["store_path"]) for state in generation["modules"].values()})
    for path in paths:
        if path.exists():
            _make_removable(path)
            shutil.rmtree(path)
    return paths


class World:
    def __init__(self, root: Path, seed: int) -> None:
        self.root = root
        self.home = root / ".local/share/gripsack"
        self.rng = random.Random(seed)
        self.seed = seed
        self.trace: list[str] = []
        self.payload = root / "payload.tar.gz"
        self.model = Model(b"initial\n", b"initial\n")
        make_tarball(self.payload, {"input": self.model.desired})
        self.repo = make_workspace_repo(root / "env", {
            "recipe": declaration(self.payload),
            "source": f'''import {{pkg,provider,tarball,profile,file,artifactFile,
  identity,trackedCopyTo}} from "@gripsack/core";
export default [
 pkg("source-payload", {{producer:provider(tarball({json.dumps(self.payload.as_uri())})),
   commands:{{}},target:{{os:"linux",arch:"x86_64"}},layout:{{kind:"relocatable"}}}}),
 profile("source", {{files:[file({{source:artifactFile("source-payload","input"),
   content:identity(),destination:trackedCopyTo("~/.journey/source")}})]}}),
];''',
        })
        self.destinations = (root / ".journey/source", root / ".journey/recipe")
        self.lock: Path | None = None

    def command(self, *args: str, error: str | None = None) -> subprocess.CompletedProcess[str]:
        result = grip(*args, cwd=self.repo)
        diagnostic = f"seed={self.seed} trace={self.trace}\n{result.stdout}\n{result.stderr}"
        if error is None:
            assert result.returncode == 0, diagnostic
        else:
            assert result.returncode != 0, diagnostic
            if error:
                assert error in result.stdout + result.stderr, diagnostic
        return result

    def snapshot(self) -> Commit:
        current = self.home / "current"
        return Commit(
            tuple(path.read_bytes() for path in self.destinations),
            current.readlink(),
            (current / "manifest.json").read_bytes(),
        )

    def check_committed(self) -> None:
        assert self.model.committed is not None
        assert self.snapshot() == self.model.committed

    def lock_bytes(self) -> bytes:
        assert self.lock is not None
        return self.lock.read_bytes()

    def edit(self) -> None:
        self.model.desired = f"seed={self.seed}; revision={self.rng.getrandbits(64)}\n".encode()
        make_tarball(self.payload, {"input": self.model.desired})
        self.check_committed()

    def update(self) -> None:
        self.command("update", "--host", "testhost")
        self.lock, data = read_lock(self.repo)
        digest = hashlib.sha256(self.payload.read_bytes()).hexdigest()
        for name in ("source-payload", "recipe-payload"):
            pin = data[name]["resolved"]
            assert pin["sha256"] == digest, (name, pin)
        # Surveys resolve bytes but never publish profile artifacts or deploy.
        self.model.pinned = self.model.desired
        if self.model.committed is not None:
            self.check_committed()
        else:
            assert not (self.home / "current").exists()
            assert all(not path.exists() for path in self.destinations)

    def apply(self) -> None:
        before = self.lock_bytes()
        self.command("apply", "--host", "testhost")
        assert tuple(path.read_bytes() for path in self.destinations) == (
            self.model.pinned, self.model.pinned,
        )
        assert self.lock_bytes() == before, "apply must not complete update's pins"
        self.model.committed = self.snapshot()

    def healthy(self) -> None:
        self.check_committed()
        self.command("check", "--host", "testhost")
        # strict: gc/store-verify must retain and verify every
        # generation-rooted path; the cold actions below never remove
        # anything a retained generation still references
        self.command("store-verify")

    def fail_and_heal(self, fault: Fault) -> None:
        # Always exercise a new candidate, even if preceding actions left the
        # world satisfied. Desired contents remain distinct until healing.
        self.edit()
        self.update()
        committed = self.model.committed
        before = self.lock_bytes()
        output = self.repo / "outputs/recipe.ts"
        output.write_text(declaration(self.payload, fault))
        for _ in range(2):
            # The state contract is the oracle: the run fails, and the
            # committed generation, its destinations and the lock stay
            # byte-identical. Which diagnostic phrased it is not.
            self.command("apply", "--host", "testhost", error="")
            assert self.model.committed == committed
            self.check_committed()
            assert self.lock_bytes() == before
        output.write_text(declaration(self.payload))
        self.apply()
        assert self.model.committed != committed

    def reconstruct(self, cold: bool) -> None:
        # A pending file edit is intentionally not an update. Reconstruction
        # here uses the last acquired source, so keep the local archive at its
        # pinned version for a genuinely offline cold acquisition.
        if self.model.desired != self.model.pinned:
            self.update()
            self.apply()
        before = self.lock_bytes()
        committed = self.model.committed
        if cold:
            # Remove only current profile artifacts; historical roots remain.
            evict_generation_paths(self.home)
            # trackedCopy destinations survive cache eviction independently.
            self.check_committed()
        self.apply()
        assert self.lock_bytes() == before
        assert self.model.committed == committed

    def corrupt_and_heal(self, active: bool) -> None:
        # Ensure a retained, non-current generation exists regardless of the
        # preceding shuffled actions or GC's retention policy.
        self.edit()
        self.update()
        self.apply()
        current = (self.home / "current/manifest.json").resolve()
        retained = sorted(path for path in (self.home / "generations").rglob("manifest.json")
                          if path.resolve() != current)
        assert retained, "journey must have historical generations"
        victim = current if active else self.rng.choice(retained)
        original = victim.read_bytes()
        before = self.lock_bytes()
        committed = self.model.committed
        victim.write_bytes(b'{"modules":')
        try:
            self.command("gc", error="")
            if active:
                self.command("apply", "--host", "testhost", error="")
            # Invalid history authorizes neither collection nor deployment.
            assert victim.read_bytes() == b'{"modules":'
            assert committed is not None
            assert tuple(path.read_bytes() for path in self.destinations) == committed.contents
            assert (self.home / "current").readlink() == committed.current
            assert self.lock_bytes() == before
            if not active:
                self.check_committed()
        finally:
            victim.write_bytes(original)
        self.check_committed()
        self.apply()
        assert self.model.committed == committed
        self.command("gc")
        self.check_committed()


@pytest.mark.parametrize("seed", SEEDS)
def test_seeded_contract_journey(sandbox: Path, seed: int) -> None:
    world = World(sandbox, seed)
    world.trace.append("bootstrap:update+cold-apply")
    world.update()
    # The update-time cache is unrooted until the first apply — the one
    # whole-store eviction 0042 C permits. The first apply is therefore
    # COLD: it reconstructs the pinned sources and must not touch the
    # lock (world.apply asserts byte-stability).
    remove_store(world.home / "store")
    world.apply()
    # Balanced coverage without twelve independent scenario tests. Shuffling
    # changes which actions encounter pending desired edits, warm failed
    # recipes, retained generations, and already acquired source revisions.
    actions = [
        "edit", "edit", "update_apply", "update_apply",
        "source", "rendered", "post_link", "rendered",
        "warm", "cold", "retained", "current",
    ]
    world.rng.shuffle(actions)
    for index, action in enumerate(actions):
        world.trace.append(f"{index}:{action}")
        try:
            if action == "edit":
                world.edit()
            elif action == "update_apply":
                world.edit()
                world.update()
                world.apply()
            elif action in {"source", "rendered", "post_link"}:
                world.fail_and_heal(action)
            elif action in {"warm", "cold"}:
                world.reconstruct(cold=action == "cold")
            else:
                world.corrupt_and_heal(active=action == "current")
            world.healthy()
        except AssertionError as exc:
            raise AssertionError(f"seed={seed} trace={world.trace}\n{exc}") from exc
