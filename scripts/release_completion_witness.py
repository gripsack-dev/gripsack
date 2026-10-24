"""Authenticated observation of the trusted host verifier's completed run.

Evidence files are necessary but not their own provenance. Only the host path
that actually executes release verification may publish this separate status.
"""
from __future__ import annotations

from release_admission import fetch_context_status
from release_manifest import REPOSITORY, require

CONTEXT = "release-consumer-verification"


def completion_payload(record, run, receipt_sha256):
    from release_operator import hex_digest, request_id, validate_run
    hex_digest(receipt_sha256)
    require(record.get("request_id") == request_id(record["binding"]), "completion request identity differs")
    validate_run(run, record)
    require(run.get("status") == "completed" and run.get("conclusion") == "success",
            "consumer verification cannot complete an unfinished or unsuccessful signing run")
    return {"context": CONTEXT, "state": "success",
            "description": "completion-sha256:" + receipt_sha256,
            "target_url": f"https://github.com/{REPOSITORY}/actions/runs/{run['id']}"}


def fetch_completion_status(record, run, receipt_sha256, api):
    return fetch_context_status(record["binding"], completion_payload(record, run, receipt_sha256), api)
