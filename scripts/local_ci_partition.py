"""Owned Docker lifecycle for one persistence partition, including reattachment.

Create -> durable exact-ID journal -> start. The container is deliberately not
--rm: an interrupted controller must observe its real exit status before a
checkpoint can qualify. Only a matching recorded identity may be reconciled.
"""
from __future__ import annotations

from pathlib import Path
import hashlib
import json
import os
import re
import subprocess
import time
import uuid

from local_ci_checkpoints import LOG_BYTES, sync_directory
from local_ci_runtime import OWNER_LABEL, SOURCE_LABEL, capture, run

CANDIDATE_LABEL = "dev.gripsack.local-ci.candidate"
INPUTS_LABEL = "dev.gripsack.local-ci.partition-inputs"


def command_for(checkpoint):
    _, scenario, fault, _ = checkpoint.key
    return ["timeout", "--signal=KILL", str(checkpoint.binding["partition_timeout_seconds"]),
            "uv", "run", "--locked", "--no-sync", "pytest", "-s",
            f"test_persistence_matrix.py::test_every_reachable_persistence_boundary[{scenario}-{fault}]"]


def labels_for(attempt):
    binding = attempt["binding"]
    return {OWNER_LABEL: attempt["owner"], SOURCE_LABEL: binding["source"],
            CANDIDATE_LABEL: binding["candidate"],
            INPUTS_LABEL: hashlib.sha256(json.dumps(binding, sort_keys=True).encode()).hexdigest()}


def validate_attempt(attempt, checkpoint):
    if (set(attempt) != {"binding", "owner", "name", "mount", "command", "container_id",
                        "boot_id", "started_monotonic_ns", "deadline_monotonic_ns", "phase"}
            or attempt["binding"] != checkpoint.binding
            or not isinstance(attempt["owner"], str)
            or not re.fullmatch(r"[0-9a-f]{32}", attempt["owner"])
            or attempt["name"] != "gripsack-persist-" + attempt["owner"]
            or not isinstance(attempt["mount"], str) or not attempt["mount"].startswith("/")
            or attempt["command"] != command_for(checkpoint)
            or attempt["phase"] not in ("intent", "created", "started", "failed")
            or (attempt["container_id"] is not None
                and (not isinstance(attempt["container_id"], str)
                     or not re.fullmatch(r"[0-9a-f]{64}", attempt["container_id"])))):
        raise ValueError("invalid or foreign partition lifecycle journal")
    if attempt["phase"] in ("intent", "created"):
        if any(attempt[key] is not None for key in ("boot_id", "started_monotonic_ns", "deadline_monotonic_ns")):
            raise ValueError("unstarted partition has unexpected timing authority")
    elif (not isinstance(attempt["boot_id"], str)
          or not re.fullmatch(r"[0-9a-f-]{36}", attempt["boot_id"])
          or type(attempt["started_monotonic_ns"]) is not int
          or type(attempt["deadline_monotonic_ns"]) is not int
          or attempt["started_monotonic_ns"] <= 0
          or attempt["deadline_monotonic_ns"] - attempt["started_monotonic_ns"]
          != checkpoint.binding["partition_timeout_seconds"] * 1_000_000_000):
        raise ValueError("invalid original monotonic execution budget")


def validate_container(metadata, attempt):
    """Fail before cleanup/attachment if any actual ownership input changed."""
    expected_labels = labels_for(attempt)
    labels = metadata["Config"].get("Labels") or {}
    mounts = metadata["Mounts"]
    binding = attempt["binding"]
    if (not re.fullmatch(r"[0-9a-f]{64}", metadata["Id"])
            or (attempt["container_id"] is not None and metadata["Id"] != attempt["container_id"])
            or metadata["Name"] != "/" + attempt["name"]
            or metadata["Image"] != binding["image"]
            or any(labels.get(key) != value for key, value in expected_labels.items())
            or metadata["Config"]["Cmd"] != attempt["command"]
            or metadata["Config"].get("Entrypoint") not in (None, [])
            or metadata["Config"]["WorkingDir"] != "/app/e2e"
            or len(mounts) != 1 or mounts[0]["Type"] != "bind"
            or mounts[0]["Source"] != attempt["mount"]
            or mounts[0]["Destination"] != "/persistence-reports" or mounts[0]["RW"] is not True
            or metadata["HostConfig"]["Memory"] != binding["partition_memory_bytes"]
            or metadata["HostConfig"]["MemorySwap"] != binding["partition_memory_bytes"]
            or metadata["HostConfig"]["NanoCpus"] != 1_000_000_000):
        raise ValueError("partition container ownership/image/command/mount/resource identity changed")
    environment = dict(value.split("=", 1) for value in metadata["Config"]["Env"])
    expected_environment = environment_for(binding)
    if any(environment.get(key) != value for key, value in expected_environment.items()):
        raise ValueError("partition container environment changed")


