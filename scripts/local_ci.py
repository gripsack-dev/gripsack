#!/usr/bin/env python3
"""Project-owned Linux qualification commands; never publishes or emulates Actions.

Each command runs in a fresh committed checkout. .ci/pilot.json declares the
precise predecessor files required; no daemon tag or previous checkout is an
implicit dependency. The controller admits gate exit codes and same-source
receipts, while existing project validators retain their authority.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import tarfile
import tomllib

from local_ci_runtime import (ROOT, STATE, build_image, capture, container,
                              digest, load_image, record, run, source)

TARGET = "x86_64-unknown-linux-musl"


def profile():
    value = json.loads((ROOT / ".ci/pilot.json").read_text())
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    sdk = json.loads((ROOT / "typescript/package.json").read_text())
    helper = tomllib.loads((ROOT / "tools/conda-helper/Cargo.toml").read_text())["package"]["version"]
    if (value["version"] != workspace or helper != workspace
            or value["external_inputs"]["sdk"] != {"name": sdk["name"], "version": sdk["version"]}):
        raise ValueError("profile component identities differ from committed package manifests")
    return value


def archive_tree(directory, destination):
    with tarfile.open(destination, "w:gz") as bundle:
        bundle.add(directory, arcname=directory.name)


def restore_tree(archive, directory):
    with tarfile.open(archive) as bundle:
        bundle.extractall(directory, filter="data")


def inventory():
    run(["python3", "scripts/check_delivery.py", "--validate"])
    run(["python3", "scripts/delivery_checker_calibration.py"])
    run(["python3", "scripts/check_architecture.py", "--self-check"])
    run(["python3", "scripts/check_ci_gate.py", "--self-check"])
    run(["python3", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_local_ci_checkpoints.py"])


def images():
    build_image("builder", export=True)
    e2e = build_image("e2e", export=True)
    build_image("local-tools", export=True)
    destination = STATE / "debug"
    destination.mkdir()
    container(e2e, ["install", "-m755", "/usr/local/bin/grip", "/export/grip"],
              mounts=[(destination, "/export", False)])


def audit():
    image = load_image("local-tools")
    directory = STATE / "audit"
    directory.mkdir()
    container(image, ["sh", "-ec", "\n".join([
        'mkdir -p "$CARGO_HOME"',
        "cargo audit --db /app/local-ci/audit/db",
        "git -C /app/local-ci/audit/db rev-parse HEAD > /app/local-ci/audit/revision.txt",
        # --no-fetch also prevents fetching missing registry entries. Populate
        # both graphs, then require one advisory revision before freezing both.
        "cargo audit --db /app/local-ci/audit/db --file tools/conda-helper/Cargo.lock",
        'test "$(git -C /app/local-ci/audit/db rev-parse HEAD)" = "$(cat /app/local-ci/audit/revision.txt)" || { echo "advisory revision changed during paired audits; repeat qualification" >&2; exit 1; }',
    ])], mounts=[(STATE, "/app/local-ci", False)],
              user=f"{os.getuid()}:{os.getgid()}",
              environment={"HOME": "/tmp/gripsack-audit",
                           "CARGO_HOME": "/app/local-ci/audit/cargo"})
    archive_tree(directory, STATE / "audit-snapshot.tar.gz")
    record(STATE / "audit.json", {
        "source": source(), "tool_image": image,
        "advisory_revision": (directory / "revision.txt").read_text().strip(),
        "snapshot_sha256": digest(STATE / "audit-snapshot.tar.gz"),
        "locks": {str(path): digest(ROOT / path) for path in (Path("Cargo.lock"), Path("tools/conda-helper/Cargo.lock"))},
    })


def bridge():
    run(["sh", "tools/buildkit-bridge/dist.sh", "--check", "--dist", "dist"])


def b0():
    target = ROOT / "target/debug"
    target.mkdir(parents=True)
    shutil.copy2(STATE / "debug/grip", target / "grip")
    run(["sh", "verification/buildkit-qualification/bridge/build.sh"])
    run(["sh", "verification/buildkit-qualification/probes.sh"])
    archive_tree(ROOT / "verification/buildkit-qualification/results", STATE / "b0-results.tar.gz")


def helper():
    image = build_image("local-helper")
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    container(image, ["python3", "tools/conda-helper/package.py", "--dist", "/out",
                      "--target", TARGET, "--check"], mounts=[(dist, "/out", False)],
              environment={"GRIPSACK_CONDA_PACKAGING_CONTAINER": "1"})


def core():
    builder = load_image("builder")
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    # The release service remains the one authoritative tarball producer.
    config = json.loads(capture(["docker", "compose", "config", "--format", "json"]))
    container(builder, config["services"]["release"]["command"],
              mounts=[(dist, "/dist", False)],
              environment={"VERSION": profile()["version"], "TARGET": TARGET})
    tools = load_image("local-tools")
    audit_receipt = json.loads((STATE / "audit.json").read_text())
    if (audit_receipt["source"] != source()
            or audit_receipt["snapshot_sha256"] != digest(STATE / "audit-snapshot.tar.gz")):
        raise ValueError("foreign or changed audit snapshot handoff")
    restore_tree(STATE / "audit-snapshot.tar.gz", STATE)
    name = f"gripsack-{profile()['version']}-{TARGET}"
    container(tools, ["sh", "-ec", "\n".join([
        'mkdir -p "$CARGO_HOME"',
        f"cd /app/dist && sha256sum -c {name}.tar.gz.sha256",
        f"mkdir /app/local-ci/core-check && tar -xzf {name}.tar.gz -C /app/local-ci/core-check",
        f"file /app/local-ci/core-check/{name}/grip",
        f"python3 /app/scripts/check_native_binary.py /app/local-ci/core-check/{name}/grip",
        f"test \"$(/app/local-ci/core-check/{name}/grip --version)\" = 'grip {profile()['version']}'",
        f"cd /app && cargo audit --db /app/local-ci/audit/db --no-fetch bin /app/local-ci/core-check/{name}/grip",
    ])], mounts=[(dist, "/app/dist", True), (STATE, "/app/local-ci", False)],
              user=f"{os.getuid()}:{os.getgid()}",
              environment={"HOME": "/tmp/gripsack-audit",
                           "CARGO_HOME": "/app/local-ci/audit/cargo"})
    record(STATE / "core-artifact.json", {"source": source(), "builder": builder,
           "validator_image": tools, "advisory_revision": audit_receipt["advisory_revision"],
           "asset": name + ".tar.gz", "sha256": digest(dist / (name + ".tar.gz"))})


def repro():
    builder = load_image("builder")
    output = STATE / "repro"
    run(["sh", "scripts/check_reproducible.sh", output],
        env={**os.environ, "REPRO_IMAGE_ID": builder})
    archive_tree(output, STATE / "core-repro.tar.gz")


def flows():
    container(load_image("e2e"), ["uv", "run", "--locked", "--no-sync", "pytest",
              "--ignore=test_persistence_matrix.py", "--ignore=test_golden.py"])


def sdk():
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    container(load_image("local-tools"), ["sh", "-ec",
              "npm ci --no-audit --no-fund && npm run build && npm pack --silent --pack-destination /app/dist"],
              workdir="/app/typescript", mounts=[(dist, "/app/dist", False)])


def external_sources():
    directory = STATE / "external"
    directory.mkdir()
    receipts = {}
    for name, pin in profile()["external_inputs"]["repositories"].items():
        checkout = directory / name
        run(["git", "init", checkout])
        run(["git", "-C", checkout, "remote", "add", "origin", pin["url"]])
        run(["git", "-C", checkout, "fetch", "--no-tags", "origin", pin["commit"]])
        actual = capture(["git", "-C", checkout, "rev-parse", "FETCH_HEAD^{commit}"])
        if actual != pin["commit"]:
            raise ValueError(f"external source {name} resolved to a different commit")
        run(["git", "-C", checkout, "update-ref", "refs/heads/candidate", actual])
        bundle = directory / f"{name}.bundle"
        run(["git", "-C", checkout, "bundle", "create", bundle, "refs/heads/candidate"])
        receipts[name] = {**pin, "bundle_sha256": digest(bundle)}
    record(directory / "sources.json", {"source": source(), "repositories": receipts})


def consumers():
    external = STATE / "external"
    receipts = json.loads((external / "sources.json").read_text())
    if receipts["source"] != source():
        raise ValueError("foreign external-source receipt")
    mounts = [(ROOT / "dist", "/app/dist", True), (STATE, "/app/local-ci", False)]
    for name, pin in profile()["external_inputs"]["repositories"].items():
        receipt = receipts["repositories"][name]
        bundle = external / f"{name}.bundle"
        if receipt != {**pin, "bundle_sha256": digest(bundle)}:
            raise ValueError(f"external source handoff changed: {name}")
        checkout = ROOT / name
        run(["git", "clone", "--no-checkout", bundle, checkout])
        run(["git", "-C", checkout, "checkout", "--detach", pin["commit"]])
        if capture(["git", "-C", checkout, "rev-parse", "HEAD"]) != pin["commit"]:
            raise ValueError("external checkout did not match pinned commit")
        mounts.append((checkout, f"/app/{name}", True))
    image = load_image("local-tools")
    container(image, ["python3", "scripts/local_artifact_consumers.py", "--dist", "/app/dist",
                      "--site", "/app/site", "--example", "/app/example",
                      "--report", "/app/local-ci/consumers.json"], mounts=mounts,
              environment={"GRIPSACK_CANDIDATE_COMMIT": source(), "GRIPSACK_TOOL_IMAGE": image},
              user=f"{os.getuid()}:{os.getgid()}")
    evidence = sorted(STATE.glob("artifact-consumers-*"))
    if len(evidence) != 1 or not evidence[0].is_dir():
        raise ValueError("consumer did not retain exactly one fresh evidence directory")
    archive_tree(evidence[0], STATE / "consumer-evidence.tar.gz")


def main():
    commands = {function.__name__.replace("_", "-"): function for function in
                (inventory, images, audit, bridge, b0, helper, core, repro, flows, sdk,
                 external_sources, consumers)}
    stages = ("test", "gnu-test", "ts-test", "model", "tlaps", "verify")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gate", choices=(*commands, *stages))
    args = parser.parse_args()
    source()
    profile()
    STATE.mkdir(exist_ok=True)
    if args.gate in stages:
        build_image(args.gate)
    else:
        commands[args.gate]()
    source()  # A producer may not rewrite the frozen tracked candidate.


if __name__ == "__main__":
    main()
