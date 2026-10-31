#!/usr/bin/env python3
"""Build a pinned native release asset; independent reproducibility is opt-in."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import tomllib

COMPILER = "1.98.0"
LINUX_IMAGE = "rust:alpine@sha256:a10e64dd139b7387337c7fbe8aca31b959b57b2fd4c8ae20a02cf1d6ea424dce"
TARGETS = {
    ("Linux", "x86_64"): "x86_64-unknown-linux-musl",
}
SLOTS = {"x86_64-unknown-linux-musl": "LinuxX86_64Musl"}
PIN_VARIANTS = tuple(SLOTS.values())
REQUIRED = set(SLOTS.values())


def read_pins(path, toolchain):
    """Let Rust parse/evaluate Rust; consume a JSON projection of the constants."""
    with tempfile.TemporaryDirectory(prefix="conda-pins-") as directory:
        directory = Path(directory)
        source = directory / "pins.rs"
        source.write_text(
            "#![allow(dead_code)]\n"
            "mod host { #[derive(Debug)] pub enum AssetTarget {"
            + ",".join(PIN_VARIANTS) + "} }\n"
            + f"#[path = {json.dumps(str(path))}] mod pins;\n"
            + 'fn main() { println!("{:?}", pins::CONDA_VERSION);'
            + 'for (slot, hash) in pins::CONDA_SHA256 { println!("[\\"{:?}\\",{:?}]", slot, hash); } }\n'
        )
        binary = directory / "pins"
        subprocess.run(["rustc", *toolchain, "--edition=2024", str(source), "-o", str(binary)], check=True)
        rows = subprocess.check_output([str(binary)], text=True).splitlines()
    version = json.loads(rows[0])
    pins = {}
    for row in rows[1:]:
        slot, digest = json.loads(row)
        if slot in pins or len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest):
            raise SystemExit(f"duplicate or invalid Conda pin: {slot}")
        pins[slot] = digest
    if REQUIRED != pins.keys():
        raise SystemExit(f"Conda pin targets {sorted(pins)} != release targets {sorted(REQUIRED)}")
    return version, pins


def check_pin(version, target, digest, committed):
    pinned_version, pins = committed
    if version != pinned_version:
        raise SystemExit(f"Conda version {version} != committed {pinned_version}")
    if pins.get(SLOTS[target]) != digest:
        raise SystemExit(f"Conda pin mismatch or missing slot for {target}: measured {digest}")


def smoke(binary):
    result = subprocess.run([str(binary)], input=b"", capture_output=True, timeout=30)
    frame = result.stdout
    if len(frame) < 8 or int.from_bytes(frame[:8], "little") != len(frame) - 8:
        raise SystemExit("native helper did not return one complete protocol frame")
    message = json.loads(frame[8:])
    payload = message.get("payload") if isinstance(message, dict) else None
    detail = payload.pop("message", None) if isinstance(payload, dict) else None
    if result.returncode != 1 or not isinstance(detail, str) or message != {
        "protocol": 4, "payload": {"kind": "error", "attempt": None, "code": "protocol"}
    }:
        raise SystemExit(f"unexpected native helper smoke result: {message}")


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def portable_notice_sources(here):
    """Canonical redistribution inputs shared with the offline helper bundle."""
    return {
        "LICENSE-gripsack": here.parent.parent / "LICENSE",
        "LICENSE-pixi": here / "vendor/pixi_git/LICENSE",
        "LICENSE-UV-MIT": here / "vendor/pixi_git/LICENSE-UV-MIT",
        "LICENSE-rattler": here / "vendor/rattler_package_streaming/LICENSE",
    }


def check_portable_notices(here):
    embedded = here.parent.parent / "crates/gripsack-exec/src/workspace/sharing/notices"
    for name, source in portable_notice_sources(here).items():
        copy = embedded / name
        if not copy.is_file() or copy.read_bytes() != source.read_bytes():
            raise SystemExit(f"portable helper notice differs from canonical source: {name}")


def source_digest(here):
    """Bind helper and transitive local build inputs without hashing core/SDK."""
    here = here.resolve()
    repo = here.parent.parent
    workspace = tomllib.loads((repo / "Cargo.toml").read_text())["workspace"]
    inherited = {"package": {}, "dependencies": {}, "lints": {}}
    inputs = {here / "Cargo.lock", here / "package.py"}
    inputs.update(path for path in (here / "vendor").rglob("*") if path.is_file())
    # Notice copies are standalone-crate package inputs even though they are
    # not linked into the helper executable. Bind canonical and embedded bytes.
    embedded = repo / "crates/gripsack-exec/src/workspace/sharing/notices"
    for name, source in portable_notice_sources(here).items():
        inputs.update((source, embedded / name))
    pending = [here]
    visited = set()
    while pending:
        directory = pending.pop().resolve()
        if directory in visited:
            continue
        if not directory.is_relative_to(repo):
            raise SystemExit("helper local build input escapes the source repository")
        visited.add(directory)
        manifest_path = directory / "Cargo.toml"
        manifest = tomllib.loads(manifest_path.read_text())
        inputs.add(manifest_path)
        inputs.update(path for path in (directory / "src").rglob("*") if path.is_file())
        build = manifest.get("package", {}).get("build", "build.rs")
        if isinstance(build, str) and (directory / build).is_file():
            inputs.add(directory / build)
        for target in [manifest.get("lib", {}), *manifest.get("bin", [])]:
            if "path" in target:
                inputs.add(directory / target["path"])
        for key, value in manifest.get("package", {}).items():
            if isinstance(value, dict) and value.get("workspace") is True:
                inherited["package"][key] = workspace["package"][key]
        if manifest.get("lints", {}).get("workspace") is True:
            inherited["lints"] = workspace["lints"]
        tables = [manifest, *manifest.get("target", {}).values()]
        for table in tables:
            for section in ("dependencies", "build-dependencies"):
                for name, dependency in table.get(section, {}).items():
                    if not isinstance(dependency, dict):
                        continue
                    base = directory
                    if dependency.get("workspace") is True:
                        dependency = workspace["dependencies"][name]
                        inherited["dependencies"][name] = dependency
                        base = repo
                    if isinstance(dependency, dict) and "path" in dependency:
                        pending.append(base / dependency["path"])
        for patches in manifest.get("patch", {}).values():
            for dependency in patches.values():
                if isinstance(dependency, dict) and "path" in dependency:
                    pending.append(directory / dependency["path"])
    # The pin is checked independently against the measured binary. Hashing the
    # pin table into its own measurement would create a self-reference cycle.
    inputs.discard(repo / "crates/gripsack-fetch/src/conda_pins.rs")
    result = hashlib.sha256(json.dumps(inherited, sort_keys=True, separators=(",", ":")).encode())
    for path in sorted(inputs):
        if not path.resolve().is_relative_to(repo):
            raise SystemExit("helper local build input escapes the source repository")
        result.update(path.relative_to(repo).as_posix().encode() + b"\0")
        result.update(bytes.fromhex(sha(path)))
    return result.hexdigest()


def reuse(here, repo, source, dist, version, target, toolchain):
    """Import pinned bytes only when their measured build inputs still match."""
    name = f"gripsack-conda-{version}-{target}"
    manifest = json.loads((source / f"{name}.manifest.json").read_text())
    binary = source / name
    digest = sha(binary)
    check_pin(version, target, digest, read_pins(repo / "crates/gripsack-fetch/src/conda_pins.rs", toolchain))
    expected = {"schema": 1, "version": version, "target": target, "asset": name,
                "sha256": digest, "bytes": binary.stat().st_size,
                "lock_sha256": sha(here / "Cargo.lock"), "source_sha256": source_digest(here),
                "build_image": LINUX_IMAGE}
    if any(manifest.get(key) != value for key, value in expected.items()):
        raise SystemExit("existing helper has different build inputs or metadata; remeasure it")
    if (type(manifest.get("reproducible_rounds")) is not int
            or manifest["reproducible_rounds"] not in (1, 2)
            or not manifest.get("compiler", "").startswith(f"rustc {COMPILER} ")):
        raise SystemExit("existing helper has invalid measurement metadata")
    if (source / f"{name}.sha256").read_text() != f"{digest}  {name}\n":
        raise SystemExit("existing helper checksum sidecar differs")
    names = [name, f"{name}.sha256", f"{name}.manifest.json"]
    licenses = {}
    for suffix, relative in (("LICENSE-pixi", "pixi_git/LICENSE"),
                             ("LICENSE-UV-MIT", "pixi_git/LICENSE-UV-MIT"),
                             ("LICENSE-rattler", "rattler_package_streaming/LICENSE")):
        filename = f"{name}.{suffix}"
        if (source / filename).read_bytes() != (here / "vendor" / relative).read_bytes():
            raise SystemExit("existing helper license differs")
        licenses[filename] = sha(source / filename)
        names.append(filename)
    if manifest.get("licenses") != licenses:
        raise SystemExit("existing helper license inventory differs")
    subprocess.run(["python3", str(repo / "scripts/check_native_binary.py"), str(binary)], check=True)
    dist.mkdir(parents=True, exist_ok=True)
    for filename in names:
        shutil.copyfile(source / filename, dist / filename)
    (dist / name).chmod(0o755)
    smoke(dist / name)
    print(json.dumps(manifest, indent=2))

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", required=True, type=Path)
    parser.add_argument("--target", required=True, choices=sorted(TARGETS.values()))
    parser.add_argument("--check", action="store_true", help="require exact committed release pins")
    parser.add_argument("--reproducible", action="store_true",
                        help="explicitly build twice and compare independent outputs")
    parser.add_argument("--reuse-dist", type=Path,
                        help="reuse measured pinned bytes only if every helper source input is unchanged")
    args = parser.parse_args()
    if TARGETS.get((platform.system(), platform.machine())) != args.target:
        parser.error("release qualification requires a native worker for the target")
    if args.reuse_dist and args.reproducible:
        parser.error("--reuse-dist and --reproducible are mutually exclusive")
    here = Path(__file__).resolve().parent
    repo = here.parent.parent
    dist = args.dist.resolve()
    if dist.is_relative_to(repo):
        parser.error("release artifacts must live outside the source repository")
    version = tomllib.loads((here / "Cargo.toml").read_text())["package"]["version"]
    core_version = tomllib.loads((repo / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if version != core_version or os.environ.get("VERSION", version) != version:
        parser.error("helper, core workspace, and requested release versions must match")
    check_portable_notices(here)
    if args.reuse_dist:
        reuse(here, repo, args.reuse_dist.resolve(), dist, version, args.target,
              [] if os.environ.get("GRIPSACK_CONDA_PACKAGING_CONTAINER") else [f"+{COMPILER}"])
        return
    if platform.system() == "Linux" and not os.environ.get("GRIPSACK_CONDA_PACKAGING_CONTAINER"):
        dist.mkdir(parents=True, exist_ok=True)
        subprocess.run([
            "docker", "run", "--rm", "-v", f"{repo}:/src:ro", "-v", f"{dist}:/out",
            "-e", "GRIPSACK_CONDA_PACKAGING_CONTAINER=1", "-e", "CARGO_BUILD_JOBS", LINUX_IMAGE,
            "sh", "-ec", 'apk add --no-cache python3 build-base binutils cmake perl linux-headers git xz-dev bzip2-dev zstd-dev && exec python3 /src/tools/conda-helper/package.py --dist /out "$@"',
            "package", "--target", args.target, *(["--check"] if args.check else []),
            *(["--reproducible"] if args.reproducible else []),
        ], check=True)
        return
    lock = sha(here / "Cargo.lock")
    name = f"gripsack-conda-{version}-{args.target}"
    env = os.environ.copy()
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTFLAGS"):
        env.pop(key, None)
    env.update(SOURCE_DATE_EPOCH="0", CARGO_INCREMENTAL="0", TZ="UTC", LC_ALL="C")
    toolchain = [] if os.environ.get("GRIPSACK_CONDA_PACKAGING_CONTAINER") else [f"+{COMPILER}"]
    committed = read_pins(repo / "crates/gripsack-fetch/src/conda_pins.rs", toolchain) if args.check else None
    compiler = subprocess.check_output(["rustc", *toolchain, "-Vv"], text=True)
    if not compiler.startswith(f"rustc {COMPILER} "):
        raise SystemExit(f"expected Rust {COMPILER}, got {compiler}")
    subprocess.run(["python3", str(repo / "scripts/check_architecture.py"),
                    "--root", str(repo), "--resolved-manifest", str(here / "Cargo.toml"),
                    "--target", args.target], env=env, check=True)
    dist.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="conda-release-") as staging:
        staging = Path(staging)
        binaries = []
        for round_name in (("a", "b") if args.reproducible else ("a",)):
            target = staging / round_name
            flags = [f"--remap-path-prefix={repo}=/src", f"--remap-path-prefix={target}=/build", "-Cstrip=symbols"]
            cargo_home = Path(env.get("CARGO_HOME", Path.home() / ".cargo")).resolve()
            flags.append(f"--remap-path-prefix={cargo_home}=/cargo")
            env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(flags)
            subprocess.run(["cargo", *toolchain, "build", "--release", "--locked", "--manifest-path", str(here / "Cargo.toml"), "--target", args.target, "--target-dir", str(target)], env=env, check=True)
            binaries.append(target / args.target / "release" / "gripsack-conda")
        if sha(here / "Cargo.lock") != lock:
            raise SystemExit("helper lock changed during packaging")
        digest = sha(binaries[0])
        if args.reproducible and digest != sha(binaries[1]):
            raise SystemExit("independent helper builds differ; refusing artifact publication")
        subprocess.run(["python3", str(repo / "scripts/check_native_binary.py"),
                        str(binaries[0])], check=True)
        smoke(binaries[0])
        if committed is not None:
            check_pin(version, args.target, digest, committed)
        # Atomic publication from a private file on the destination filesystem.
        with tempfile.NamedTemporaryFile(dir=dist, prefix=".conda-", delete=False) as output:
            temporary = Path(output.name)
            with binaries[0].open("rb") as source:
                shutil.copyfileobj(source, output)
            output.flush()
            os.fchmod(output.fileno(), 0o755)
            os.fsync(output.fileno())
        temporary.replace(dist / name)
    manifest = {"schema": 1, "version": version, "target": args.target, "asset": name, "sha256": digest, "bytes": (dist / name).stat().st_size, "compiler": compiler, "lock_sha256": lock, "reproducible_rounds": 2 if args.reproducible else 1}
    manifest["source_sha256"] = source_digest(here)
    if platform.system() == "Linux":
        manifest["build_image"] = LINUX_IMAGE
    notices = {}
    for suffix, source in (
        ("LICENSE-pixi", "pixi_git/LICENSE"),
        ("LICENSE-UV-MIT", "pixi_git/LICENSE-UV-MIT"),
        ("LICENSE-rattler", "rattler_package_streaming/LICENSE"),
    ):
        notice = dist / f"{name}.{suffix}"
        shutil.copyfile(here / "vendor" / source, notice)
        notices[notice.name] = sha(notice)
    manifest["licenses"] = notices
    (dist / f"{name}.manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (dist / f"{name}.sha256").write_text(f"{digest}  {name}\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
