"""Fixture scaffolding: the workspace repo each example runs in, the
packaged SDK installed into every repo's node_modules (the
deliberate-pin rule — the real package, never a shadow copy of the
frontend sources), and isolated homes."""

from __future__ import annotations

import io
import json
import shutil
import stat
import tarfile
from pathlib import Path

from .extract import Block, CheckFailure

CORE_PKG_DIR = Path("node_modules/@gripsack/core")



# --------------------------------------------------------------------------
# the packaged SDK
# --------------------------------------------------------------------------

def prepare_sdk(tarball: Path, base: Path, expect_version: str | None) -> Path:
    """Extract the npm tarball once; return the package directory.

    Validates it is really @gripsack/core with a resolvable export
    entry, and that its version matches the frontend the driver comes
    from (candidate mode: built from that same tree; published mode:
    the deliberately pinned release) — version skew fails here, not
    as a confusing eval error later.
    """
    dest = base / "sdk-package"
    with tarfile.open(tarball, "r:gz") as tar:
        tar.extractall(dest, filter="data")
    pkg = dest / "package"
    meta_path = pkg / "package.json"
    if not meta_path.is_file():
        raise CheckFailure(f"{tarball}: no package.json inside the tarball")
    meta = json.loads(meta_path.read_text(encoding="utf-8"))
    if meta.get("name") != "@gripsack/core":
        raise CheckFailure(f"{tarball}: package name {meta.get('name')!r}, want @gripsack/core")
    if expect_version is not None and meta.get("version") != expect_version:
        raise CheckFailure(
            f"{tarball}: package version {meta.get('version')!r} != frontend "
            f"version {expect_version!r} — the SDK artifact and the driver "
            "tree must be the same release"
        )
    dot = (meta.get("exports") or {}).get(".", {})
    entry = dot.get("import") or dot.get("default") if isinstance(dot, dict) else dot
    entry = entry or meta.get("main") or "index.js"
    if not (pkg / entry).is_file():
        raise CheckFailure(f"{tarball}: export entry {entry!r} missing from the tarball")
    return pkg


def install_sdk(repo: Path, sdk_pkg: Path) -> None:
    """Copy the extracted package into the repo's node_modules — the
    deliberate pin: both the standalone driver eval and the real core
    resolve @gripsack/core to THIS copy (pin.ts prefers the repo
    install; the core's sandbox grants the resolved pin its read)."""
    target = repo / CORE_PKG_DIR
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(sdk_pkg, target)


def sdk_version_of(pkg: Path) -> str:
    return json.loads((pkg / "package.json").read_text(encoding="utf-8"))["version"]


# --------------------------------------------------------------------------
# scaffold repos
# --------------------------------------------------------------------------

def write_fixture(repo: Path, rel: str, spec) -> None:
    target = repo / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(spec, str):
        target.write_text(spec, encoding="utf-8")
    elif "executable" in spec:
        write_executable(target, spec["executable"])
    elif "payload" in spec:
        with tarfile.open(target, "w:gz") as tar:
            for name, content in spec["payload"].items():
                info = tarfile.TarInfo(name)
                info.size = len(content)
                info.mode = 0o755
                tar.addfile(info, io.BytesIO(content))
    else:
        raise CheckFailure(f"unknown fixture spec for {rel}")


def write_executable(path: Path, body: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)




def scaffold(entry: dict, block: Block, base: Path, sdk_pkg: Path) -> Path:
    repo = base / "repos" / entry["id"]
    repo.mkdir(parents=True, exist_ok=True)
    if entry["kind"] != "workspace":
        raise CheckFailure(f"{entry['id']}: only complete root workspaces are executable")
    for rel, spec in entry.get("fixtures", {}).items():
        write_fixture(repo, rel, spec)
    (repo / "gripsack.ts").write_text(block.code, encoding="utf-8")
    install_sdk(repo, sdk_pkg)
    return repo


def fresh_home(base: Path, name: str) -> Path:
    home = base / "homes" / name
    (home / ".local/share/gripsack").mkdir(parents=True, exist_ok=True)
    return home
