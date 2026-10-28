#!/usr/bin/env python3
"""Consume a private Linux candidate; never publish or substitute public assets."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import xml.etree.ElementTree as ET

from examples_check.fixtures import install_sdk, prepare_sdk
from examples_check.extract import CheckFailure

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-unknown-linux-musl"


def external_repositories() -> dict[str, tuple[str, str]]:
    """The frozen profile is the sole external source-pin authority."""
    repositories = json.loads((ROOT / ".ci/pilot.json").read_text())["external_inputs"]["repositories"]
    return {name: (pin["url"].removesuffix(".git"), pin["commit"]) for name, pin in repositories.items()}


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def require(condition: bool, detail: str) -> None:
    if not condition:
        raise RuntimeError(detail)


def snapshot(root: Path) -> dict:
    """Content identities, not mutable access times; never follow directory aliases."""
    result = {}
    for path in sorted(root.rglob("*")):
        relative = str(path.relative_to(root))
        if path.is_symlink():
            result[relative] = {"symlink": os.readlink(path)}
        elif path.is_file():
            result[relative] = {"sha256": sha(path), "mode": path.stat().st_mode & 0o777}
    return result


class Consumer:
    """One candidate receipt and its private installed consumer state."""

    def __init__(self, args):
        self.args = args
        self.report = args.report.resolve()
        self.report.parent.mkdir(parents=True, exist_ok=True)
        self.work = Path(tempfile.mkdtemp(prefix="artifact-consumers-", dir=self.report.parent))
        self.work.chmod(0o700)
        self.logs = self.work / "commands"
        self.logs.mkdir()
        self.receipt = {
            "schema": 1, "status": "running", "work": str(self.work),
            "platform": platform.platform(), "commands": [], "artifacts": {},
            "limits": [
                "Private non-publishing candidate, not a signed or promoted release.",
                "Linux/WSL x86_64 only; no ARM, macOS, Windows, private el9 kernel or glibc hardware qualification.",
                "No fuzz engine or corpus replay (REL-FUZZ-2026-09-27).",
                "Bridge bytes/static admission only here; protected BuildKit solves belong to their separate gate.",
                "Tool-image/build caches may be retained; every command below executes anew in fresh consumer homes.",
            ],
        }
        # Do not inherit credentials, evaluator overrides, fault injection, or trust flags.
        self.env = {key: os.environ[key] for key in ("PATH", "LANG", "LC_ALL", "TZ", "RUSTUP_HOME") if key in os.environ}
        home = self.work / "home"
        home.mkdir(mode=0o700)
        temporary = self.work / "tmp"
        temporary.mkdir(mode=0o700)
        self.env.update(HOME=str(home), GRIPSACK_HOME=str(home / ".gs"), TMPDIR=str(temporary),
                        PYTHONDONTWRITEBYTECODE="1", PYTEST_DISABLE_PLUGIN_AUTOLOAD="1",
                        GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null", GIT_OPTIONAL_LOCKS="0")
        self.env["CARGO_HOME"] = str(home / ".cargo")
        self.grip = home / ".local/bin/grip"

    def run(self, argv, *, cwd=None, env=None, expected=0, input=None):
        argv = list(map(str, argv))
        directory = Path(cwd or ROOT)
        environment = self.env if env is None else env
        number = len(self.receipt["commands"])
        prefix = self.logs / f"{number:04d}"
        command = {"argv": argv, "cwd": str(directory), "expected_returncode": expected,
                   "environment": environment, "stdout": str(prefix.with_suffix(".stdout")),
                   "stderr": str(prefix.with_suffix(".stderr"))}
        self.receipt["commands"].append(command)
        if input is not None:
            input_path = prefix.with_suffix(".stdin")
            input_path.write_bytes(input.encode() if isinstance(input, str) else input)
            command["stdin"] = str(input_path)
            command["stdin_sha256"] = sha(input_path)
        with prefix.with_suffix(".stdout").open("wb") as out, prefix.with_suffix(".stderr").open("wb") as err:
            process = subprocess.run(argv, cwd=directory, env=environment,
                                     input=input.encode() if isinstance(input, str) else input,
                                     stdout=out, stderr=err)
        command["returncode"] = process.returncode
        stdout = prefix.with_suffix(".stdout").read_bytes()
        stderr = prefix.with_suffix(".stderr").read_bytes()
        print(f"[{number}] rc={process.returncode} {argv}", flush=True)
        require(process.returncode == expected,
                f"command {number}: expected {expected}, got {process.returncode}; {stderr.decode(errors='replace')[-4000:]}")
        return stdout.decode()

    def approve(self, repo: Path, env=None):
        require(repo.resolve().is_relative_to(self.work), "approval outside disposable reviewed fixture")
        inspected = json.loads(self.run([self.grip, "trust", "inspect", str(repo), "--json"], cwd=repo, env=env))
        require(Path(inspected["repository"]).resolve() == repo.resolve(), "inspection repository mismatch")
        self.run([self.grip, "trust", "add", str(repo), "--bundle", inspected["bundle_digest"],
                  "--policy", inspected["policy_digest"]], cwd=repo, env=env)
        self.receipt.setdefault("approvals", []).append({
            "repository": str(repo), "bundle_digest": inspected["bundle_digest"],
            "policy_digest": inspected["policy_digest"],
        })

    def source_identity(self):
        require(platform.system() == "Linux" and platform.machine() == "x86_64", "Linux x86_64 required")
        commit = os.environ.get("GRIPSACK_CANDIDATE_COMMIT")
        if (ROOT / ".git").exists():
            actual = self.run(["git", "rev-parse", "HEAD"]).strip()
            require(commit is None or commit == actual, "candidate commit differs from source checkout")
            commit = actual
            require(not self.run(["git", "status", "--porcelain", "--untracked-files=no"]).strip(), "tracked candidate source is dirty")
        require(commit is not None and len(commit) == 40 and all(c in "0123456789abcdef" for c in commit),
                "require exact GRIPSACK_CANDIDATE_COMMIT when image has no .git")
        image = os.environ.get("GRIPSACK_TOOL_IMAGE", "")
        require(image.startswith("sha256:") and len(image) == 71, "require immutable GRIPSACK_TOOL_IMAGE image ID")
        self.receipt["source"] = {"commit": commit, "tool_image": image, "contracts": {}}
        for relative in ("Cargo.toml", "Cargo.lock", "typescript/package.json", "typescript/package-lock.json",
                         "tools/conda-helper/Cargo.toml", "tools/conda-helper/Cargo.lock",
                         "crates/gripsack-fetch/src/conda_pins.rs", "crates/gripsack-fetch/src/bridge_pins.rs",
                         "scripts/local_artifact_consumers.py", "scripts/local_ci_conda_consumer.py", ".ci/pilot.json"):
            self.receipt["source"]["contracts"][relative] = sha(ROOT / relative)
        for tool in ("python3", "node", "npm", "deno", "rustc", "git", "readelf", "zstd"):
            self.run([tool, "--version"])
        self.env["GRIPSACK_DENO"] = str(Path(shutil.which("deno", path=self.env["PATH"])).resolve())
        self.receipt["external_inputs"] = {}
        for name, (url, expected) in external_repositories().items():
            source = getattr(self.args, name).resolve()
            git = ["git", "-c", f"safe.directory={source}"]
            actual = self.run([*git, "rev-parse", "HEAD"], cwd=source).strip()
            require(actual == expected, f"{name} commit {actual} != {expected}")
            require(not self.run([*git, "status", "--porcelain", "--untracked-files=all", "--ignored"], cwd=source).strip(),
                    f"{name} contains unreviewed changes or extra files")
            self.receipt["external_inputs"][name] = {"repository": url, "commit": actual}

    def artifacts(self):
        dist = self.args.dist.resolve()
        version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        helper_version = tomllib.loads((ROOT / "tools/conda-helper/Cargo.toml").read_text())["package"]["version"]
        sdk_version = json.loads((ROOT / "typescript/package.json").read_text())["version"]
        require(version == helper_version, "core/helper source versions differ")
        # Rust evaluates its own pin constants; no regex interpretation of Rust source.
        projection = self.work / "pins.rs"
        projection.write_text(
            '#![allow(dead_code)]\nmod host { #[derive(Debug)] pub enum AssetTarget { LinuxX86_64Musl } }\n'
            + f'#[path={json.dumps(str(ROOT / "crates/gripsack-fetch/src/conda_pins.rs"))}] mod conda;\n'
            + f'#[path={json.dumps(str(ROOT / "crates/gripsack-fetch/src/bridge_pins.rs"))}] mod bridge;\n'
            + 'fn main() { println!("{:?}", conda::CONDA_VERSION); println!("{:?}", bridge::BRIDGE_VERSION); '
            + 'for (name, pins) in [("conda", conda::CONDA_SHA256), ("bridge", bridge::BRIDGE_SHA256)] {'
            + 'for (slot, hash) in pins { println!("[\\\"{}\\\",\\\"{:?}\\\",{:?}]", name, slot, hash); } } }\n')
        self.run(["rustc", "--edition=2024", projection, "-o", self.work / "pins"])
        rows = self.run([self.work / "pins"]).splitlines()
        require(json.loads(rows[0]) == version, "helper source pin version differs")
        bridge_version = json.loads(rows[1])
        pins = {}
        for row in rows[2:]:
            kind, slot, digest = json.loads(row)
            require((kind, slot) not in pins, "duplicate source pin")
            pins[kind, slot] = digest
        require(set(pins) == {("conda", "LinuxX86_64Musl"), ("bridge", "LinuxX86_64Musl")}, "unexpected helper target inventory")
        core = f"gripsack-{version}-{TARGET}.tar.gz"
        helper = f"gripsack-conda-{version}-{TARGET}"
        require(bridge_version == version, "bridge release namespace differs from core")
        bridge = f"grip-buildkit-bridge-{bridge_version}-{TARGET}"
        sdk = f"gripsack-core-{sdk_version}.tgz"
        notices = {"LICENSE-pixi": "pixi_git/LICENSE", "LICENSE-UV-MIT": "pixi_git/LICENSE-UV-MIT",
                   "LICENSE-rattler": "rattler_package_streaming/LICENSE"}
        expected = {core, helper, bridge, sdk, *(f"{name}.sha256" for name in (core, helper, bridge)),
                    f"{helper}.manifest.json", *(f"{helper}.{suffix}" for suffix in notices)}
        actual = {path.name for path in dist.iterdir()}
        require(actual == expected, f"DIST inventory mismatch: missing={expected - actual}, unexpected={actual - expected}")
        for name in sorted(expected):
            path = dist / name
            require(path.is_file() and not path.is_symlink(), f"not a regular artifact: {name}")
            self.receipt["artifacts"][name] = {"sha256": sha(path), "bytes": path.stat().st_size}
        for name in (core, helper, bridge):
            require((dist / f"{name}.sha256").read_text() == f"{sha(dist / name)}  {name}\n", f"checksum sidecar mismatch: {name}")
        for kind, name in (("conda", helper), ("bridge", bridge)):
            require(sha(dist / name) == pins[kind, "LinuxX86_64Musl"], f"{kind} differs from compiled source pin")
        # Transported artifacts are immutable data, not executable files.
        # Install the exact verified helper into this private consumer home.
        self.helper = self.work / helper
        shutil.copyfile(dist / helper, self.helper)
        self.helper.chmod(0o555)
        require(sha(self.helper) == sha(dist / helper), "installed helper differs from artifact")
        self.run([sys.executable, "-c",
                  "import runpy,sys; from pathlib import Path; runpy.run_path(sys.argv[1])['smoke'](Path(sys.argv[2]))",
                  ROOT / "tools/conda-helper/package.py", self.helper])
        spec = importlib.util.spec_from_file_location("conda_package", ROOT / "tools/conda-helper/package.py")
        package = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(package)
        manifest = json.loads((dist / f"{helper}.manifest.json").read_text())
        for key, value in {"schema": 1, "version": version, "target": TARGET, "asset": helper,
                           "sha256": sha(dist / helper), "bytes": (dist / helper).stat().st_size,
                           "lock_sha256": sha(ROOT / "tools/conda-helper/Cargo.lock"),
                           "reproducible_rounds": 2, "build_image": package.LINUX_IMAGE}.items():
            require(manifest.get(key) == value, f"helper manifest {key} mismatch")
        require(manifest["compiler"].startswith(f"rustc {package.COMPILER} "), "helper compiler identity mismatch")
        require(set(manifest["licenses"]) == {f"{helper}.{suffix}" for suffix in notices}, "helper license inventory mismatch")
        for suffix, relative in notices.items():
            path = dist / f"{helper}.{suffix}"
            require(path.read_bytes() == (ROOT / "tools/conda-helper/vendor" / relative).read_bytes(), f"license bytes differ: {suffix}")
            require(manifest["licenses"][path.name] == sha(path), f"license digest differs: {suffix}")
        extracted = self.work / "release"
        with tarfile.open(dist / core, "r:gz") as archive:
            members = archive.getmembers()
            binaries = [entry for entry in members if entry.name == f"gripsack-{version}-{TARGET}/grip"]
            require(len(binaries) == 1 and binaries[0].isfile() and binaries[0].mode & 0o111 != 0,
                    "core archive must contain exactly its named executable")
            require(all(entry.isdir() or (entry.isfile() and Path(entry.name).parts[0] == f"gripsack-{version}-{TARGET}") for entry in members),
                    "core archive contains links/special files or foreign roots")
            require(len({entry.name for entry in members}) == len(members), "duplicate core archive member")
            archive.extractall(extracted, filter="data")
        self.grip.parent.mkdir(parents=True)
        shutil.copy2(extracted / f"gripsack-{version}-{TARGET}/grip", self.grip)
        self.grip.chmod(0o755)
        for binary in (self.grip, self.helper, dist / bridge):
            with binary.open("rb") as stream:
                header = stream.read(20)
            require(header[:6] == b"\x7fELF\x02\x01" and header[18:20] == b"\x3e\x00",
                    f"not a little-endian x86_64 ELF executable: {binary}")
            self.run(["readelf", "-h", binary])
            self.run(["file", binary])
            self.run([sys.executable, ROOT / "scripts/check_native_binary.py", binary])
        require(self.run([self.grip, "--version"]).strip() == f"grip {version}", "installed core version mismatch")
        self.env["GRIPSACK_BIN"] = str(self.grip)
        self.env["GRIPSACK_CONDA_HELPER_MIRROR"] = str(dist)
        self.env["GRIPSACK_BRIDGE_MIRROR"] = str(dist)
        self.sdk = prepare_sdk(dist / sdk, self.work, sdk_version)
        try:
            prepare_sdk(dist / sdk, self.work / "sdk-negative", "invalid-candidate-version")
        except CheckFailure:
            self.receipt["sdk_wrong_version_refused"] = True
        else:
            raise RuntimeError("SDK version mismatch unexpectedly admitted")
        npm_project = self.work / "npm-consumer"
        npm_project.mkdir()
        (npm_project / "package.json").write_text('{"name":"private-candidate-consumer","private":true,"type":"module"}\n')
        self.run(["npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund",
                  "--package-lock=false", "--save-exact", dist / sdk], cwd=npm_project)
        installed = npm_project / "node_modules/@gripsack/core"
        require(not installed.is_symlink() and installed.is_dir(), "npm installed SDK is not an independent package")
        require({name: value["sha256"] for name, value in snapshot(installed).items()}
                == {name: value["sha256"] for name, value in snapshot(self.sdk).items()},
                "npm installed SDK differs from packaged bytes")
        self.run(["node", "--input-type=module", "-e",
                  "import {defineWorkspace,workspace,pkg,pixi} from '@gripsack/core';"
                  "if (![defineWorkspace,workspace,pkg,pixi.fromLock].every(x=>typeof x==='function')) process.exit(1);"],
                 cwd=npm_project)
        self.sdk = installed
        self.receipt["components"] = {"core": version, "helper": helper_version, "sdk": sdk_version,
                                      "bridge": bridge_version, "installed_core_sha256": sha(self.grip)}
        self.sdk_tar = dist / sdk

    def journeys(self):
        self.run([sys.executable, ROOT / "scripts/check_examples.py", "--site", self.args.site,
                  "--core-repo", ROOT, "--sdk", self.sdk_tar, "--grip", self.grip,
                  "--deno", self.env["GRIPSACK_DENO"], "--selfcheck", "--keep"])
        junit = self.work / "installed-flows.xml"
        cases = ["test_journey_contracts.py", "test_generation_history.py", "test_workspace_packages.py",
                 "test_source_approval.py", "test_capture_policy.py",
                 "test_workspace_captured_files.py", "test_workspace_archive_roots.py",
                 "test_workspace_managed_launchers.py", "test_workspace_file_checks.py",
                 "test_workspace_migration.py", "test_workspace_directory_evolution.py",
                 "test_task_graphs.py", "test_workspace_gnu_loader.py",
                 "test_store_hygiene.py", "test_plan_operations.py"]
        # Fault-control tests remain mandatory in the debug flow gate; the
        # installed release intentionally does not contain those controls.
        self.run([sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider",
                  "-m", "not requires_fault_controls",
                  f"--basetemp={self.work / 'flows'}", f"--junitxml={junit}",
                  *(ROOT / "e2e" / name for name in cases)])
        suites = ET.parse(junit).getroot()
        tests = list(suites.iter("testcase"))
        require(tests and not any(child.tag in {"skipped", "error", "failure"} for test in tests for child in test),
                "installed flow suite has skipped or failed cases")
        self.receipt["installed_flows"] = {
            "cases": len(tests), "junit": str(junit), "sha256": sha(junit),
            "requires_fault_controls": "required in the separate debug flow gate; not part of this release run",
        }

    def external_consumer(self):
        repo = self.work / "example"
        shutil.copytree(self.args.example, repo, ignore=shutil.ignore_patterns(".git"))
        before = snapshot(repo)
        install_sdk(repo, self.sdk)
        # Frozen root caller: no automatic lock migration or mutable GitHub release install.
        self.approve(repo)
        self.run([self.grip, "check"], cwd=repo)
        # This published lock was captured against the embedded frontend.
        # Installing a repository SDK would deliberately change its import pins.
        conda = self.work / "example-conda"
        shutil.copytree(self.args.example / "workspaces/conda", conda)
        conda_before = snapshot(conda)
        lock = conda / "gripsack.lock"
        frozen = lock.read_bytes()
        self.approve(conda)
        self.run([self.grip, "check"], cwd=conda)
        built = json.loads(self.run([self.grip, "build", "ripgrep", "--json"], cwd=conda))
        require([(output["name"], output["kind"]) for output in built["outputs"]] == [("ripgrep", "package")],
                "external Conda build did not return the requested package")
        roots = [Path(output["path"]) for output in built["outputs"]]
        retained = {str(root): snapshot(root) for root in roots}
        result = self.run([self.grip, "run", "--env", "search", "--", "rg", "^needle$"],
                          cwd=conda, input="other\nneedle\n")
        require(result.strip() == "needle", "real packaged ripgrep execution differs")
        self.run([self.grip, "task", "run", "version"], cwd=conda)
        self.run([self.grip, "apply", "personal"], cwd=conda)
        self.run([self.grip, "apply", "personal"], cwd=conda)
        unavailable = dict(self.env, GRIPSACK_CONDA_HELPER=str(self.work / "unavailable-helper"))
        self.run([self.grip, "build", "ripgrep", "--json"], cwd=conda, env=unavailable)
        self.run([self.grip, "gc"], cwd=conda, env=unavailable)
        self.run([self.grip, "shell", "search"], cwd=conda, env=unavailable, input="rg --version\nexit\n")
        require(lock.read_bytes() == frozen, "frozen external lock was changed")
        require(all(snapshot(Path(path)) == content for path, content in retained.items()), "retained external prefix changed")
        conda_after = snapshot(conda)
        require(all(conda_after.get(path) == content for path, content in conda_before.items()),
                "frozen external Conda source/lock changed")
        after = snapshot(repo)
        require(all(after.get(path) == content for path, content in before.items()), "original external source/locks changed")
        self.receipt["external_consumer"] = {"root": "root workspace check with candidate installed SDK",
            "conda": "embedded-frontend frozen build/run/task/apply/apply; unavailable-helper retained build/gc/shell",
            "frozen_lock_sha256": sha(lock), "retained_prefixes": retained,
            "network": "initial exact lock-pinned public archives fetched by core; retained reuse is not a network-namespace isolation claim"}

    def execute(self):
        self.source_identity()
        self.artifacts()
        self.journeys()
        self.external_consumer()
        from local_ci_conda_consumer import consume_archives
        consume_archives(self)
        self.receipt["status"] = "passed"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("dist", "site", "example", "report"):
        parser.add_argument(f"--{option}", required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    consumer = Consumer(args)
    try:
        consumer.execute()
    except Exception as error:
        consumer.receipt.update(status="failed", error=f"{type(error).__name__}: {error}")
        print(consumer.receipt["error"], file=sys.stderr)
        return 1
    finally:
        consumer.report.write_text(json.dumps(consumer.receipt, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
