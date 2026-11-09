#!/usr/bin/env python3
"""Build, verify and (only when told to) publish a coordinated gripsack release.

    release.py prepare [--helper-from DIR]   build every asset into ./dist via docker compose
    release.py verify [--dist DIR] [--no-smoke]
                                             check the assets; run the installed-consumer smoke
    release.py status CANDIDATE              post `local-practical-qualification` for a
                                             qualified Buildbot candidate's commit (host only)
    release.py publish CANDIDATE --notes F [--yes]
                                             publish that candidate's qualified bytes as
                                             GitHub release core-vX.Y.Z (dry run without --yes)

`prepare` and `verify` are the `package` and `smoke` gates of .ci/pilot.json, so a
green qualification has already run them on the exact source. `publish` never
rebuilds: it copies the qualified artifacts out of the ci-infra archive, checks
their recorded digests, re-runs the artifact checks and uploads them unchanged.
Core, Conda helper and SDK share one version; the tag is `core-vX.Y.Z`.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import re
import os
from pathlib import Path
import shlex
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.error
import urllib.request
import zlib

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-unknown-linux-musl"
REPOSITORY = "gripsack-dev/gripsack"
CONTEXT = "local-practical-qualification"
CI_STATE = Path.home() / ".local/share/buildbot-ci"
HELPER_NOTICES = ("LICENSE-pixi", "LICENSE-UV-MIT", "LICENSE-rattler")


class ReleaseError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise ReleaseError(message)


# --- pure logic (unit-tested in test_release.py) -----------------------------

def coordinated_version(core: str, helper: str, sdk: str) -> str:
    require(core == helper == sdk,
            f"core {core}, helper {helper} and SDK {sdk} versions must match")
    return core


def asset_names(version: str) -> list[str]:
    """The published asset set, in upload order."""
    core = f"gripsack-{version}-{TARGET}.tar.gz"
    helper = f"gripsack-conda-{version}-{TARGET}"
    return [core, f"{core}.sha256", helper, f"{helper}.sha256", f"{helper}.manifest.json",
            *(f"{helper}.{notice}" for notice in HELPER_NOTICES),
            f"gripsack-core-{version}.tgz", "install.sh"]


def sidecar_ok(text: str, name: str, digest: str) -> bool:
    return text == f"{digest}  {name}\n"


def tag_for(version: str) -> str:
    return f"core-v{version}"


def elf_facts(data: bytes) -> dict:
    """Static-linkage and section facts from an x86_64 ELF64 image."""
    require(data[:6] == b"\x7fELF\x02\x01" and data[18:20] == b"\x3e\x00",
            "not a little-endian x86_64 ELF64 executable")
    phoff, shoff = struct.unpack_from("<QQ", data, 0x20)
    phentsize, phnum, shentsize, shnum, shstrndx = struct.unpack_from("<HHHHH", data, 0x36)
    segments = []
    for index in range(phnum):
        kind, _flags, offset, _vaddr, _paddr, filesz = struct.unpack_from(
            "<IIQQQQ", data, phoff + index * phentsize)
        segments.append((kind, offset, filesz))
    needed = 0
    for kind, offset, filesz in segments:
        if kind == 2:  # PT_DYNAMIC: static-pie keeps it, but must need nothing
            for entry in range(offset, offset + filesz, 16):
                tag, _value = struct.unpack_from("<qQ", data, entry)
                if tag == 0:
                    break
                needed += tag == 1  # DT_NEEDED
    sections = {}
    if shnum:
        str_offset = struct.unpack_from("<Q", data, shoff + shstrndx * shentsize + 0x18)[0]
        for index in range(shnum):
            base = shoff + index * shentsize
            name_offset = struct.unpack_from("<I", data, base)[0]
            offset, size = struct.unpack_from("<QQ", data, base + 0x18)
            start = str_offset + name_offset
            name = data[start:data.index(b"\0", start)].decode()
            sections[name] = data[offset:offset + size]
    return {"interp": any(kind == 3 for kind, _, _ in segments),
            "load": any(kind == 1 for kind, _, _ in segments),
            "needed": needed, "sections": sections}


def check_static(data: bytes, label: str) -> dict:
    facts = elf_facts(data)
    require(facts["load"] and not facts["interp"] and not facts["needed"],
            f"{label}: must be a static executable (no interpreter or shared libraries)")
    return facts


def embedded_sbom(facts: dict) -> dict:
    """cargo-auditable's dependency list (.dep-v0, zlib-compressed JSON)."""
    require(".dep-v0" in facts["sections"], "core binary has no embedded SBOM (.dep-v0)")
    sbom = json.loads(zlib.decompress(facts["sections"][".dep-v0"]))
    roots = [package for package in sbom.get("packages", []) if package.get("root")]
    require([package.get("name") for package in roots] == ["gripsack"],
            "embedded SBOM does not describe the gripsack crate")
    return sbom


