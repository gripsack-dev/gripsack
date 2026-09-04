"""E2E harness (plan/0003 §5, .agents/skills/gripsack-e2e).

Rules:
- everything under tmp_path; HOME and GRIPSACK_HOME are redirected there —
  a test can never touch the developer's real profile;
- GRIPSACK_TRUST_ALL=1 (plan/0013 D7): the suite never hits the trust
  prompt; the gate itself has dedicated tests that unset it;
- offline only: sources are file:// fixture tarballs built here, or local
  git repos. Network in e2e is a bug;
- the binary comes from GRIPSACK_BIN (set in docker; the gate stage has
  already compiled it — never rebuild from e2e);
- fixture env repos use the TypeScript frontend under the defineEnv
  contract (plan/0013 D5): each modules/<name>.ts default-exports its
  module value — module() constructs, it never registers — and
  hosts/<host>.ts imports the modules it wants and returns them from
  defineEnv. Falsy entries drop out; that is how tests undeclare.
"""

from __future__ import annotations

import io
import os
import re
import subprocess
import sys
import tarfile
from pathlib import Path

import pytest

GRIP = Path(os.environ.get("GRIPSACK_BIN", "target/debug/grip"))


def make_tarball(path: Path, files: dict[str, bytes]) -> Path:
    """Build a fixture tarball: module payload without the network."""
    with tarfile.open(path, "w:gz") as tar:
        for name, content in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            tar.addfile(info, io.BytesIO(content))
    return path


def refresh_host(repo: Path, host: str = "testhost") -> Path:
    """(Re)write hosts/<host>.ts importing every modules/*.ts — the
    defineEnv contract. Deterministic: files are globbed sorted, so the
    emitted module order (and the IR) is stable."""
    mods = sorted((repo / "modules").glob("*.ts")) if (repo / "modules").is_dir() else []
    # hyphens are legal filenames but not JS identifiers
    ident = lambda stem: re.sub(r"\W", "_", stem)  # noqa: E731
    imports = "".join(
        f'import {ident(p.stem)} from "../modules/{p.stem}.ts";\n' for p in mods
    )
    listing = ", ".join(ident(p.stem) for p in mods)
    (repo / "hosts").mkdir(exist_ok=True)
    (repo / "hosts" / f"{host}.ts").write_text(
        f'import {{ defineEnv }} from "@gripsack/core";\n'
        f"{imports}"
        f"\n"
        f"export default defineEnv((ctx) => ({{\n"
        f'  tags: ["test"],\n'
        f"  modules: [{listing}],\n"
        f"}}));\n"
    )
    return repo


def make_env_repo(
    root: Path,
    modules: dict[str, str] | str,
    host: str = "testhost",
) -> Path:
    """A fixture env repo mirroring plan/0001 §5, TypeScript frontend.

    `modules` maps a file basename under modules/ to file content; a
    bare string is shorthand for {"hello": ...}. Each file must
    default-export its module value (or an array of them)."""
    if isinstance(modules, str):
        modules = {"hello": modules}
    (root / "modules").mkdir(parents=True)
    (root / "hosts").mkdir()
    (root / "env.toml").write_text('[env]\nname = "fixture"\n')
    for name, ts in modules.items():
        (root / "modules" / f"{name}.ts").write_text(ts)
    return refresh_host(root, host=host)


def remove_module(repo: Path, name: str, host: str = "testhost") -> None:
    """Undeclare a module: delete its file and drop it from the host
    entrypoint (registration is explicit now — an unimported file is
    inert, but keeping the host honest is the point)."""
    (repo / "modules" / f"{name}.ts").unlink(missing_ok=True)
    refresh_host(repo, host=host)


@pytest.fixture
def sandbox(tmp_path, monkeypatch, request):
    """Redirect everything gripsack touches into tmp_path. On test
    failure, the grip run log (JSONL with causal spans — the debug
    skill's first stop) is printed before the sandbox evaporates:
    bazel keeps per-test logs for the same reason, and CI failures
    on a platform you can't run locally are otherwise archaeology."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("GRIPSACK_HOME", str(tmp_path / ".local/share/gripsack"))
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)
    # the trust gate (0013 D7) would prompt on every fixture repo;
    # CI is the documented bypass
    monkeypatch.setenv("GRIPSACK_TRUST_ALL", "1")
    yield tmp_path
    rep = getattr(request.node, "rep_call", None)
    if rep is not None and rep.failed:
        runs = tmp_path / ".local/share/gripsack/runs"
        logs = sorted(runs.glob("*.jsonl")) if runs.is_dir() else []
        if logs:
            tail = logs[-1].read_text(errors="replace").splitlines()[-25:]
            print(
                "\n--- grip run log ({}), last 25 lines ---\n{}".format(
                    logs[-1].name, "\n".join(tail)
                ),
                file=sys.stderr,
            )


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    """Stash the per-phase result on the node so the sandbox fixture
    can react to the call phase's outcome (see sandbox)."""
    outcome = yield
    rep = outcome.get_result()
    setattr(item, f"rep_{rep.when}", rep)


def grip(*args: str, cwd: Path | None = None) -> subprocess.CompletedProcess:
    assert GRIP.exists(), f"grip binary not found at {GRIP} (build first)"
    return subprocess.run(
        [str(GRIP.resolve()), *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=120,
    )

def _seed_plugin_store(sandbox, exe, fixture, tag="1.0"):
    """Pre-seed the managed plugin store as if a prior provision ran."""
    home = sandbox / ".local/share/gripsack"
    bindir = home / "plugins" / exe / tag
    bindir.mkdir(parents=True)
    (bindir / exe).write_text(fixture)
    (bindir / exe).chmod(0o755)
    (home / "plugins" / exe / "current").symlink_to(f"{tag}/")
    (home / "plugins" / "receipts").mkdir(parents=True)
    (home / "plugins" / "receipts" / f"{exe}.toml").write_text(
        f'source = "acme/{exe}"\ntag = "{tag}"\nsha256 = "ab"\n'
    )


def only_store_path(sandbox):
    store = sandbox / ".local/share/gripsack/store"
    entries = [p.name for p in store.iterdir()]
    assert len(entries) == 1, f"expected one store path, found {entries}"
    return entries[0]
