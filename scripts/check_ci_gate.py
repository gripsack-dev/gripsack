#!/usr/bin/env python3
"""Admit the actual GitHub `needs` results; only a named owner waiver can skip fuzz.

The feedback exception is plan/0048 §15, REL-FUZZ-2026-09-27. It is selected
explicitly on a manual dispatch, not inferred from a branch, CI variable or
missing result. All other required jobs must succeed, including when fuzz is
waived. This qualifies CI composition, not the whole release's implementation.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass

REQUIRED_JOBS = ("test", "e2e-macos", "docs", "audit", "fuzz")
FEEDBACK_WAIVER = "REL-FUZZ-2026-09-27"
MAX_RESULTS_BYTES = 64 * 1024
EVENTS = ("push", "pull_request", "workflow_dispatch")


class GateFailure(ValueError):
    pass


@dataclass(frozen=True)
class GateDecision:
    fuzz: str
    waiver: str | None


def unique_fields(pairs):
    result = {}
    for name, value in pairs:
        if name in result:
            raise GateFailure(f"duplicate result field {name!r}")
        result[name] = value
    return result


def decode(raw: str) -> dict:
    if len(raw.encode("utf-8")) > MAX_RESULTS_BYTES:
        raise GateFailure("CI result envelope exceeds 64 KiB")
    try:
        value = json.loads(raw, object_pairs_hook=unique_fields)
    except (json.JSONDecodeError, RecursionError) as error:
        raise GateFailure("invalid CI result JSON") from error
    if not isinstance(value, dict):
        raise GateFailure("CI results must be a job map")
    return value


def admit(results: dict, event: str, waiver: str) -> GateDecision:
    if event not in EVENTS:
        raise GateFailure(f"unsupported CI event {event!r}")
    if waiver not in ("none", FEEDBACK_WAIVER):
        raise GateFailure(f"unknown fuzz waiver {waiver!r}")
    waived = waiver == FEEDBACK_WAIVER
    if waived and event != "workflow_dispatch":
        raise GateFailure("the feedback waiver requires an explicit manual dispatch")
    if set(results) != set(REQUIRED_JOBS):
        missing = sorted(set(REQUIRED_JOBS) - set(results))
        unexpected = sorted(set(results) - set(REQUIRED_JOBS))
        raise GateFailure(f"required-job inventory mismatch: missing={missing!r}, unexpected={unexpected!r}")
    failures = []
    for name in REQUIRED_JOBS:
        record = results[name]
        expected = "skipped" if name == "fuzz" and waived else "success"
        actual = record.get("result") if isinstance(record, dict) else None
        if actual != expected:
            failures.append(f"{name}: expected {expected}, got {actual!r}")
    if failures:
        raise GateFailure("; ".join(failures))
    return GateDecision(
        fuzz="not_run_owner_waived" if waived else "passed",
        waiver=FEEDBACK_WAIVER if waived else None,
    )


def self_check() -> None:
    def clean():
        return {
            "test": {"result": "success"},
            "e2e-macos": {"result": "success"},
            "docs": {"result": "success"},
            "audit": {"result": "success"},
            "fuzz": {"result": "success"},
        }

    positive = 0
    for event in ("push", "pull_request", "workflow_dispatch"):
        assert admit(clean(), event, "none") == GateDecision("passed", None)
        positive += 1
    feedback = clean()
    feedback["fuzz"]["result"] = "skipped"
    assert admit(feedback, "workflow_dispatch", FEEDBACK_WAIVER) == GateDecision(
        "not_run_owner_waived", FEEDBACK_WAIVER,
    )
    positive += 1

    negative = 0

    def rejects(label, results, event="pull_request", waiver="none"):
        nonlocal negative
        try:
            admit(results, event, waiver)
        except GateFailure:
            negative += 1
            return
        raise GateFailure(f"calibration admitted {label}")

    for name in clean():
        missing = clean()
        del missing[name]
        rejects(f"missing {name}", missing)
        for outcome in ("failure", "cancelled", "skipped", "", None, "unknown"):
            failed = clean()
            failed[name]["result"] = outcome
            rejects(f"{name}={outcome!r}", failed)
        malformed = clean()
        malformed[name] = "success"
        rejects(f"malformed {name}", malformed)
    extra = clean()
    extra["new-required-lane"] = {"result": "success"}
    rejects("unreviewed new job inventory", extra)
    for name in clean():
        for outcome in ("failure", "cancelled", "success" if name == "fuzz" else "skipped"):
            failed = clean()
            failed["fuzz"]["result"] = "skipped"
            failed[name]["result"] = outcome
            rejects(f"waiver concealed {name}={outcome}", failed, "workflow_dispatch", FEEDBACK_WAIVER)
    rejects("unknown waiver", clean(), "workflow_dispatch", "unapproved")
    for event in ("push", "pull_request"):
        rejects("automatic waiver", feedback, event, FEEDBACK_WAIVER)
    rejects("unknown trigger", clean(), "unrecognized")
    for raw in ('[]', '{"test":{},"test":{}}', '{broken', ' ' * (MAX_RESULTS_BYTES + 1)):
        try:
            decode(raw)
        except GateFailure:
            negative += 1
        else:
            raise GateFailure("calibration admitted invalid result envelope")
    print(f"CI_GATE_POSITIVES={positive}")
    print(f"CI_GATE_NEGATIVES={negative}")
    print("CI gate calibration: OK")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-check", action="store_true")
    args = parser.parse_args()
    try:
        if args.self_check:
            self_check()
            return 0
        decision = admit(
            decode(os.environ.get("GRIPSACK_CI_RESULTS", "")),
            os.environ.get("GRIPSACK_CI_EVENT", ""),
            os.environ.get("GRIPSACK_FUZZ_WAIVER", "none"),
        )
    except GateFailure as error:
        print(f"CI gate refused: {error}", file=sys.stderr)
        return 1
    print(json.dumps({
        "format": "gripsack-ci-gate", "version": 1,
        "source": os.environ.get("GITHUB_SHA"),
        "required_jobs": list(REQUIRED_JOBS),
        "fuzz": decision.fuzz, "owner_waiver": decision.waiver,
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