def qualification_artifacts(qualification: dict, candidate: dict, candidate_id: str) -> list[dict]:
    """The qualified artifact records, checked against the frozen candidate."""
    require(qualification.get("kind") == "trusted-local-pilot-qualification"
            and qualification.get("candidate_id") == candidate_id
            and candidate.get("candidate_id") == candidate_id,
            "qualification record does not belong to this candidate")
    profile = candidate["inputs"]["profile"]
    require(profile["project"] == "gripsack", "candidate is not a gripsack candidate")
    require([gate["gate"] for gate in qualification["gates"]] == [gate["id"] for gate in profile["gates"]],
            "qualification does not cover every profile gate")
    expected = ["dist/" + name for name in asset_names(profile["version"])]
    actual = [item["source_path"] for item in qualification["artifacts"]]
    require(sorted(actual) == sorted(expected), f"qualified artifacts {actual} != release assets {expected}")
    return qualification["artifacts"]


# --- process helpers ----------------------------------------------------------

def run(argv, **kwargs):
    argv = [str(arg) for arg in argv]
    print("+ " + shlex.join(argv), flush=True)
    return subprocess.run(argv, check=True, cwd=kwargs.pop("cwd", ROOT), **kwargs)


def capture(argv, **kwargs) -> str:
    return run(argv, stdout=subprocess.PIPE, text=True, **kwargs).stdout.strip()


def compose(*args, profile=None):
    run(["docker", "compose", *(["--profile", profile] if profile else []), *args])


def compose_captured(*args, profile=None) -> str:
    """A compose run whose output is returned instead of streamed, for runs
    that happen concurrently: the log stays readable, one build after another."""
    argv = ["docker", "compose", *(["--profile", profile] if profile else []), *args]
    completed = subprocess.run([str(arg) for arg in argv], cwd=ROOT, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True)
    text = "+ " + shlex.join(map(str, argv)) + "\n" + completed.stdout
    if completed.returncode:
        raise ReleaseError(f"{shlex.join(map(str, argv))} failed ({completed.returncode}):\n{text}")
    return text


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def workspace_version() -> str:
    core = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    helper = tomllib.loads((ROOT / "tools/conda-helper/Cargo.toml").read_text())["package"]["version"]
    sdk = json.loads((ROOT / "typescript/package.json").read_text())["version"]
    return coordinated_version(core, helper, sdk)


def owner() -> list[str]:
    return ["-e", f"HOST_UID={os.getuid()}", "-e", f"HOST_GID={os.getgid()}"]


# --- prepare -------------------------------------------------------------------

def tag_published(version: str) -> bool:
    """Whether `version`'s release is public. Asked of GitHub over HTTPS, not
    `origin`: a qualification checkout's origin is a frozen bundle with no tags."""
    helper = asset_names(version)[2]
    url = f"https://github.com/{REPOSITORY}/releases/download/{tag_for(version)}/{helper}.sha256"
    request = urllib.request.Request(url, method="HEAD")
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status == 200
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return False
        raise


def pinned_helper_digest() -> str:
    text = (ROOT / "crates/gripsack-fetch/src/conda_pins.rs").read_text()
    match = re.search(r"LinuxX86_64Musl,\s*\"([0-9a-f]{64})\"", text)
    require(match is not None, "no linux-x86_64-musl helper pin in conda_pins.rs")
    return match.group(1)


def fetch_published_helper(version: str, directory: Path):
    """Download the published helper set for `version`; require the pinned digest."""
    directory.mkdir(parents=True, exist_ok=True)
    base = f"https://github.com/{REPOSITORY}/releases/download/{tag_for(version)}"
    for name in asset_names(version)[2:8]:
        with urllib.request.urlopen(f"{base}/{name}", timeout=120) as response:
            (directory / name).write_bytes(response.read())
    helper = asset_names(version)[2]
    require(sha256(directory / helper) == pinned_helper_digest(),
            f"published {helper} does not match the committed pin")
    print(f"reusing published {helper} ({pinned_helper_digest()[:12]}…, matches the pin)")


