"""Behavioral checkpoint admission/ownership tests; no Docker daemon required."""
from copy import deepcopy
from pathlib import Path
import shutil
import tempfile
import time
import unittest
from unittest.mock import patch

from local_ci_checkpoints import (Checkpoints, LOG_BYTES, METADATA_BYTES,
                                  checksum, read_json, write_json)
from local_ci_partition import (boot_identity, command_for, create, environment_for,
                                execute, labels_for, validate_container)
from local_ci_persistence import resume_order


class CheckpointTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name) / "checkpoints"
        self.binding = {"candidate": "candidate-1", "source": "a" * 40,
                        "image": "sha256:" + "b" * 64,
                        "dispatcher_inputs": {"dispatcher": "c" * 64},
                        "partition_timeout_seconds": 7200,
                        "partition_memory_bytes": 2147483648, "shards": 8}
        self.name = "Linux-apply-deploy-error-0"
        self.keys = {self.name: ("Linux", "apply-deploy", "error", 0)}
        self.store = Checkpoints(self.root, self.binding, self.keys).__enter__()
        self.checkpoint = self.store.partition(self.name)

    def tearDown(self):
        self.store.__exit__()
        self.temporary.cleanup()

    def prepare(self):
        checkpoint = self.checkpoint
        started = time.monotonic_ns() - 2_000_000_000
        attempt = {"binding": checkpoint.binding, "owner": "d" * 32,
                   "name": "gripsack-persist-" + "d" * 32,
                   "mount": str(checkpoint.reports), "command": command_for(checkpoint),
                   "container_id": "e" * 64, "boot_id": boot_identity(),
                   "started_monotonic_ns": started,
                   "deadline_monotonic_ns": started + 7_200_000_000_000, "phase": "started"}
        checkpoint.save_attempt(attempt)
        report = {"source": self.binding["source"], "system": "Linux", "machine": "x86_64",
                  "scenario": "apply-deploy", "fault": "error", "shard": 0, "shards": 8,
                  "inventory": [["write", str(index)] for index in range(16)],
                  "completed_cuts": [1, 9], "drift_states": [False, True], "seconds": 1.0}
        write_json(checkpoint.report, report)
        checkpoint.log.write_bytes(b"real-output-fixture\n")
        result = {"exit_code": 0, "oom_killed": False, "error": "",
                  "docker_started_at": "2026-10-19T00:00:15Z",
                  "docker_finished_at": "2026-10-19T00:00:00Z",
                  "observed_boot_id": attempt["boot_id"],
                  "observed_monotonic_ns": started + 1_000_000_000}
        return attempt, report, result

    def test_completed_partition_reuses_without_execution(self):
        _, report, result = self.prepare()
        self.checkpoint.complete(result)
        with patch("local_ci_partition.inspect_attempt", return_value=None), \
                patch("local_ci_partition.run") as run, patch("local_ci_partition.capture") as capture:
            actual, origin = execute(self.checkpoint)
        self.assertEqual((actual, origin), (report, "checkpoint"))
        run.assert_not_called()
        capture.assert_not_called()

    def test_partial_report_and_log_never_grant_success(self):
        self.prepare()
        self.assertIsNone(self.checkpoint.completed())

    def test_checkpoint_rejects_changed_report_or_log(self):
        _, _, result = self.prepare()
        self.checkpoint.complete(result)
        for path in (self.checkpoint.report, self.checkpoint.log):
            before = path.read_bytes()
            path.write_bytes(before + b" ")
            with self.assertRaises(ValueError):
                self.checkpoint.completed()
            path.write_bytes(before)

    def test_rehashed_bad_drift_still_fails_original_semantic_admission(self):
        _, report, result = self.prepare()
        self.checkpoint.complete(result)
        report["drift_states"] = [False]
        write_json(self.checkpoint.report, report)
        marker = self.checkpoint.directory / "complete.json"
        receipt = read_json(marker)
        receipt["report_sha256"] = checksum(self.checkpoint.report, METADATA_BYTES)
        write_json(marker, receipt)
        with self.assertRaises(ValueError):
            self.checkpoint.completed()

    def test_no_success_for_missing_duplicate_or_foreign_cuts(self):
        _, report, result = self.prepare()
        for changes in ({"completed_cuts": [1]}, {"completed_cuts": [1, 1]},
                        {"source": "f" * 40}, {"shard": 1, "completed_cuts": [2, 10]}):
            with self.subTest(changes=changes):
                write_json(self.checkpoint.report, {**report, **changes})
                with self.assertRaises(ValueError):
                    self.checkpoint.complete(result)
                self.assertFalse((self.checkpoint.directory / "complete.json").exists())

    def test_only_real_clean_timely_zero_exit_can_complete(self):
        attempt, _, result = self.prepare()
        for changes in ({"exit_code": 1}, {"exit_code": False}, {"oom_killed": True},
                        {"error": "runtime error"},
                        {"observed_monotonic_ns": attempt["deadline_monotonic_ns"] + 1},
                        {"observed_monotonic_ns": float("inf")},
                        {"observed_boot_id": "a-different-boot"}):
            with self.subTest(changes=changes):
                with self.assertRaises(ValueError):
                    self.checkpoint.complete({**result, **changes})
                self.assertFalse((self.checkpoint.directory / "complete.json").exists())

    def test_root_binding_rejects_foreign_candidate_source_image_and_dispatcher(self):
        self.store.__exit__()
        try:
            for field in ("candidate", "source", "image", "dispatcher_inputs"):
                with self.subTest(field=field):
                    changed = {**self.binding, field: "foreign"}
                    with self.assertRaises(ValueError):
                        with Checkpoints(self.root, changed, self.keys):
                            self.fail("foreign checkpoint accepted")
        finally:
            self.store = Checkpoints(self.root, self.binding, self.keys).__enter__()

    def test_second_dispatcher_cannot_acquire_live_checkpoint_root(self):
        with self.assertRaises(BlockingIOError):
            with Checkpoints(self.root, self.binding, self.keys):
                self.fail("two dispatchers admitted")

    def test_symlink_and_oversized_log_are_rejected(self):
        self.prepare()
        self.checkpoint.log.unlink()
        self.checkpoint.log.symlink_to(self.checkpoint.report)
        with self.assertRaises((OSError, ValueError)):
            self.store.partition(self.name)
        self.checkpoint.log.unlink()
        with self.checkpoint.log.open("wb") as stream:
            stream.truncate(LOG_BYTES + 1)
        with self.assertRaises(ValueError):
            self.store.partition(self.name)

    def test_unknown_partition_files_are_not_adopted(self):
        (self.checkpoint.directory / "foreign.json").write_text("{}")
        with self.assertRaises(ValueError):
            self.store.partition(self.name)

    def test_completed_partition_survives_relocation_without_old_mount(self):
        _, report, result = self.prepare()
        self.checkpoint.complete(result)
        restored = Path(self.temporary.name) / "restored"
        shutil.copytree(self.root, restored)
        with Checkpoints(restored, self.binding, self.keys) as store:
            with patch("local_ci_partition.inspect_attempt", return_value=None), \
                    patch("local_ci_partition.capture") as capture:
                actual, origin = execute(store.partition(self.name))
        self.assertEqual((actual, origin), (report, "checkpoint"))
        capture.assert_not_called()

    def metadata(self, attempt):
        return {"Id": attempt["container_id"], "Name": "/" + attempt["name"],
                "Image": self.binding["image"],
                "Config": {"Labels": labels_for(attempt), "Cmd": attempt["command"],
                           "Entrypoint": None, "WorkingDir": "/app/e2e",
                           "Env": [f"{key}={value}" for key, value in environment_for(attempt["binding"]).items()]},
                "Mounts": [{"Type": "bind", "Source": attempt["mount"],
                            "Destination": "/persistence-reports", "RW": True}],
                "HostConfig": {"Memory": 2147483648, "MemorySwap": 2147483648,
                               "NanoCpus": 1_000_000_000,
                               "LogConfig": {"Type": "local", "Config": {
                                   "max-size": "16m", "max-file": "1", "compress": "false"}}}}

    def test_ownership_rejects_foreign_container_before_cleanup(self):
        attempt, _, _ = self.prepare()
        expected = self.metadata(attempt)
        validate_container(expected, attempt)
        mutations = [lambda value: value.update(Id="f" * 64),
                     lambda value: value.update(Image="sha256:" + "f" * 64),
                     lambda value: value["Mounts"][0].update(Source="/somebody-elses-state"),
                     lambda value: value["Config"].update(Labels={}),
                     lambda value: value["Config"].update(Cmd=["different-program"]),
                     lambda value: value["HostConfig"].update(Memory=0),
                     lambda value: value["HostConfig"]["LogConfig"].update(Type="none"),
                     lambda value: value["HostConfig"]["LogConfig"]["Config"].update({"max-size": "1g"}),
                     lambda value: value["HostConfig"]["LogConfig"]["Config"].update({"compress": "true"})]
        for mutate in mutations:
            actual = deepcopy(expected)
            mutate(actual)
            with self.assertRaises(ValueError):
                validate_container(actual, attempt)

    def test_running_partition_is_reattached_not_started_again(self):
        attempt, report, result = self.prepare()
        running = self.metadata(attempt)
        running["State"] = {"Status": "running", "Running": True}
        exited = deepcopy(running)
        exited["State"] = {"Status": "exited", "Running": False, "ExitCode": 0,
                           "OOMKilled": False, "Error": "",
                           "StartedAt": "2020-01-01T00:00:00Z", "FinishedAt": "2020-01-01T00:00:01Z"}
        with patch("local_ci_partition.inspect_attempt", side_effect=[running, running, exited, exited]), \
                patch("local_ci_partition.retain_logs"), patch("local_ci_partition.run") as run, \
                patch("local_ci_partition.capture") as capture:
            actual, origin = execute(self.checkpoint)
        self.assertEqual((actual, origin), (report, "reattached"))
        capture.assert_not_called()
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual(commands, [["docker", "wait", attempt["container_id"]],
                                    ["docker", "rm", "--force", attempt["container_id"]]])

    def test_backward_wall_clock_does_not_change_monotonic_admission(self):
        _, report, result = self.prepare()
        self.checkpoint.complete(result)
        self.assertEqual(self.checkpoint.completed(), report)

    def test_boot_change_expires_live_attempt_without_new_budget(self):
        attempt, _, _ = self.prepare()
        running = self.metadata(attempt)
        running["State"] = {"Status": "running", "Running": True}
        with patch("local_ci_partition.inspect_attempt", side_effect=[running, running]), \
                patch("local_ci_partition.boot_identity", return_value="0" * 36), \
                patch("local_ci_partition.run") as run, patch("local_ci_partition.capture") as capture:
            with self.assertRaises(TimeoutError):
                execute(self.checkpoint)
        capture.assert_not_called()
        self.assertEqual(run.call_args.args[0], ["docker", "rm", "--force", attempt["container_id"]])
        self.assertEqual(self.checkpoint.attempt()["deadline_monotonic_ns"], attempt["deadline_monotonic_ns"])
        self.assertEqual(self.checkpoint.attempt()["phase"], "failed")

    def test_completed_checkpoint_remains_reusable_after_boot_change(self):
        _, report, result = self.prepare()
        self.checkpoint.complete(result)
        with patch("local_ci_partition.inspect_attempt", return_value=None), \
                patch("local_ci_partition.boot_identity", return_value="0" * 36):
            self.assertEqual(execute(self.checkpoint), (report, "checkpoint"))

    def test_container_identity_is_durable_before_start_is_possible(self):
        def created(argv):
            self.assertEqual(argv[:2], ["docker", "create"])
            self.assertIsNone(self.checkpoint.attempt()["container_id"])
            return "f" * 64

        with patch("local_ci_partition.capture", side_effect=created), \
                patch("local_ci_partition.run") as run:
            attempt = create(self.checkpoint)
        self.assertEqual(self.checkpoint.attempt(), attempt)
        self.assertEqual(attempt["container_id"], "f" * 64)
        self.assertEqual(attempt["phase"], "created")
        run.assert_not_called()

    def test_live_work_occupies_slots_before_new_work(self):
        attempt, _, _ = self.prepare()
        second_name = "Linux-apply-deploy-kill-0"
        self.store.partitions = {second_name: ("Linux", "apply-deploy", "kill", 0), **self.keys}
        second = self.store.partition(second_name)
        second_attempt = {**attempt, "binding": second.binding, "mount": str(second.reports),
                          "command": command_for(second), "owner": "f" * 32,
                          "name": "gripsack-persist-" + "f" * 32, "container_id": "f" * 64}
        second.save_attempt(second_attempt)
        with patch("local_ci_persistence.inspect_attempt", return_value={"State": {"Running": True}}):
            with self.assertRaises(ValueError):
                resume_order(self.store, 1)
        with patch("local_ci_persistence.inspect_attempt",
                   side_effect=[{"State": {"Running": False}}, {"State": {"Running": True}}]):
            self.assertEqual(resume_order(self.store, 1), [self.name, second_name])


if __name__ == "__main__":
    unittest.main()
