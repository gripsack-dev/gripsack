#!/usr/bin/env python3
"""Host-only, journalled GitHub release dispatch and read-only observation.

A dispatched request is never automatically dispatched again, including when an
API response is lost and a subsequent successful run listing is empty. Completion
requires an independently verified receipt, not a green workflow run.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import stat
import subprocess
import tempfile
import time
from urllib.parse import urlsplit

from release_manifest import MAX_JSON_BYTES, canonical, decode_json, load_json, regular, validate_manifest

REPOSITORY = "gripsack-dev/gripsack"
WORKFLOW = ".github/workflows/release-core.yml"
HEX = re.compile(r"[0-9a-f]{64}\Z")
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
TAG = re.compile(r"core-v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?\Z")
MAX_RESPONSE = 2 * 1024 * 1024
API_TIMEOUT = 30
MAX_PAGES = 10


def host_only():
    if Path("/.dockerenv").exists() or "CI_CANDIDATE_ID" in os.environ:
        raise ValueError("release operator credentials are host-only; container/worker execution refused")


def hex_digest(value):
    if not isinstance(value, str) or not HEX.fullmatch(value):
        raise ValueError("expected a lowercase SHA256 digest")
    return value


def validate_binding(binding):
    expected = {"repository", "workflow", "bundle_url", "bundle_sha256",
                "manifest_sha256", "source_commit", "tag", "candidate_id"}
    if not isinstance(binding, dict) or set(binding) != expected:
        raise ValueError("request identity is incomplete or has unknown fields")
    if binding["repository"] != REPOSITORY or binding["workflow"] != WORKFLOW:
        raise ValueError("request repository/workflow mismatch")
    for field in ("bundle_sha256", "manifest_sha256", "candidate_id"):
        hex_digest(binding[field])
    if not isinstance(binding["source_commit"], str) or not COMMIT.fullmatch(binding["source_commit"]):
        raise ValueError("request must bind an exact source commit")
    if not isinstance(binding["tag"], str) or not TAG.fullmatch(binding["tag"]):
        raise ValueError("invalid core tag nomination")
    url = binding["bundle_url"]
    if not isinstance(url, str) or len(url) > 2048 or any(ord(c) < 33 or ord(c) > 126 for c in url):
        raise ValueError("invalid bundle URL")
    parsed = urlsplit(url)
    if (parsed.scheme != "https" or not parsed.hostname or parsed.username is not None
            or parsed.password is not None or parsed.port not in (None, 443)
            or parsed.query or parsed.fragment or "%" in parsed.path
            or "\\" in url or any(p in (".", "..") for p in parsed.path.split("/"))):
        raise ValueError("bundle URL must be credential-free, immutable HTTPS")
    # Content addressing is explicit in the path, not a mutable 'latest' alias
    # or expiring credential-bearing query. The signer still checks actual bytes.
    if not re.search(r"(?:^|/)" + binding["bundle_sha256"] + r"(?:/|\.[A-Za-z0-9.]+$|$)", parsed.path):
        raise ValueError("bundle URL path must contain its SHA256 as a component or filename")
    return binding


def request_id(binding):
    validate_binding(binding)
    return hashlib.sha256(canonical({"format": "gripsack.release-request.v1", **binding})).hexdigest()


def json_snapshot(path):
    """Hash and decode identical bounded bytes, never two mutable source reads."""
    fd = os.open(regular(Path(path)), os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as source:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_JSON_BYTES:
            raise ValueError("release JSON must be a bounded regular file")
        data = source.read(MAX_JSON_BYTES + 1)
    if len(data) > MAX_JSON_BYTES:
        raise ValueError("release JSON exceeds bound")
    return decode_json(data), hashlib.sha256(data).hexdigest()


def make_binding(manifest_path, bundle_url, bundle_sha256):
    value, manifest_sha256 = json_snapshot(manifest_path)
    manifest = validate_manifest(value)
    return validate_binding({
        "repository": REPOSITORY, "workflow": manifest["signer"]["workflow"],
        "bundle_url": bundle_url, "bundle_sha256": bundle_sha256,
        "manifest_sha256": manifest_sha256, "source_commit": manifest["source"]["commit"],
        "candidate_id": manifest["candidate_id"], "tag": manifest["signer"]["ref"].removeprefix("refs/tags/"),
    })


def now_text():
    return datetime.now(timezone.utc).isoformat()


def private_file(fd):
    info = os.fstat(fd)
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
            or info.st_mode & 0o077 or info.st_nlink != 1):
        raise ValueError("request files must be private, owned regular files without hardlinks")


class Requests:
    def __init__(self, root):
        self.root = Path(root).absolute()

    @contextmanager
    def locked(self, identifier, *, create=False):
        hex_digest(identifier)
        if create:
            try:
                self.root.mkdir(mode=0o700, parents=False)
                parent = os.open(self.root.parent, os.O_RDONLY | os.O_DIRECTORY)
                try:
                    os.fsync(parent)
                finally:
                    os.close(parent)
            except FileExistsError:
                pass
        info = self.root.lstat()
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid()
                or info.st_mode & 0o077):
            raise ValueError("request directory must be private and owned, not a symlink")
        flags = os.O_RDWR | os.O_NOFOLLOW | (os.O_CREAT if create else 0)
        fd = os.open(self.root / (identifier + ".lock"), flags, 0o600)
        try:
            private_file(fd)
            fcntl.flock(fd, fcntl.LOCK_EX if create else fcntl.LOCK_SH)
            yield
        finally:
            os.close(fd)

    def load(self, identifier):
        hex_digest(identifier)
        path = self.root / (identifier + ".json")
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        except FileNotFoundError:
            return None
        try:
            private_file(fd)
            if os.fstat(fd).st_size > 16384:
                raise ValueError("request record exceeds bound")
            record = load_json(path)
        finally:
            os.close(fd)
        if (not isinstance(record, dict)
                or set(record) != {"format", "request_id", "binding", "state", "created_at", "run_id"}
                or record["format"] != "gripsack.release-request.v1"
                or record["request_id"] != identifier
                or request_id(record["binding"]) != identifier
                or record["state"] not in {"prepared", "uncertain", "submitted", "reconciled"}
                or (record["run_id"] is not None
                    and (type(record["run_id"]) is not int or record["run_id"] <= 0))):
            raise ValueError("invalid durable request record")
        timestamp(record["created_at"])
        return record

    def save(self, record):
        identifier = hex_digest(record["request_id"])
        fd, temporary = tempfile.mkstemp(prefix=".request-", dir=self.root)
        try:
            with os.fdopen(fd, "wb") as stream:
                stream.write(canonical(record))
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, self.root / (identifier + ".json"))
            directory = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)


class GitHub:
    """No shell, bounded stdout/stderr, fixed GitHub host and per-call deadline."""
    def api(self, endpoint, payload=None):
        host_only()
        base = f"repos/{REPOSITORY}"
        if endpoint == "user":
            if payload is not None:
                raise ValueError("operator authentication is read-only")
        elif endpoint != base and not endpoint.startswith(base + "/"):
            raise ValueError("unexpected GitHub API endpoint")
        command = ["gh", "api", "--hostname", "github.com", "--method",
                   "POST" if payload is not None else "GET", endpoint]
        if payload is not None:
            command += ["--input", "-"]
        environment = dict(os.environ, GH_PROMPT_DISABLED="1", GH_PAGER="cat")
        process = subprocess.Popen(command, stdin=subprocess.PIPE if payload is not None else subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment)
        selector = selectors.DefaultSelector()
        chunks = {"stdout": bytearray(), "stderr": bytearray()}
        deadline = time.monotonic() + API_TIMEOUT
        try:
            if payload is not None:
                process.stdin.write(canonical(payload))
                process.stdin.close()
            selector.register(process.stdout, selectors.EVENT_READ, "stdout")
            selector.register(process.stderr, selectors.EVENT_READ, "stderr")
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("GitHub API deadline exceeded; dispatch may have been accepted")
                for key, _ in selector.select(remaining):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    chunks[key.data].extend(chunk)
                    if sum(map(len, chunks.values())) > MAX_RESPONSE:
                        raise ValueError("GitHub API response exceeds bound")
            process.wait(timeout=max(0.001, deadline - time.monotonic()))
            if process.returncode:
                # Do not expose arbitrary remote messages or credentials in errors.
                raise RuntimeError(f"GitHub API failed with exit status {process.returncode}")
            raw = bytes(chunks["stdout"])
            try:
                return decode_json(raw) if raw.strip() else None
            except RecursionError as error:
                raise ValueError("GitHub API JSON nesting exceeds bound") from error
        finally:
            selector.close()
            if process.poll() is None:
                process.kill()
            process.wait()
            process.stdout.close()
            process.stderr.close()
            if process.stdin is not None and not process.stdin.closed:
                process.stdin.close()


def validate_run(run, record):
    binding = record["binding"]
    if (not isinstance(run, dict) or type(run.get("id")) is not int or run["id"] <= 0
            or run.get("display_title") != "release-request-" + record["request_id"]
            or run.get("head_sha") != binding["source_commit"]
            or run.get("head_branch") != binding["tag"]
            or run.get("event") != "workflow_dispatch" or run.get("path") != WORKFLOW
            or not isinstance(run.get("repository"), dict)
            or run["repository"].get("full_name") != REPOSITORY
            or (record["run_id"] is not None and record["run_id"] != run["id"])):
        raise ValueError("GitHub run does not match the complete request identity")
    return run


def reconcile(api, record):
    if record["run_id"] is not None:
        run = api.api(f"repos/{REPOSITORY}/actions/runs/{record['run_id']}")
        return validate_run(run, record)
    found = None
    for page in range(1, MAX_PAGES + 1):
        result = api.api(f"repos/{REPOSITORY}/actions/workflows/release-core.yml/runs"
                         f"?event=workflow_dispatch&per_page=100&page={page}")
        if not isinstance(result, dict) or not isinstance(result.get("workflow_runs"), list):
            raise ValueError("invalid workflow run listing")
        runs = result["workflow_runs"]
        if len(runs) > 100:
            raise ValueError("workflow run page exceeds bound")
        for run in runs:
            if not isinstance(run, dict):
                raise ValueError("invalid workflow run entry")
            if run.get("display_title") == "release-request-" + record["request_id"]:
                validate_run(run, record)
                if found is not None and found["id"] != run["id"]:
                    raise ValueError("multiple runs claim the same release request")
                found = run
        if len(runs) < 100:
            return found
    raise ValueError("workflow run reconciliation exceeds bounded history; no dispatch allowed")


def authenticate_operator(api):
    from release_admission import CREATOR_ID
    actor = api.api("user")
    if not isinstance(actor, dict) or type(actor.get("id")) is not int or actor["id"] != CREATOR_ID:
        raise ValueError("GitHub account is not the authorized local admission publisher")


def dispatch(root, source, state, archive, bundle_url, api=None):
    """Re-admit private controller evidence; callers cannot supply a green binding."""
    from release_admission import admit_local
    host_only()
    admission = admit_local(source, state, archive, bundle_url)
    return _dispatch_admitted(root, admission["binding"], api)


def _dispatch_admitted(root, binding, api=None):
    host_only()
    identifier = request_id(binding)
    store = Requests(root)
    api = api or GitHub()
    with store.locked(identifier, create=True):
        record = store.load(identifier)
        if record is None:
            record = {"format": "gripsack.release-request.v1", "request_id": identifier,
                      "binding": binding, "state": "prepared", "created_at": now_text(), "run_id": None}
            store.save(record)  # Durable identity precedes even the first network read.
        try:
            run = reconcile(api, record)
        except (OSError, ValueError, RuntimeError, TimeoutError, subprocess.SubprocessError) as error:
            return {"request_id": identifier, "run_id": record["run_id"], "status": "unknown",
                    "dispatch_state": record["state"], "detail": str(error)}
        if run is not None:
            record.update(state="reconciled", run_id=run["id"])
            store.save(record)
            return {"request_id": identifier, "run_id": run["id"], "status": "existing"}
        if record["state"] != "prepared":
            return {"request_id": identifier, "run_id": None, "status": "unknown",
                    "dispatch_state": record["state"], "detail": "No visible run; automatic resubmission prohibited"}
        # This private state-machine path is entered only after controller
        # admission. Publish a distinct authenticated local witness, not a
        # fabricated hosted-test result, and independently read it back.
        try:
            from release_admission import fetch_status, status_payload
            authenticate_operator(api)
            api.api(f"repos/{REPOSITORY}/statuses/{binding['source_commit']}", status_payload(binding))
            fetch_status(binding, api.api)
        except (OSError, ValueError, RuntimeError, TimeoutError, subprocess.SubprocessError) as error:
            return {"request_id": identifier, "run_id": None, "status": "unknown",
                    "dispatch_state": record["state"], "detail": str(error)}
        record["state"] = "uncertain"
        store.save(record)  # Crash/lost response from this point must never cause resubmission.
        inputs = {key: binding[key] for key in ("bundle_url", "bundle_sha256", "manifest_sha256",
                                               "source_commit", "tag", "candidate_id")}
        inputs["request_id"] = identifier
        try:
            api.api(f"repos/{REPOSITORY}/actions/workflows/release-core.yml/dispatches",
                    {"ref": binding["tag"], "inputs": inputs})
        except (OSError, ValueError, RuntimeError, TimeoutError, subprocess.SubprocessError) as error:
            return {"request_id": identifier, "run_id": None, "status": "unknown",
                    "dispatch_state": "uncertain", "detail": str(error)}
        record["state"] = "submitted"
        store.save(record)
        return {"request_id": identifier, "run_id": None, "status": "submitted"}


def timestamp(value):
    if not isinstance(value, str) or len(value) > 64:
        raise ValueError("invalid observation timestamp")
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("observation timestamp lacks timezone")
    return parsed


def durations(run, jobs, now):
    created = timestamp(run["created_at"])
    starts = [timestamp(job["started_at"]) for job in jobs if job.get("started_at")]
    # run_started_at can equal creation while the run is still queued. Actual
    # job starts, not that field, establish execution commencement.
    start = min(starts) if starts else None
    if start is None:
        end = timestamp(run["updated_at"]) if run.get("status") == "completed" else now
        return {"queue_seconds": max(0, (end - created).total_seconds()), "execution_seconds": 0}
    ends = [timestamp(job["completed_at"]) for job in jobs if job.get("completed_at")]
    end = max(ends) if run.get("status") == "completed" and ends else now
    return {"queue_seconds": max(0, (start - created).total_seconds()),
            "execution_seconds": max(0, (end - start).total_seconds())}


def verify_completion(path, expected_digest, record, run, api):
    from release_completion_witness import fetch_completion_status
    from release_evidence import verify_completion as admit_completion
    # Local files and caller-supplied hashes alone cannot authenticate that
    # consumer verification ran. Require the trusted verifier's independent
    # source/run-bound witness before admitting the referenced evidence.
    fetch_completion_status(record, run, expected_digest, api.api)
    return admit_completion(path, expected_digest, record, run)


def observe(root, identifier, api=None, *, receipt=None, receipt_sha256=None, now=None):
    host_only()
    if (receipt is None) != (receipt_sha256 is None):
        raise ValueError("completion requires both receipt and independently verified SHA256")
    store = Requests(root)
    with store.locked(identifier):
        record = store.load(identifier)
    if record is None:
        raise ValueError("unknown durable release request")
    result = {"request_id": identifier, "run_id": record["run_id"], "status": "unknown",
              "queue_seconds": None, "execution_seconds": None}
    try:
        api = api or GitHub()
        run = reconcile(api, record)
        if run is None:
            result["detail"] = "No visible matching run; observation never resubmits"
            return result
        result["run_id"] = run["id"]
        response = api.api(f"repos/{REPOSITORY}/actions/runs/{run['id']}/jobs?filter=latest&per_page=100")
        if (not isinstance(response, dict) or not isinstance(response.get("jobs"), list)
                or type(response.get("total_count")) is not int
                or response["total_count"] != len(response["jobs"]) or len(response["jobs"]) > 100):
            raise ValueError("incomplete or oversized workflow job observation")
        jobs = response["jobs"]
        if any(not isinstance(job, dict) or job.get("run_id") != run["id"] for job in jobs):
            raise ValueError("workflow job identity mismatch")
        result.update(durations(run, jobs, now or datetime.now(timezone.utc)))
        status = run.get("status")
        if status in {"queued", "requested", "waiting", "pending"}:
            result["status"] = "queued"
        elif status == "in_progress":
            result["status"] = "running"
            for job in jobs:
                if job.get("name") == "signing-publishing" and job.get("status") == "in_progress":
                    steps = job.get("steps", [])
                    if (not isinstance(steps, list) or len(steps) > 100
                            or any(not isinstance(step, dict) for step in steps)):
                        raise ValueError("invalid or oversized workflow steps")
                    for step in steps:
                        if step.get("status") == "in_progress":
                            if step.get("name") == "Sign release manifest":
                                result["status"] = "signing"
                            elif step.get("name") in {"Publish qualified release", "Publish qualified packages"}:
                                result["status"] = "publishing"
        elif status == "completed":
            result["status"] = "partial"
            result["detail"] = ("signed_published_pending_consumer_verification"
                                if run.get("conclusion") == "success" else "workflow_not_successful")
        if receipt is not None:
            verify_completion(receipt, receipt_sha256, record, run, api)
            result.update(status="complete", completion_receipt_sha256=receipt_sha256)
            result.pop("detail", None)
    except (OSError, ValueError, RuntimeError, TimeoutError, KeyError, TypeError,
            subprocess.SubprocessError) as error:
        result.update(status="unknown", detail=str(error))
    return result


def verify_consumers(root, identifier, source, example, output, previous_version=None, api=None):
    """Run real consumers before publishing their authenticated completion witness."""
    from release_completion_witness import completion_payload, fetch_completion_status
    from release_evidence import verify_completion as admit_completion
    from release_manifest import digest
    from release_verify import Verification
    host_only()
    store = Requests(root)
    with store.locked(identifier):
        record = store.load(identifier)
    if record is None:
        raise ValueError("unknown durable release request")
    api = api or GitHub()
    run = reconcile(api, record)
    if run is None or run.get("status") != "completed" or run.get("conclusion") != "success":
        return {"request_id": identifier, "run_id": None if run is None else run["id"],
                "status": "partial", "detail": "signing/publication is not successfully complete"}
    authenticate_operator(api)
    with store.locked(identifier, create=True):
        current = store.load(identifier)
        if current is None or current["binding"] != record["binding"]:
            raise ValueError("release request changed during verification admission")
        current.update(state="reconciled", run_id=run["id"])
        store.save(current)
        record = current
    verifier = Verification(argparse.Namespace(
        request=store.root / f"{identifier}.json", run_id=run["id"], source=Path(source),
        example=Path(example), output=Path(output), previous_version=previous_version,
    ))
    receipt = verifier.execute()
    receipt_sha256 = digest(receipt)
    result = {"request_id": identifier, "run_id": run["id"], "receipt": str(receipt),
              "receipt_sha256": receipt_sha256, "status": "partial",
              "detail": verifier.receipt["status"]}
    if verifier.receipt["status"] != "complete":
        return result
    run = reconcile(api, record)
    if run is None or run.get("status") != "completed" or run.get("conclusion") != "success":
        result["detail"] = "signing run changed while consumers were being verified"
        return result
    admit_completion(receipt, receipt_sha256, record, run)
    try:
        api.api(f"repos/{REPOSITORY}/statuses/{record['binding']['source_commit']}",
                completion_payload(record, run, receipt_sha256))
        fetch_completion_status(record, run, receipt_sha256, api.api)
    except (OSError, ValueError, RuntimeError, TimeoutError, subprocess.SubprocessError) as error:
        result.update(status="unknown", detail=f"consumer witness observation failed: {error}")
        return result
    result.update(status="complete")
    result.pop("detail", None)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    submit = commands.add_parser("dispatch", help="durably nominate once, without waiting")
    submit.add_argument("--state-dir", type=Path, required=True)
    submit.add_argument("--source", type=Path, required=True, help="clean frozen candidate checkout")
    submit.add_argument("--ci-state", type=Path, required=True, help="selected private Buildbot installation")
    submit.add_argument("--archive", type=Path, required=True, help="locally assembled immutable release bundle")
    submit.add_argument("--bundle-url", required=True)
    watch = commands.add_parser("observe", help="read-only run reconciliation; never dispatches")
    watch.add_argument("--state-dir", type=Path, required=True)
    watch.add_argument("--request-id", required=True)
    watch.add_argument("--completion-receipt", type=Path)
    watch.add_argument("--receipt-sha256", help="digest obtained from independent completion verification")
    verify = commands.add_parser("verify-consumers", help="execute real consumers, then publish their completion witness")
    verify.add_argument("--state-dir", type=Path, required=True)
    verify.add_argument("--request-id", required=True)
    verify.add_argument("--source", type=Path, required=True)
    verify.add_argument("--example", type=Path, required=True)
    verify.add_argument("--output", type=Path, required=True, help="new private retained evidence directory")
    verify.add_argument("--previous-version")
    args = parser.parse_args(argv)
    try:
        host_only()
        if args.command == "dispatch":
            result = dispatch(args.state_dir, args.source, args.ci_state, args.archive, args.bundle_url)
        elif args.command == "verify-consumers":
            result = verify_consumers(args.state_dir, args.request_id, args.source, args.example,
                                      args.output, args.previous_version)
        else:
            result = observe(args.state_dir, args.request_id, receipt=args.completion_receipt,
                             receipt_sha256=args.receipt_sha256)
        print(json.dumps(result, sort_keys=True))
        return 2 if result["status"] == "unknown" else 3 if result["status"] == "partial" else 0
    except (OSError, ValueError, RuntimeError, KeyError, TypeError, subprocess.SubprocessError) as error:
        parser.exit(2, f"release operator: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