def prepare(args):
    version = workspace_version()
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    for name in asset_names(version):
        try:
            (dist / name).unlink(missing_ok=True)
        except PermissionError:
            raise ReleaseError(f"dist/{name} is not removable (root-owned?); clean dist/ via docker")
    core = ["run", "--build", "--rm", "-e", f"VERSION={version}", "-e", f"TARGET={TARGET}", *owner(), "release"]
    helper = f"gripsack-conda-{version}-{TARGET}"
    helper_from = args.helper_from
    if not helper_from and tag_published(version):
        # Between releases the version's helper is already published and
        # pinned; main ships those bytes, so reuse them (pin-checked) rather
        # than rebuild. The `test` gate still builds, lints and tests the
        # helper source on every candidate. Only a release-bump candidate
        # (unpublished version) rebuilds and measures it (plan/0057).
        helper_from = ROOT / "dist" / ".published-helper"
        fetch_published_helper(version, helper_from)
    if helper_from:
        args.helper_from = helper_from
        compose(*core)
        # Reuse previously measured bytes; verify still checks them against the pin.
        for name in asset_names(version)[2:8]:
            shutil.copyfile(args.helper_from / name, dist / name)
        (dist / helper).chmod(0o755)
    elif not os.environ.get("GRIPSACK_RELEASE_PARALLEL"):
        # Default: one build at a time, so the gate's peak memory is one
        # cargo's worth (plan/0057 D6; two concurrent release builds are the
        # leading suspect for the measurement-host reboots recorded there).
        compose(*core)
        compose("run", "--build", "--rm", *owner(), "helper", profile="tools")
    else:
        # GRIPSACK_RELEASE_PARALLEL=1: the core and the helper are independent
        # images with independent outputs; build both at once and print both
        # logs in order. Both logs reach the evidence record even when one
        # build fails; the first failure is raised once both have been printed.
        with ThreadPoolExecutor(max_workers=2) as pool:
            builds = [pool.submit(compose_captured, *core),
                      pool.submit(compose_captured, "run", "--build", "--rm", *owner(), "helper", profile="tools")]
        failed = [build for build in builds if build.exception() is not None]
        for build in builds:
            print(build.result() if build not in failed else build.exception(), end="", flush=True)
        if failed:
            raise ReleaseError(f"{len(failed)} of the concurrent core/helper builds failed (logs above)")
    compose("run", "--build", "--rm", "--user", f"{os.getuid()}:{os.getgid()}",
            "-e", "HOME=/tmp/home", "-v", f"{dist}:/out", "tools", "sh", "-ec",
            "cp -r /app/typescript /tmp/sdk && cd /tmp/sdk"
            " && npm ci --no-audit --no-fund && npm run build"
            " && npm pack --silent --pack-destination /out", profile="tools")
    shutil.copyfile(ROOT / "install.sh", dist / "install.sh")
    print(f"prepared {len(asset_names(version))} assets for {version} in {dist}")


# --- verify --------------------------------------------------------------------