def environment_for(binding):
    return {"GITHUB_SHA": binding["source"],
            "GRIPSACK_PERSISTENCE_SHARD": str(binding["partition"][3]),
            "GRIPSACK_PERSISTENCE_SHARDS": str(binding["shards"]),
            "GRIPSACK_PERSISTENCE_REPORTS": "/persistence-reports", "CARGO_BUILD_JOBS": "4"}


def inspect_attempt(attempt):
    reference = attempt["container_id"] or attempt["name"]
    result = subprocess.run(["docker", "container", "inspect", reference], capture_output=True, text=True)
    if result.returncode:
        # Missing is different from daemon failure or unreadable existing state.
        run(["docker", "info", "--format", "{{.ServerVersion}}"])
        selector = ("id=" + reference if attempt["container_id"]
                    else "name=^/" + reference + "$")
        if capture(["docker", "container", "ls", "--all", "--no-trunc", "--filter", selector,
                    "--format", "{{.ID}}"]):
            raise RuntimeError("journalled container exists but could not be inspected")
        return None
    rows = json.loads(result.stdout)
    if len(rows) != 1:
        raise ValueError("ambiguous partition container identity")
    validate_container(rows[0], attempt)
    return rows[0]


def remove_owned(attempt):
    metadata = inspect_attempt(attempt)
    if metadata is not None:
        run(["docker", "rm", "--force", metadata["Id"]])


def create(checkpoint):
    owner = uuid.uuid4().hex
    attempt = {"binding": checkpoint.binding, "owner": owner,
               "name": "gripsack-persist-" + owner, "mount": str(checkpoint.reports),
               "command": command_for(checkpoint), "container_id": None,
               "boot_id": None, "started_monotonic_ns": None, "deadline_monotonic_ns": None, "phase": "intent"}
    checkpoint.save_attempt(attempt)
    binding = checkpoint.binding
    argv = ["docker", "create", "--pull=never", "--platform=linux/amd64", "--name", attempt["name"],
            "--cpus=1", f"--memory={binding['partition_memory_bytes']}",
            f"--memory-swap={binding['partition_memory_bytes']}",
            "--log-driver=local", "--log-opt=max-size=16m", "--log-opt=max-file=1",
            "--mount", f"type=bind,src={attempt['mount']},dst=/persistence-reports"]
    for key, value in labels_for(attempt).items():
        argv += ["--label", f"{key}={value}"]
    for key, value in environment_for(binding).items():
        argv += ["--env", f"{key}={value}"]
    identifier = capture([*argv, binding["image"], *attempt["command"]])
    if not re.fullmatch(r"[0-9a-f]{64}", identifier):
        raise ValueError("Docker create did not return an exact container ID")
    attempt.update(container_id=identifier, phase="created")
    checkpoint.save_attempt(attempt)  # fsync before any execution
    return attempt


