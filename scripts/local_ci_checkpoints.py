"""Bounded, durable partition checkpoints; partial files never grant success.

Only the existing persistence admission decides report semantics. This module
binds its result to the candidate, dispatcher inputs, actual container exit and
retained bytes. A whole-dispatcher flock excludes concurrent writers/resumers.
"""
from __future__ import annotations

import fcntl
import hashlib
import json
import os
from pathlib import Path
import stat

from check_ci_gate import admit_persistence_partition, decode

METADATA_BYTES = 64 * 1024
LOG_BYTES = 16 * 1024 * 1024


def regular_bytes(path, maximum):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size > maximum:
            raise ValueError(f"checkpoint is not a bounded private regular file: {path}")
        data = stream.read(maximum + 1)
        if len(data) > maximum:
            raise ValueError(f"checkpoint grew beyond its bound: {path}")
        return data


def read_json(path):
    return decode(regular_bytes(path, METADATA_BYTES).decode("utf-8"))


def checksum(path, maximum):
    return hashlib.sha256(regular_bytes(path, maximum)).hexdigest()


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def write_json(path, value):
    encoded = (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()
    if len(encoded) > METADATA_BYTES:
        raise ValueError("checkpoint metadata exceeds its bound")
    temporary = path.with_name(path.name + ".pending")
    if temporary.exists() or temporary.is_symlink():
        regular_bytes(temporary, METADATA_BYTES)
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(encoded)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    sync_directory(path.parent)


def private_directory(path):
    path.mkdir(mode=0o700, exist_ok=True)
    if not stat.S_ISDIR(path.lstat().st_mode) or path.resolve() != path.absolute():
        raise ValueError(f"checkpoint directory aliases another location: {path}")
    sync_directory(path.parent)


class Checkpoints:
    def __init__(self, root, binding, partitions):
        self.root = Path(root)
        if not self.root.is_absolute():
            raise ValueError("CI_CHECKPOINT_ROOT must be absolute")
        private_directory(self.root)
        self.binding = binding
        self.partitions = partitions
        self.lock = None

    def __enter__(self):
        descriptor = os.open(self.root / "lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        self.lock = os.fdopen(descriptor, "r+")
        try:
            info = os.fstat(descriptor)
            if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size != 0:
                raise ValueError("invalid checkpoint lock")
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            allowed = {"lock", "binding.json", "binding.json.pending", *self.partitions}
            if {path.name for path in self.root.iterdir()} - allowed:
                raise ValueError("unknown entries in checkpoint root")
            pending = self.root / "binding.json.pending"
            if pending.exists() or pending.is_symlink():
                regular_bytes(pending, METADATA_BYTES)
            path = self.root / "binding.json"
            if path.exists() or path.is_symlink():
                if read_json(path) != self.binding:
                    raise ValueError("checkpoint belongs to foreign candidate/source/dispatcher inputs")
            else:
                if any((self.root / name).exists() for name in self.partitions):
                    raise ValueError("partition state exists without a committed root binding")
                write_json(path, self.binding)
            return self
        except BaseException:
            self.lock.close()
            self.lock = None
            raise

    def __exit__(self, *_):
        self.lock.close()

    def partition(self, name):
        return PartitionCheckpoint(self.root / name, self.binding, self.partitions[name])


class PartitionCheckpoint:
    def __init__(self, directory, binding, key):
        self.directory = directory
        private_directory(directory)
        self.key = tuple(key)
        self.binding = {**binding, "partition": list(key)}
        self.reports = directory / "reports"
        private_directory(self.reports)
        self.report = self.reports / (directory.name + ".json")
        self.log = directory / "partition.log"
        allowed = {"reports", "partition.log", "attempt.json", "attempt.json.pending",
                   "complete.json", "complete.json.pending"}
        if {path.name for path in directory.iterdir()} - allowed:
            raise ValueError("unknown partition checkpoint entries")
        if {path.name for path in self.reports.iterdir()} - {self.report.name}:
            raise ValueError("unknown partition report entries")
        for path in directory.iterdir():
            if path.name != "reports":
                regular_bytes(path, LOG_BYTES if path.name == "partition.log" else METADATA_BYTES)
        if self.report.exists() or self.report.is_symlink():
            regular_bytes(self.report, METADATA_BYTES)

    def attempt(self):
        path = self.directory / "attempt.json"
        if not path.exists():
            return None
        value = read_json(path)
        if value.get("binding") != self.binding:
            raise ValueError("partition attempt has foreign inputs")
        return value

    def save_attempt(self, attempt):
        if attempt["binding"] != self.binding:
            raise ValueError("cannot journal foreign partition inputs")
        write_json(self.directory / "attempt.json", attempt)

    def report_value(self):
        if {path.name for path in self.reports.iterdir()} != {self.report.name}:
            raise ValueError("partition emitted an unexpected checkpoint report inventory")
        value = read_json(self.report)
        if admit_persistence_partition(value, self.binding["source"]) != self.key:
            raise ValueError("report substituted another logical partition")
        return value

    def validate_result(self, result, attempt):
        if (set(result) != {"exit_code", "oom_killed", "error", "docker_started_at",
                           "docker_finished_at", "observed_boot_id", "observed_monotonic_ns"}
                or type(result["exit_code"]) is not int or result["exit_code"] != 0
                or result["oom_killed"] is not False or result["error"] != ""):
            raise ValueError("partition did not record an actual clean exit zero")
        started = attempt.get("started_monotonic_ns")
        finished = result["observed_monotonic_ns"]
        deadline = attempt.get("deadline_monotonic_ns")
        if (any(type(value) is not int for value in (started, finished, deadline))
                or not 0 < started <= finished <= deadline
                or deadline - started != self.binding["partition_timeout_seconds"] * 1_000_000_000
                or not attempt.get("boot_id") or result["observed_boot_id"] != attempt["boot_id"]
                or any(not isinstance(result[field], str) or not result[field]
                       for field in ("docker_started_at", "docker_finished_at"))):
            raise ValueError("partition completion lacks a valid original boot-bound monotonic budget")

    def complete(self, result):
        attempt = self.attempt()
        if attempt is None or not attempt.get("container_id"):
            raise ValueError("cannot complete without a journalled container identity")
        self.validate_result(result, attempt)
        self.report_value()
        # The producer has exited; sync its regular report and our retained log
        # before atomically publishing the only success marker.
        for path in (self.report, self.log):
            with path.open("rb") as stream:
                os.fsync(stream.fileno())
        sync_directory(self.reports)
        write_json(self.directory / "complete.json", {
            "schema": 1, "binding": self.binding, "result": result,
            "attempt_sha256": checksum(self.directory / "attempt.json", METADATA_BYTES),
            "report_sha256": checksum(self.report, METADATA_BYTES),
            "log_sha256": checksum(self.log, LOG_BYTES),
        })

    def completed(self):
        path = self.directory / "complete.json"
        if not path.exists():
            return None
        receipt = read_json(path)
        if (set(receipt) != {"schema", "binding", "result", "attempt_sha256", "report_sha256", "log_sha256"}
                or type(receipt["schema"]) is not int or receipt["schema"] != 1
                or receipt["binding"] != self.binding
                or receipt["attempt_sha256"] != checksum(self.directory / "attempt.json", METADATA_BYTES)
                or receipt["report_sha256"] != checksum(self.report, METADATA_BYTES)
                or receipt["log_sha256"] != checksum(self.log, LOG_BYTES)):
            raise ValueError("corrupt or foreign completed partition checkpoint")
        attempt = self.attempt()
        if not attempt.get("container_id"):
            raise ValueError("completed partition lost its actual container identity")
        self.validate_result(receipt["result"], attempt)
        return self.report_value()

    def reset_unfinished(self):
        if (self.directory / "complete.json").exists():
            raise ValueError("completed checkpoints are immutable")
        # Only exact known bounded regular files may be replaced at a boundary.
        for path, maximum in ((self.report, METADATA_BYTES), (self.log, LOG_BYTES)):
            if path.exists() or path.is_symlink():
                regular_bytes(path, maximum)
                path.unlink()
        sync_directory(self.directory)
