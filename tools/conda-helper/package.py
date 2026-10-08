#!/usr/bin/env python3
"""Build native release assets twice with a pinned compiler and the helper lock."""
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
    ("Linux", "aarch64"): "aarch64-unknown-linux-musl",
}
SLOTS = dict(zip(TARGETS.values(), (
    "LinuxX86_64Musl", "LinuxAarch64Musl",
)))
# Historical pin tables may retain retired platform variants; never package them.
PIN_VARIANTS = (*SLOTS.values(), "MacosAarch64", "MacosX86_64")
REQUIRED = {"LinuxX86_64Musl", "LinuxAarch64Musl"}


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
    if not REQUIRED <= pins.keys():
        raise SystemExit(f"missing mandatory Conda pins: {sorted(REQUIRED - pins.keys())}")
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
        "protocol": 3, "payload": {"kind": "error", "attempt": None, "code": "protocol"}
    }:
        raise SystemExit(f"unexpected native helper smoke result: {message}")


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", required=True, type=Path)
    parser.add_argument("--target", required=True, choices=sorted(TARGETS.values()))
    parser.add_argument("--check", action="store_true", help="require exact committed release pins")
    args = parser.parse_args()
    if TARGETS.get((platform.system(), platform.machine())) != args.target:
        parser.error("release qualification requires a native worker for the target")
    here = Path(__file__).resolve().parent
    repo = here.parent.parent
    dist = args.dist.resolve()
    if dist.is_relative_to(repo):
        parser.error("release artifacts must live outside the source repository")
    version = tomllib.loads((here / "Cargo.toml").read_text())["package"]["version"]
    core_version = tomllib.loads((repo / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if version != core_version or os.environ.get("VERSION", version) != version:
        parser.error("helper, core workspace, and requested release versions must match")
    if platform.system() == "Linux" and not os.environ.get("GRIPSACK_CONDA_PACKAGING_CONTAINER"):
        dist.mkdir(parents=True, exist_ok=True)
        subprocess.run([
            "docker", "run", "--rm", "-v", f"{repo}:/src:ro", "-v", f"{dist}:/out",
            "-e", "GRIPSACK_CONDA_PACKAGING_CONTAINER=1", LINUX_IMAGE,
            "sh", "-ec", 'apk add --no-cache python3 build-base binutils cmake perl linux-headers git xz-dev bzip2-dev zstd-dev && exec python3 /src/tools/conda-helper/package.py --dist /out --target "$1" ${2:+"$2"}',
            "package", args.target, *(["--check"] if args.check else []),
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
        for round_name in ("a", "b"):
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
        if digest != sha(binaries[1]):
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
    manifest = {"schema": 1, "version": version, "target": args.target, "asset": name, "sha256": digest, "bytes": (dist / name).stat().st_size, "compiler": compiler, "lock_sha256": lock, "reproducible_rounds": 2}
    if platform.system() == "Linux":
        manifest["build_image"] = LINUX_IMAGE
    notices = {}
    for suffix, source in (("LICENSE-pixi", "LICENSE"), ("LICENSE-UV-MIT", "LICENSE-UV-MIT")):
        notice = dist / f"{name}.{suffix}"
        shutil.copyfile(here / "vendor/pixi_git" / source, notice)
        notices[notice.name] = sha(notice)
    manifest["licenses"] = notices
    (dist / f"{name}.manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (dist / f"{name}.sha256").write_text(f"{digest}  {name}\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