def retain_logs(checkpoint, identifier):
    descriptor = os.open(checkpoint.log, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        process = subprocess.Popen(["docker", "logs", identifier], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        retained = 0
        try:
            while block := process.stdout.read(65536):
                if retained + len(block) > LOG_BYTES:
                    raise ValueError("partition retained log exceeds the checkpoint bound")
                output.write(block)
                retained += len(block)
            if process.wait() != 0:
                raise RuntimeError("could not retain actual partition container logs")
            output.flush()
            os.fsync(output.fileno())
        finally:
            process.stdout.close()
            if process.poll() is None:
                process.kill()
                process.wait()
    sync_directory(checkpoint.directory)


def boot_identity():
    value = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    if not re.fullmatch(r"[0-9a-f-]{36}", value):
        raise ValueError("cannot establish monotonic clock boot identity")
    return value


def execute(checkpoint):
    completed = checkpoint.completed()
    attempt = checkpoint.attempt()
    if attempt is not None:
        validate_attempt(attempt, checkpoint)
        metadata = inspect_attempt(attempt)
    else:
        metadata = None
    if completed is not None:
        if metadata is not None:
            if metadata["State"]["Status"] != "exited":
                raise ValueError("completed checkpoint container is unexpectedly live")
            remove_owned(attempt)
        return completed, "checkpoint"

    if (metadata is not None and attempt["started_monotonic_ns"] is not None
            and attempt["boot_id"] != boot_identity()):
        # A changed boot has no comparable monotonic deadline. Do not grant
        # already-started work a new budget, even if Docker claims it exited.
        attempt["phase"] = "failed"
        checkpoint.save_attempt(attempt)
        remove_owned(attempt)
        raise TimeoutError("partition timing is ambiguous after a boot change; unfinished boundary expired")

    if attempt is not None and (attempt["phase"] == "failed"
                                or attempt["mount"] != str(checkpoint.reports)):
        # A new operator invocation may restart a failed/interrupted boundary.
        # Relocated backups cannot attach to an old report mount. First stop only
        # the exact previously journalled and fully matched owned container.
        remove_owned(attempt)
        metadata = None
    if metadata is None:
        checkpoint.reset_unfinished()
        attempt = create(checkpoint)
        metadata = inspect_attempt(attempt)
        if metadata is None:
            raise RuntimeError("newly created partition container disappeared")
        origin = "fresh"
    else:
        origin = "reattached"
        if attempt["container_id"] is None:
            if metadata["State"]["Status"] != "created":
                raise ValueError("unjournalled container unexpectedly executed")
            attempt.update(container_id=metadata["Id"], phase="created")
            checkpoint.save_attempt(attempt)
    identifier = attempt["container_id"]
    if metadata["State"]["Status"] == "created":
        if attempt["started_monotonic_ns"] is None:
            started = time.monotonic_ns()
            attempt.update(boot_id=boot_identity(), started_monotonic_ns=started,
                           deadline_monotonic_ns=started + checkpoint.binding["partition_timeout_seconds"] * 1_000_000_000,
                           phase="started")
            checkpoint.save_attempt(attempt)
        if time.monotonic_ns() >= attempt["deadline_monotonic_ns"]:
            attempt["phase"] = "failed"
            checkpoint.save_attempt(attempt)
            remove_owned(attempt)
            raise TimeoutError("partition's original start deadline expired")
        run(["docker", "start", identifier])
    remaining = (attempt["deadline_monotonic_ns"] - time.monotonic_ns()) / 1_000_000_000
    metadata = inspect_attempt(attempt)
    if metadata is None:
        raise RuntimeError("partition disappeared before observing its real exit status")
    if metadata["State"]["Running"]:
        if remaining <= 0:
            attempt["phase"] = "failed"
            checkpoint.save_attempt(attempt)
            remove_owned(attempt)
            raise TimeoutError("partition exceeded its original deadline")
        try:
            run(["docker", "wait", identifier], timeout=remaining, stdout=subprocess.PIPE, text=True)
        except subprocess.TimeoutExpired:
            attempt["phase"] = "failed"
            checkpoint.save_attempt(attempt)
            remove_owned(attempt)
            raise
        metadata = inspect_attempt(attempt)
    if metadata is None or metadata["State"]["Status"] != "exited":
        raise RuntimeError("partition has no terminal container execution result")
    state = metadata["State"]
    result = {"exit_code": state["ExitCode"], "oom_killed": state["OOMKilled"], "error": state["Error"],
              "docker_started_at": state["StartedAt"], "docker_finished_at": state["FinishedAt"],
              "observed_boot_id": boot_identity(), "observed_monotonic_ns": time.monotonic_ns()}
    try:
        retain_logs(checkpoint, identifier)
        checkpoint.complete(result)
    except (ValueError, FileNotFoundError):
        attempt["phase"] = "failed"
        checkpoint.save_attempt(attempt)
        remove_owned(attempt)
        raise
    remove_owned(attempt)
    return checkpoint.completed(), origin
