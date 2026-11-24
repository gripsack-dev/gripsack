"""#403 artifact symlinks stay inside their package; #404 published store is read-only."""

import os
import stat
import subprocess
from pathlib import Path

import pytest

from conftest import grip, make_toolchain_tarball, make_workspace_repo, native_target

TOOL = (
    b'#!/bin/sh\nhere=$(dirname "$(readlink -f "$0")")\n'
    b'cat "$here/../lib/sibling.txt"\n'
)


def workspace(sandbox, members, destination):
    payload = make_toolchain_tarball(sandbox / "env" / "tool.tar.gz", members)
    return make_workspace_repo(
        sandbox / "env",
        f"""
import {{ artifactFile, file, fileFetch, identity, pkg, profile, provider, symlinkTo, trackedCopyTo }} from "@gripsack/core";
export default [pkg("tool", {{ producer: provider(fileFetch("{payload.name}")), commands: {{}}, target: {native_target()}, layout: {{ kind: "relocatable" }} }}),
profile("main", {{ files: [
    file({{ source: artifactFile("tool", "bin/tool"), content: identity(), destination: {destination} }})
  ] }})];
""",
    )


def test_symlink_destination_links_into_the_realized_package(sandbox):
    repo = workspace(
        sandbox, {"bin/tool": TOOL, "lib/sibling.txt": b"sibling\n"}, 'symlinkTo("~/.local/bin/tool")'
    )
    applied = grip("apply", "--host", "testhost", cwd=repo)
    assert applied.returncode == 0, applied.stderr
    link = sandbox / ".local/bin/tool"
    assert link.is_symlink()
    real = Path(os.path.realpath(link))
    assert "workspace-files" not in str(real), real
    assert (real.parent.parent / "lib/sibling.txt").exists(), "link must reach the package prefix"
    ran = subprocess.run([str(link)], capture_output=True, text=True)
    assert ran.returncode == 0, ran.stderr
    assert ran.stdout == "sibling\n"
    owner = grip("why-owns", "~/.local/bin/tool", cwd=repo)
    assert owner.returncode == 0 and "main" in owner.stdout, owner.stdout + owner.stderr
    # the generation retains the package it links into
    assert grip("gc", cwd=repo).returncode == 0
    assert real.exists()
    assert subprocess.run([str(link)], capture_output=True, text=True).stdout == "sibling\n"


def test_tracked_copy_of_an_artifact_file_stays_a_copy(sandbox):
    repo = workspace(sandbox, {"bin/tool": TOOL}, 'trackedCopyTo("~/.local/bin/tool")')
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    copied = sandbox / ".local/bin/tool"
    assert copied.is_file() and not copied.is_symlink()
    assert copied.read_bytes() == TOOL


def test_published_store_entries_are_read_only_and_collectable(sandbox):
    repo = workspace(
        sandbox, {"bin/tool": TOOL, "lib/pkg/mod.py": b"x = 1\n"}, 'symlinkTo("~/.local/bin/tool")'
    )
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    store = sandbox / ".local/share/gripsack/store"
    directories = [
        path
        for entry in store.iterdir()
        if entry.is_dir() and entry.name.split("-", 1)[1].startswith(("workspace-", "package-payload"))
        for path in [entry, *(p for p in entry.rglob("*") if p.is_dir() and not p.is_symlink())]
    ]
    assert directories
    writable = [d for d in directories if stat.S_IMODE(d.lstat().st_mode) & 0o222]
    assert not writable, f"published store directories must be read-only: {writable[:3]}"
    if os.geteuid() != 0:
        target = next(store.glob("*/payload/lib/pkg"), None) or next(store.glob("*/lib/pkg"))
        with pytest.raises(PermissionError):
            (target / "__pycache__").mkdir()
    # an unchanged re-apply, store verification and gc all cope with sealed trees
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    assert grip("store-verify", cwd=repo).returncode == 0
    # undeclaring the package orphans its sealed entries; gc must remove them
    (sandbox / "env" / "outputs" / "hello.ts").write_text(
        'import { profile } from "@gripsack/core";\nexport default profile("main", { files: [] });\n'
    )
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    collected = grip("gc", cwd=repo)
    assert collected.returncode == 0, collected.stderr


def test_rollback_restores_the_link_into_the_package(sandbox):
    repo = workspace(
        sandbox, {"bin/tool": TOOL, "lib/sibling.txt": b"sibling\n"}, 'symlinkTo("~/.local/bin/tool")'
    )
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    link = sandbox / ".local/bin/tool"
    (sandbox / "env" / "outputs" / "hello.ts").write_text(
        'import { profile } from "@gripsack/core";\nexport default profile("main", { files: [] });\n'
    )
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    assert not link.exists() and not link.is_symlink()
    rolled = grip("rollback", cwd=repo)
    assert rolled.returncode == 0, rolled.stderr
    assert link.is_symlink()
    assert "workspace-files" not in os.path.realpath(link)
    assert subprocess.run([str(link)], capture_output=True, text=True).stdout == "sibling\n"