def verify_assets(dist: Path, version: str, installer: bytes):
    names = asset_names(version)
    present = sorted(path.name for path in dist.iterdir() if not path.name.startswith("."))
    require(present == sorted(names), f"dist inventory {present} != expected {sorted(names)}")
    for name in names:
        require((dist / name).is_file() and not (dist / name).is_symlink(), f"{name}: not a regular file")
    core, helper, sdk = names[0], names[2], names[8]
    for name in (core, helper):
        require(sidecar_ok((dist / f"{name}.sha256").read_text(), name, sha256(dist / name)),
                f"{name}: checksum sidecar mismatch")

    root = f"gripsack-{version}-{TARGET}"
    with tarfile.open(dist / core, "r:gz") as archive:
        members = archive.getmembers()
        require(sorted(member.name for member in members) in ([root, f"{root}/grip"], [f"{root}/grip"]),
                f"core tarball must contain only {root}/grip")
        entry = archive.getmember(f"{root}/grip")
        require(entry.isfile() and entry.mode & 0o111, "core tarball grip is not an executable file")
        grip = archive.extractfile(entry).read()
    sbom = embedded_sbom(check_static(grip, "grip"))
    with tempfile.TemporaryDirectory() as scratch:
        binary = Path(scratch) / "grip"
        binary.write_bytes(grip)
        binary.chmod(0o755)
        reported = capture([binary, "--version"])
    require(reported == f"grip {version}", f"core reports {reported!r}, want 'grip {version}'")

    check_static((dist / helper).read_bytes(), helper)
    manifest = json.loads((dist / f"{helper}.manifest.json").read_text())
    for key, value in {"version": version, "target": TARGET, "asset": helper,
                       "sha256": sha256(dist / helper), "bytes": (dist / helper).stat().st_size}.items():
        require(manifest.get(key) == value, f"helper manifest {key} mismatch")
    require(manifest.get("licenses") == {f"{helper}.{notice}": sha256(dist / f"{helper}.{notice}")
                                         for notice in HELPER_NOTICES}, "helper license inventory mismatch")

    with tarfile.open(dist / sdk, "r:gz") as archive:
        package = json.load(archive.extractfile("package/package.json"))
    require((package.get("name"), package.get("version")) == ("@gripsack/core", version),
            f"SDK package is {package.get('name')}@{package.get('version')}, want @gripsack/core@{version}")
    require((dist / "install.sh").read_bytes() == installer, "install.sh differs from the source installer")
    print(f"verified {len(names)} assets for {version}: static core with "
          f"{len(sbom['packages'])}-package embedded SBOM, helper, SDK, installer")


def external_checkouts() -> dict[str, Path]:
    """Fresh checkout of the pinned website repository."""
    pins = json.loads((ROOT / ".ci/pilot.json").read_text())["external_inputs"]["repositories"]
    base = ROOT / "local-ci/external"
    checkouts = {}
    for name, pin in pins.items():
        checkout = base / name
        if checkout.exists():
            shutil.rmtree(checkout)
        checkout.mkdir(parents=True)
        run(["git", "init", "-q", checkout])
        run(["git", "-C", checkout, "fetch", "-q", "--depth=1", pin["url"], pin["commit"]])
        run(["git", "-C", checkout, "-c", "advice.detachedHead=false", "checkout", "-q", "FETCH_HEAD"])
        require(capture(["git", "-C", checkout, "rev-parse", "HEAD"]) == pin["commit"],
                f"{name} did not resolve to its pinned commit")
        checkouts[name] = checkout
    return checkouts


def smoke(dist: Path):
    """Install the exact artifacts into private homes and drive real consumers."""
    checkouts = external_checkouts()
    report = ROOT / "local-ci"
    report.mkdir(exist_ok=True)
    commit = capture(["git", "rev-parse", "HEAD"])
    compose("run", "--build", "--rm", "--user", f"{os.getuid()}:{os.getgid()}",
            "-e", "HOME=/tmp/home", "-e", f"GRIPSACK_CANDIDATE_COMMIT={commit}",
            "-v", f"{dist.resolve()}:/app/dist:ro", "-v", f"{report}:/app/local-ci",
            *(arg for name, path in checkouts.items() for arg in ("-v", f"{path}:/app/{name}:ro")),
            "tools", "python3", "scripts/local_artifact_consumers.py", "--dist", "/app/dist",
            "--site", "/app/site",
            "--report", "/app/local-ci/consumers.json", profile="tools")


def verify(args):
    version = workspace_version()
    dist = (args.dist or ROOT / "dist").resolve()
    verify_assets(dist, version, (ROOT / "install.sh").read_bytes())
    if not args.no_smoke:
        smoke(dist)


# --- status / publish (host only; uses the operator's gh credentials) ---------

def host_only():
    require(not Path("/.dockerenv").exists() and "CI_CANDIDATE_ID" not in os.environ,
            "status/publish use the operator's GitHub credentials; run them on the host")


def load_candidate(state: Path, candidate_id: str):
    require(len(candidate_id) == 64 and all(c in "0123456789abcdef" for c in candidate_id),
            "candidate id must be the 64-hex id printed by ci_infra qualify")
    qualification = state / "archive" / candidate_id / "qualification.json"
    require(qualification.is_file(), f"{candidate_id} is not qualified (no {qualification})")
    candidate = json.loads((state / "inbox" / candidate_id / "candidate.json").read_text())
    record = json.loads(qualification.read_text())
    artifacts = qualification_artifacts(record, candidate, candidate_id)
    return candidate["inputs"], record, artifacts


