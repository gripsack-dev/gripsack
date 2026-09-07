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
from conftest import grip, make_env_repo, make_tarball


Fault = Literal["none", "build", "run", "verify"]
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
    """Two independent output gates; both feed the installed artifact.

    Recipes are run steps with declared outputs (0007 §3, 0041): the
    postcondition is the contract. A fault makes its gate lie — the
    argv succeeds while the declared output never appears.
    """
    build = ["true"] if fault == "build" else ["cp", "input", "built"]
    argv = ["true"] if fault == "run" else ["cp", "built", "result"]
    verify = "exit 17" if fault == "verify" else "test -f result"
    return f'''import {{ module, fetchStep, fileFetch, runStep,
  installStep, trackedCopy, verifyShell }} from "@gripsack/core";
export default module("recipe", {{
  steps: [
    fetchStep(fileFetch({json.dumps(str(payload))})),
    runStep({json.dumps(build)}, "build", {{ needs: ["fetch"], outputs: ["built"] }}),
    runStep({json.dumps(argv)}, "run", {{ needs: ["build"], outputs: ["result"] }}),
    installStep({{ result: trackedCopy("~/.journey/recipe") }}, "install",
      {{ needs: ["run"] }}),
  ],
  verify: verifyShell({json.dumps(verify)}),
}});
'''


def read_lock(repo: Path) -> tuple[Path, dict[str, Any]]:
    """The host lock lives at locks/<host>.lock (0001 §4); read it
    directly. Requiring exactly our two resolved module entries also
    prevents accidentally accepting an unrelated receipt."""
    path = repo / "locks" / "testhost.lock"
    data = json.loads(path.read_text())
    modules = data.get("modules")
    assert isinstance(modules, dict) and set(modules) == {"source", "recipe"}, (
        f"expected one complete host lock at {path}, found {sorted(modules or [])}"
    )
    assert all(isinstance(value, dict) and isinstance(value.get("resolved"), dict)
               for value in modules.values())
    return path, data


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
    """Cold reconstruct, correctly scoped: remove ONLY the current
    generation's module artifacts — exactly what a pinned apply rebuilds
    — while every distinct historical root stays on disk."""
    manifest = json.loads((home / "current/manifest.json").read_bytes())
    paths = [Path(state["store_path"]) for state in manifest["modules"].values()]
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
        self.repo = make_env_repo(root / "env", {
            "recipe": declaration(self.payload),
            "source": f'''import {{ module, fetchStep, fileFetch, installStep,
  trackedCopy }} from "@gripsack/core";
export default module("source", {{ steps: [
  fetchStep(fileFetch({json.dumps(str(self.payload))})),
  installStep({{ input: trackedCopy("~/.journey/source") }}, "install",
    {{ needs: ["fetch"] }}),
] }});
''',
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
        for name in ("source", "recipe"):
            pin = data["modules"][name]["resolved"]
            assert pin["sha256"] == digest, (name, pin)
        # Source-only acquisition must publish its canonical tree at update
        # time; recipes must acquire their source without executing the build.
        tree = data["modules"]["source"]["resolved"]["tree256"]
        assert isinstance(tree, str) and len(tree) == 64
        assert all(c in "0123456789abcdef" for c in tree)
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
        module = self.repo / "modules/recipe.ts"
        module.write_text(declaration(self.payload, fault))
        for _ in range(2):
            # The state contract is the oracle: the run fails, and the
            # committed generation, its destinations and the lock stay
            # byte-identical. Which diagnostic phrased it is not.
            self.command("apply", "--host", "testhost", error="")
            assert self.model.committed == committed
            self.check_committed()
            assert self.lock_bytes() == before
        module.write_text(declaration(self.payload))
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
            # Cold reconstruct is correctly scoped (Main's ruling): evict
            # only the current generation's module artifacts — exactly
            # what this pinned apply rebuilds — never the distinct roots
            # retained history still references.
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
        "build", "run", "verify", "verify",
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
            elif action in {"build", "run", "verify"}:
                fault: Fault = {"build": "build", "run": "run", "verify": "verify"}[action]
                world.fail_and_heal(fault)
            elif action in {"warm", "cold"}:
                world.reconstruct(cold=action == "cold")
            else:
                world.corrupt_and_heal(active=action == "current")
            world.healthy()
        except AssertionError as exc:
            raise AssertionError(f"seed={seed} trace={world.trace}\n{exc}") from exc
