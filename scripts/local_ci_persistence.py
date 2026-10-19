#!/usr/bin/env python3
"""Run/resume the protected Linux persistence matrix and its existing admission.

Completed partitions survive failed/interrupted gates in a separate CI-owned
checkpoint root. A checkpoint is not a controller success receipt: the final
full 96-partition union must still pass before outputs become gate artifacts.
"""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor, as_completed
import json
import os
from pathlib import Path
import re
import shutil
import tarfile

from check_ci_gate import (PERSISTENCE_SCENARIOS, PERSISTENCE_SHARDS,
                           admit_persistence, decode)
from local_ci_checkpoints import Checkpoints, LOG_BYTES, METADATA_BYTES, regular_bytes
from local_ci_partition import execute, inspect_attempt, validate_attempt
from local_ci_runtime import ROOT, STATE, capture, digest, load_image, record, source

PARTITION_SECONDS = 120 * 60
REQUIRED_PARTITIONS = 96
MAX_SLOTS = 8
SLOT_MEMORY_BYTES = 2 * 1024 ** 3


def input_binding(revision, image):
    candidate = os.environ.get("CI_CANDIDATE_ID", "")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,199}", candidate):
        raise ValueError("CI_CANDIDATE_ID must identify the sanitized frozen candidate")
    return {
        "schema": 1, "candidate": candidate, "source": revision, "image": image,
        "image_receipt_sha256": digest(STATE / "images/e2e.json"),
        "dispatcher_inputs": {name: digest(ROOT / name) for name in (
            ".ci/pilot.json", "scripts/local_ci_persistence.py", "scripts/local_ci_checkpoints.py",
            "scripts/local_ci_partition.py", "scripts/local_ci_runtime.py", "scripts/check_ci_gate.py",
        )},
        "shards": PERSISTENCE_SHARDS, "partition_timeout_seconds": PARTITION_SECONDS,
        "partition_memory_bytes": SLOT_MEMORY_BYTES, "partition_cpus": 1,
        "retained_log_policy": "Docker local ring: 16 MiB x 1; checkpoint <=16 MiB",
    }


def resume_order(checkpoints, slots):
    live = []
    other = []
    for name in checkpoints.partitions:
        checkpoint = checkpoints.partition(name)
        completed = checkpoint.completed()
        attempt = checkpoint.attempt()
        metadata = None
        if attempt is not None:
            validate_attempt(attempt, checkpoint)
            metadata = inspect_attempt(attempt)
        if metadata is not None and metadata["State"]["Running"]:
            if completed is not None:
                raise ValueError("completed partition unexpectedly has a live container")
            live.append(name)
        else:
            other.append(name)
    if len(live) > slots:
        raise ValueError(f"{len(live)} journalled partitions are still live, exceeding {slots} slots; resume with sufficient measured capacity")
    # All existing live work occupies a slot before any fresh partition starts.
    return live + other


def dispatch(checkpoints, slots, reports, logs):
    completed = []
    failures = []
    origins = {}

    def partition(name):
        checkpoint = checkpoints.partition(name)
        report, origin = execute(checkpoint)
        destination = reports / name
        destination.mkdir()
        # Revalidate regular-file bounds on materialization; controller outputs
        # are copies, never hardlinks or checkpoint-directory aliases.
        regular_bytes(checkpoint.report, METADATA_BYTES)
        regular_bytes(checkpoint.log, LOG_BYTES)
        shutil.copyfile(checkpoint.report, destination / checkpoint.report.name)
        shutil.copyfile(checkpoint.log, logs / f"{name}.log")
        return report, origin

    with ThreadPoolExecutor(max_workers=slots) as workers:
        pending = {workers.submit(partition, name): name for name in resume_order(checkpoints, slots)}
        for future in as_completed(pending):
            name = pending[future]
            try:
                report, origin = future.result()
                completed.append(report)
                origins[name] = origin
                print(f"completed persistence partition {name} ({origin})", flush=True)
            except Exception as error:
                failures.append({"partition": name, "error": str(error)})
                print(f"FAILED persistence partition {name}: {error}", flush=True)
    return completed, failures, origins


def main():
    revision = source()
    image = load_image("e2e")
    resources = json.loads(capture(["docker", "info", "--format", "{{json .}}"]))
    if resources["OSType"] != "linux" or resources["Architecture"] not in ("x86_64", "amd64"):
        raise ValueError("persistence requires a native Linux x86_64 Docker daemon")
    capacity = min(MAX_SLOTS, resources["NCPU"], resources["MemTotal"] // SLOT_MEMORY_BYTES)
    if capacity < 1:
        raise ValueError("persistence needs at least one CPU and 2 GiB Docker memory")
    slots = int(os.environ.get("GRIPSACK_PERSISTENCE_SLOTS", str(capacity)))
    if not 1 <= slots <= capacity:
        raise ValueError(f"persistence slots must be 1..{capacity} for measured Docker resources")
    partitions = {f"Linux-{scenario}-{fault}-{shard}": ("Linux", scenario, fault, shard)
                  for scenario in PERSISTENCE_SCENARIOS for fault in ("error", "kill")
                  for shard in range(PERSISTENCE_SHARDS)}
    if len(partitions) != REQUIRED_PARTITIONS:
        raise ValueError("protected persistence inventory changed; update the complete profile deliberately")
    checkpoint_root = os.environ.get("CI_CHECKPOINT_ROOT")
    if not checkpoint_root:
        raise ValueError("CI_CHECKPOINT_ROOT is required for durable per-partition resumption")
    reports = STATE / "persistence" / "reports"
    logs = STATE / "persistence" / "logs"
    reports.mkdir(parents=True, exist_ok=False)
    logs.mkdir(parents=True, exist_ok=False)
    binding = input_binding(revision, image)
    with Checkpoints(Path(checkpoint_root), binding, partitions) as checkpoints:
        completed, failures, origins = dispatch(checkpoints, slots, reports, logs)
    with tarfile.open(STATE / "persistence" / "logs.tar.gz", "w:gz") as archive:
        archive.add(logs, arcname="logs")
    record(STATE / "persistence" / "dispatch.json", {
        "source": revision, "candidate": binding["candidate"], "image": image, "slots": slots,
        "measured_docker_cpus": resources["NCPU"],
        "measured_docker_memory_bytes": resources["MemTotal"],
        "partition_memory_bytes": SLOT_MEMORY_BYTES,
        "partition_timeout_seconds": PARTITION_SECONDS,
        "expected_partitions": len(partitions), "completed_partitions": len(completed),
        "partition_origins": origins, "failures": failures,
    })
    if failures:
        raise RuntimeError(f"{len(failures)} persistence partitions failed; completed checkpoints remain resumable")
    # Full union agreement remains the original authority, including inventory
    # agreement across fault modes/shards and exact both-drift cut coverage.
    admit_persistence([decode(path.read_text()) for path in sorted(reports.rglob("*.json"))], revision)
    print("existing persistence admission accepted all 96 partitions and both drift states", flush=True)


if __name__ == "__main__":
    main()