def status(args):
    host_only()
    inputs, record, _ = load_candidate(args.state, args.candidate)
    description = f"Buildbot candidate {args.candidate[:16]}: {len(record['gates'])} gates passed"
    run(["gh", "api", "-X", "POST", f"repos/{REPOSITORY}/statuses/{inputs['commit']}",
         "-f", "state=success", "-f", f"context={CONTEXT}", "-f", f"description={description}"],
        stdout=subprocess.DEVNULL)
    print(f"{CONTEXT}=success on {inputs['commit']}")


def publish(args):
    host_only()
    inputs, _, artifacts = load_candidate(args.state, args.candidate)
    version, commit = inputs["profile"]["version"], inputs["commit"]
    tag = tag_for(version)
    require(args.notes.is_file(), f"release notes {args.notes} not found")
    require(not capture(["git", "ls-remote", "--tags", "origin", f"refs/tags/{tag}"]),
            f"{tag} already exists; tags are never moved")
    run(["git", "fetch", "-q", "origin", "main"])
    require(subprocess.run(["git", "merge-base", "--is-ancestor", commit, "FETCH_HEAD"], cwd=ROOT).returncode == 0,
            f"{commit} is not on origin/main")
    statuses = json.loads(capture(["gh", "api", f"repos/{REPOSITORY}/commits/{commit}/status"]))["statuses"]
    require(any(item["context"] == CONTEXT and item["state"] == "success" for item in statuses),
            f"{commit} has no successful {CONTEXT} status (run `release.py status` first)")

    dist = ROOT / "dist" / f"release-{version}"
    if dist.exists():
        shutil.rmtree(dist)
    dist.mkdir(parents=True)
    evidence = args.state / "archive" / args.candidate / "evidence"
    for item in artifacts:
        source = evidence / item["gate"] / item["attempt"] / "artifacts" / item["source_path"]
        target = dist / Path(item["source_path"]).name
        shutil.copyfile(source, target)
        require(sha256(target) == item["sha256"] and target.stat().st_size == item["size_bytes"],
                f"{target.name}: archived bytes differ from the qualification record")
    installer = subprocess.run(["git", "show", f"{commit}:install.sh"], cwd=ROOT, check=True,
                               stdout=subprocess.PIPE).stdout
    verify_assets(dist, version, installer)

    files = [dist / name for name in asset_names(version)]
    command = ["gh", "release", "create", tag, "--repo", REPOSITORY, "--target", commit,
               "--title", f"Gripsack {version}", "--notes-file", args.notes, *files]
    if not args.yes:
        print("dry run; re-run with --yes to publish:\n  " + shlex.join(map(str, command)))
        return
    run(command)
    with tempfile.TemporaryDirectory() as readback:
        run(["gh", "release", "download", tag, "--repo", REPOSITORY, "--dir", readback])
        downloaded = sorted(path.name for path in Path(readback).iterdir())
        require(downloaded == sorted(asset_names(version)), f"published asset set differs: {downloaded}")
        for path in files:
            require(sha256(Path(readback) / path.name) == sha256(path), f"{path.name}: published bytes differ")
    print(f"published {tag} ({len(files)} assets, byte-identical readback)")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("prepare", help="build every release asset into ./dist")
    p.add_argument("--helper-from", type=Path, help="reuse measured helper bytes from this directory")
    p.set_defaults(func=prepare)
    v = commands.add_parser("verify", help="check assets and run the installed-consumer smoke")
    v.add_argument("--dist", type=Path)
    v.add_argument("--no-smoke", action="store_true", help="artifact checks only")
    v.set_defaults(func=verify)
    for name, func, text in (("status", status, "post the qualification status for a candidate"),
                             ("publish", publish, "publish a qualified candidate (dry run without --yes)")):
        c = commands.add_parser(name, help=text)
        c.add_argument("candidate")
        c.add_argument("--state", type=Path, default=CI_STATE, help="ci-infra state root")
        c.set_defaults(func=func)
        if name == "publish":
            c.add_argument("--notes", type=Path, required=True)
            c.add_argument("--yes", action="store_true", help="actually create the GitHub release")
    args = parser.parse_args(argv)
    try:
        args.func(args)
    except (ReleaseError, subprocess.CalledProcessError, OSError, KeyError, ValueError) as error:
        print(f"release: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
