#!/usr/bin/env python3
"""Offline transport/identity regressions; no credentials or live GitHub calls."""
from copy import deepcopy
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import release_operator as operator


class FakeGitHub:
    def __init__(self, responses, before_call=None):
        self.responses = list(responses)
        self.calls = []
        self.before_call = before_call
        self.witnesses = {}
        self.actor_id = 19335185
        self.completion_digest = None

    def api(self, endpoint, payload=None):
        if self.before_call:
            self.before_call(endpoint, payload)
        self.calls.append((endpoint, payload))
        if endpoint == "user":
            return {"id": self.actor_id, "login": "test-admission-publisher"}
        if endpoint == f"repos/{operator.REPOSITORY}":
            return {"id": 1342976637, "full_name": operator.REPOSITORY}
        if payload is not None and "/statuses/" in endpoint:
            item = {"id": 1, "creator": {"id": self.actor_id},
                    "url": "https://api.github.com/" + endpoint, **payload}
            self.witnesses[payload["context"]] = item
            return deepcopy(item)
        if "/commits/" in endpoint and "/statuses?" in endpoint:
            source = endpoint.split("/commits/", 1)[1].split("/", 1)[0]
            rows = list(self.witnesses.values())
            if self.completion_digest is not None:
                rows.insert(0, {"id": 2, "creator": {"id": 19335185},
                    "url": f"https://api.github.com/repos/{operator.REPOSITORY}/statuses/{source}",
                    "context": "release-consumer-verification", "state": "success",
                    "description": "completion-sha256:" + self.completion_digest,
                    "target_url": f"https://github.com/{operator.REPOSITORY}/actions/runs/101"})
            return deepcopy(rows)
        if not self.responses:
            raise AssertionError("unexpected network operation")
        result = self.responses.pop(0)
        if isinstance(result, Exception):
            raise result
        return deepcopy(result)

    @property
    def submissions(self):
        return [call for call in self.calls if call[0].endswith("/dispatches") and call[1] is not None]


def binding():
    return {"repository": operator.REPOSITORY, "workflow": operator.WORKFLOW,
            "bundle_url": "https://releases.example/bundles/" + "a" * 64 + ".tar.gz",
            "bundle_sha256": "a" * 64, "manifest_sha256": "b" * 64,
            "source_commit": "c" * 40, "candidate_id": "d" * 64, "tag": "core-v0.47.0"}


class OperatorTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "requests"
        self.binding = binding()
        self.identifier = operator.request_id(self.binding)
        self.guard = patch.object(operator, "host_only")
        self.guard.start()
        self.addCleanup(self.guard.stop)

    def record(self):
        return operator.Requests(self.root).load(self.identifier)

    def submit(self):
        return operator._dispatch_admitted(self.root, self.binding, FakeGitHub([{"workflow_runs": []}, None]))

    def workflow_run(self, **changes):
        result = {"id": 101, "display_title": "release-request-" + self.identifier,
                  "head_sha": self.binding["source_commit"], "head_branch": self.binding["tag"],
                  "event": "workflow_dispatch", "path": operator.WORKFLOW,
                  "repository": {"full_name": operator.REPOSITORY},
                  "status": "completed", "conclusion": "success",
                  "created_at": "2026-10-23T10:00:00Z"}
        result.update(changes)
        return result

    def jobs(self, **changes):
        job = {"id": 201, "run_id": 101, "name": "signing-publishing", "status": "completed",
               "started_at": "2026-10-23T10:05:00Z", "completed_at": "2026-10-23T10:08:00Z", "steps": []}
        job.update(changes)
        return {"total_count": 1, "jobs": [job]}

    def observe(self, run=None, jobs=None, witness=False, **kwargs):
        api = FakeGitHub([{"workflow_runs": [run or self.workflow_run()]}, jobs or self.jobs()])
        if witness:
            api.completion_digest = kwargs.get("receipt_sha256")
        result = operator.observe(self.root, self.identifier, api,
                                  now=datetime(2026, 10, 23, 10, 10, tzinfo=timezone.utc), **kwargs)
        self.assertFalse(api.submissions)
        self.assertTrue(all(payload is None for _, payload in api.calls))
        return result

    def receipt(self, **changes):
        receipt = {"format": "gripsack.release-completion.v1", "request_id": self.identifier,
                   "binding": self.binding, "run_id": 101, "status": "complete",
                   "surfaces": {name: {"status": "verified", "evidence_sha256": "e" * 64}
                                for name in ("signed_manifest", "registries", "downloaded_assets",
                                             "installer", "default_helpers", "sdk", "site")}}
        receipt.update(changes)
        path = Path(self.temporary.name) / "completion.json"
        path.write_text(json.dumps(receipt))
        return {"receipt": path, "receipt_sha256": hashlib.sha256(path.read_bytes()).hexdigest()}

    def test_deterministic_identity_binds_every_field(self):
        self.assertEqual(self.identifier, operator.request_id(dict(reversed(list(self.binding.items())))))
        for field in ("candidate_id", "source_commit", "manifest_sha256", "tag"):
            changed = dict(self.binding)
            changed[field] = "core-v0.47.1" if field == "tag" else "f" * len(changed[field])
            with self.subTest(field=field):
                self.assertNotEqual(self.identifier, operator.request_id(changed))

    def test_private_fsynced_identity_precedes_all_network(self):
        states = []
        real_fsync = os.fsync
        syncs = []

        def fsync(fd):
            syncs.append(stat.S_ISDIR(os.fstat(fd).st_mode))
            return real_fsync(fd)

        def before_call(endpoint, payload):
            states.append(self.record()["state"])
            self.assertEqual(stat.S_IMODE(self.root.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE((self.root / (self.identifier + ".json")).stat().st_mode), 0o600)
            self.assertTrue(syncs)
            if endpoint.endswith("/dispatches"):
                self.assertEqual(self.record()["state"], "uncertain")
                self.assertIn(False, syncs)
                self.assertIn(True, syncs)

        api = FakeGitHub([{"workflow_runs": []}, None], before_call)
        with patch.object(operator.os, "fsync", side_effect=fsync):
            result = operator._dispatch_admitted(self.root, self.binding, api)
        self.assertEqual(states[0], "prepared")
        self.assertEqual(states[-1], "uncertain")
        self.assertEqual(result["status"], "submitted")
        self.assertEqual(api.submissions[0][1]["ref"], self.binding["tag"])
        self.assertEqual(api.submissions[0][1]["inputs"]["request_id"], self.identifier)

    def test_lost_response_never_resubmits_even_after_empty_observation(self):
        api = FakeGitHub([{"workflow_runs": []}, TimeoutError("lost response")])
        result = operator._dispatch_admitted(self.root, self.binding, api)
        self.assertEqual(result["dispatch_state"], "uncertain")
        self.assertEqual(len(api.submissions), 1)
        for _ in range(2):
            retry = FakeGitHub([{"workflow_runs": []}])
            self.assertEqual(operator._dispatch_admitted(self.root, self.binding, retry)["status"], "unknown")
            self.assertFalse(retry.submissions)
        self.assertEqual(self.record()["state"], "uncertain")

    def test_lost_response_reconciles_existing_run(self):
        operator._dispatch_admitted(self.root, self.binding,
                          FakeGitHub([{"workflow_runs": []}, ConnectionError("disconnected")]))
        api = FakeGitHub([{"workflow_runs": [self.workflow_run()]}])
        result = operator._dispatch_admitted(self.root, self.binding, api)
        self.assertEqual(result["run_id"], 101)
        self.assertEqual(result["status"], "existing")
        self.assertFalse(api.submissions)
        self.assertEqual(self.record()["run_id"], 101)
        direct = FakeGitHub([self.workflow_run()])
        self.assertEqual(operator._dispatch_admitted(self.root, self.binding, direct)["run_id"], 101)
        self.assertTrue(direct.calls[0][0].endswith("/actions/runs/101"))

    def test_pre_dispatch_observer_failure_does_not_dispatch(self):
        api = FakeGitHub([TimeoutError("observation unavailable")])
        result = operator._dispatch_admitted(self.root, self.binding, api)
        self.assertEqual(result["status"], "unknown")
        self.assertEqual(self.record()["state"], "prepared")
        self.assertFalse(api.submissions)

    def test_successfully_submitted_request_is_not_resubmitted(self):
        self.submit()
        api = FakeGitHub([{"workflow_runs": []}])
        self.assertEqual(operator._dispatch_admitted(self.root, self.binding, api)["status"], "unknown")
        self.assertFalse(api.submissions)

    def test_mismatched_run_source_tag_repository_workflow_are_unknown(self):
        self.submit()
        for changes in ({"head_sha": "f" * 40}, {"head_branch": "core-v0.48.0"},
                        {"repository": {"full_name": "attacker/gripsack"}},
                        {"path": ".github/workflows/other.yml"}, {"event": "push"}):
            with self.subTest(changes=changes):
                api = FakeGitHub([{"workflow_runs": [self.workflow_run(**changes)]}])
                result = operator.observe(self.root, self.identifier, api)
                self.assertEqual(result["status"], "unknown")
                self.assertFalse(api.submissions)

    def test_other_candidate_run_cannot_satisfy_request(self):
        self.submit()
        changed = dict(self.binding, candidate_id="f" * 64)
        other = self.workflow_run(display_title="release-request-" + operator.request_id(changed))
        api = FakeGitHub([{"workflow_runs": [other]}])
        result = operator.observe(self.root, self.identifier, api)
        self.assertIsNone(result["run_id"])
        self.assertEqual(result["status"], "unknown")

    def test_duplicate_request_runs_are_not_chosen_arbitrarily(self):
        api = FakeGitHub([{"workflow_runs": [self.workflow_run(), self.workflow_run(id=102)]}])
        result = operator._dispatch_admitted(self.root, self.binding, api)
        self.assertEqual(result["status"], "unknown")
        self.assertFalse(api.submissions)

    def test_bounded_incomplete_history_never_dispatches(self):
        page = {"workflow_runs": [{"display_title": "unrelated"}] * 100}
        api = FakeGitHub([page] * operator.MAX_PAGES)
        result = operator._dispatch_admitted(self.root, self.binding, api)
        self.assertEqual(result["status"], "unknown")
        self.assertFalse(api.submissions)

    def test_observer_failure_is_read_only_and_unknown(self):
        self.submit()
        before = {path.name: path.read_bytes() for path in self.root.iterdir()}
        api = FakeGitHub([ConnectionError("observer disconnected")])
        result = operator.observe(self.root, self.identifier, api)
        after = {path.name: path.read_bytes() for path in self.root.iterdir()}
        self.assertEqual(before, after)
        self.assertEqual(result["status"], "unknown")
        self.assertIsNone(result["queue_seconds"])
        self.assertFalse(api.submissions)

    def test_queue_and_execution_are_separate(self):
        self.submit()
        result = self.observe()
        self.assertEqual(result["queue_seconds"], 300)
        self.assertEqual(result["execution_seconds"], 180)
        self.assertEqual(result["status"], "partial")
        self.assertEqual(result["detail"], "signed_published_pending_consumer_verification")

    def test_queued_run_has_no_execution_duration(self):
        self.submit()
        result = self.observe(run=self.workflow_run(status="queued", conclusion=None,
                                                    run_started_at="2026-10-23T10:00:00Z"),
                              jobs={"jobs": [], "total_count": 0})
        self.assertEqual(result["status"], "queued")
        self.assertEqual(result["queue_seconds"], 600)
        self.assertEqual(result["execution_seconds"], 0)

    def test_active_signing_and_publishing_steps(self):
        self.submit()
        for step, expected in (("Sign release manifest", "signing"),
                               ("Publish qualified release", "publishing"), ("Validate bundle", "running")):
            with self.subTest(step=step):
                result = self.observe(run=self.workflow_run(status="in_progress", conclusion=None),
                                      jobs=self.jobs(status="in_progress", completed_at=None,
                                                     steps=[{"name": step, "status": "in_progress"}]))
                self.assertEqual(result["status"], expected)
                self.assertEqual(result["execution_seconds"], 300)

    def test_failed_workflow_is_partial_not_complete(self):
        self.submit()
        self.assertEqual(self.observe(run=self.workflow_run(conclusion="failure"))["status"], "partial")
        result = self.observe(run=self.workflow_run(conclusion="failure"), **self.receipt())
        self.assertEqual(result["status"], "unknown")

    def test_asserted_completion_is_not_evidence_even_with_authenticated_witness(self):
        self.submit()
        receipt = self.receipt()
        self.assertEqual(self.observe(**receipt)["status"], "unknown")
        self.assertEqual(self.observe(witness=True, **receipt)["status"], "unknown")
        receipt["receipt_sha256"] = "f" * 64
        self.assertEqual(self.observe(witness=True, **receipt)["status"], "unknown")

    def test_unauthorized_admission_publisher_never_dispatches(self):
        api = FakeGitHub([{"workflow_runs": []}])
        api.actor_id = 7
        result = operator._dispatch_admitted(self.root, self.binding, api)
        self.assertEqual(result["status"], "unknown")
        self.assertEqual(self.record()["state"], "prepared")
        self.assertFalse(api.submissions)
        self.assertFalse(api.witnesses)

    def test_partial_identity_and_candidate_mismatch_receipts_fail_closed(self):
        self.submit()
        for changes in ({"binding": {"source_commit": self.binding["source_commit"]}},
                        {"binding": dict(self.binding, candidate_id="f" * 64)},
                        {"run_id": 102}, {"request_id": "f" * 64}, {"surfaces": {}}):
            with self.subTest(changes=changes):
                self.assertEqual(self.observe(**self.receipt(**changes))["status"], "unknown")

    def test_wrong_job_run_and_truncated_jobs_are_unknown(self):
        self.submit()
        self.assertEqual(self.observe(jobs=self.jobs(run_id=102))["status"], "unknown")
        self.assertEqual(self.observe(jobs={"total_count": 2, "jobs": []})["status"], "unknown")

    def test_receipt_digest_is_not_optional(self):
        self.submit()
        with self.assertRaises(ValueError):
            operator.observe(self.root, self.identifier, FakeGitHub([]), receipt=Path("unused"))

    def test_insecure_or_mutable_bundle_urls_are_refused(self):
        for url in ("http://example/" + "a" * 64, "https://user:secret@example/" + "a" * 64,
                    "https://example/latest.tar.gz", self.binding["bundle_url"] + "?token=secret",
                    self.binding["bundle_url"] + "#fragment", "https://example/%2e%2e/" + "a" * 64):
            with self.subTest(url=url), self.assertRaises(ValueError):
                operator.request_id(dict(self.binding, bundle_url=url))

    def test_world_readable_state_directory_is_refused_before_network(self):
        self.root.mkdir(mode=0o755)
        self.root.chmod(0o755)
        api = FakeGitHub([])
        with self.assertRaises(ValueError):
            operator._dispatch_admitted(self.root, self.binding, api)
        self.assertFalse(api.calls)

    def test_symlink_record_is_refused(self):
        self.submit()
        path = self.root / (self.identifier + ".json")
        target = self.root / "moved.json"
        path.rename(target)
        path.symlink_to(target)
        with self.assertRaises(OSError):
            operator.observe(self.root, self.identifier, FakeGitHub([]))

    def test_corrupt_partial_request_identity_is_refused(self):
        self.submit()
        path = self.root / (self.identifier + ".json")
        record = self.record()
        del record["binding"]["candidate_id"]
        path.write_text(json.dumps(record))
        with self.assertRaises(ValueError):
            operator.observe(self.root, self.identifier, FakeGitHub([]))


class TransportBoundsTests(unittest.TestCase):
    def setUp(self):
        self.process = MagicMock()
        self.process.poll.return_value = None
        self.process.stdin = None
        self.selector = MagicMock()
        self.selector.get_map.return_value = {1: True}
        for target, value in (("host_only", MagicMock()),
                              ("subprocess.Popen", MagicMock(return_value=self.process)),
                              ("selectors.DefaultSelector", MagicMock(return_value=self.selector))):
            mocked = patch("release_operator." + target, value)
            mocked.start()
            self.addCleanup(mocked.stop)

    def test_timeout_kills_transport_without_waiting_indefinitely(self):
        with patch.object(operator.time, "monotonic", side_effect=[0, operator.API_TIMEOUT + 1]):
            with self.assertRaises(TimeoutError):
                operator.GitHub().api(f"repos/{operator.REPOSITORY}/actions/runs/101")
        self.process.kill.assert_called_once()
        self.process.wait.assert_called_once()
        self.selector.close.assert_called_once()

    def test_output_bound_kills_transport(self):
        key = MagicMock()
        key.fileobj = self.process.stdout
        key.data = "stdout"
        self.selector.select.return_value = [(key, 1)]
        with patch.object(operator.os, "read", return_value=b"x" * (operator.MAX_RESPONSE + 1)):
            with self.assertRaisesRegex(ValueError, "response exceeds bound"):
                operator.GitHub().api(f"repos/{operator.REPOSITORY}/actions/runs/101")
        self.process.kill.assert_called_once()
        self.process.wait.assert_called_once()


class HostBoundaryTests(unittest.TestCase):
    def test_worker_environment_is_refused_even_if_empty(self):
        with patch.dict(os.environ, {"CI_CANDIDATE_ID": ""}), patch.object(Path, "exists", return_value=False):
            with self.assertRaises(ValueError):
                operator.host_only()

    def test_docker_marker_is_refused(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(Path, "exists", return_value=True):
            with self.assertRaises(ValueError):
                operator.host_only()

    def test_duplicate_and_nonfinite_json_are_refused(self):
        with self.assertRaises(ValueError):
            operator.decode_json(b'{"a":1,"a":2}')
        with self.assertRaises(ValueError):
            operator.decode_json(b'{"a":NaN}')


if __name__ == "__main__":
    unittest.main()
