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
from pathlib import Path

REQUIRED_JOBS = ("test", "e2e-macos", "docs", "audit", "fuzz")
TEST_LANES = ("core", "e2e", "e2e-persistence", "formal", "e2e-macos")
PERSISTENCE_SCENARIOS = (
    "apply-deploy", "apply-prune", "rollback-deploy", "rollback-prune",
    "apply-deploy-copy", "apply-prune-copy",
)
PERSISTENCE_SHARDS = 8
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


def admit_test_lanes(results: dict) -> None:
    if set(results) != set(TEST_LANES):
        raise GateFailure("required test-lane inventory mismatch")
    for lane, record in results.items():
        if not isinstance(record, dict) or record.get("result") != "success":
            raise GateFailure(f"required test lane did not succeed: {lane}={record!r}")


def admit_persistence(reports: list[dict], source: str) -> None:
    """Require every native platform/fault/cut partition, not matrix roll-up alone."""
    if not source:
        raise GateFailure("missing persistence source revision")
    expected = {
        (system, scenario, fault, shard)
        for system in ("Linux", "Darwin")
        for scenario in PERSISTENCE_SCENARIOS
        for fault in ("error", "kill")
        for shard in range(PERSISTENCE_SHARDS)
    }
    seen = set()
    inventories = {}
    for report in reports:
        if not isinstance(report, dict):
            raise GateFailure("malformed persistence receipt")
        key = tuple(report.get(field) for field in ("system", "scenario", "fault", "shard"))
        if any(not isinstance(value, str) for value in key[:3]) or type(key[3]) is not int:
            raise GateFailure("malformed persistence partition identity")
        if key not in expected or key in seen:
            raise GateFailure(f"unexpected or duplicate persistence partition: {key}")
        seen.add(key)
        if report.get("source") != source or report.get("shards") != PERSISTENCE_SHARDS:
            raise GateFailure(f"wrong persistence revision or partition count: {key}")
        if report.get("machine") != ("arm64" if key[0] == "Darwin" else "x86_64"):
            raise GateFailure(f"wrong persistence native architecture: {key}")
        inventory = report.get("inventory")
        if (not isinstance(inventory, list) or len(inventory) < PERSISTENCE_SHARDS
                or any(not isinstance(row, list) or len(row) != 2
                       or any(not isinstance(value, str) or not value for value in row)
                       for row in inventory)):
            raise GateFailure(f"missing or vacuous persistence inventory: {key}")
        # Fault mode and shard cannot change the baseline trace on a platform.
        baseline = inventories.setdefault(key[:2], inventory)
        if inventory != baseline:
            raise GateFailure(f"persistence trace inventory changed between partitions: {key}")
        cuts = report.get("completed_cuts")
        if (not isinstance(cuts, list) or any(type(cut) is not int for cut in cuts)
                or cuts != list(range(key[3] + 1, len(inventory) + 1, PERSISTENCE_SHARDS))):
            raise GateFailure(f"missing, overlapping or substituted persistence cuts: {key}")
        drift = report.get("drift_states")
        if drift != [False, True] or any(type(value) is not bool for value in drift):
            raise GateFailure(f"missing persistence drift state: {key}")
        seconds = report.get("seconds")
        if type(seconds) not in (float, int) or not 0 < seconds < float("inf"):
            raise GateFailure(f"missing persistence execution timing: {key}")
    if seen != expected:
        raise GateFailure(f"missing required persistence partitions: {sorted(expected - seen)!r}")


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

    lanes = {lane: {"result": "success"} for lane in TEST_LANES}
    admit_test_lanes(lanes)
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
    def refuses(check, label):
        nonlocal negative
        try:
            check()
        except GateFailure:
            negative += 1
        else:
            raise GateFailure(f"calibration admitted {label}")

    for lane in TEST_LANES:
        refuses(lambda: admit_test_lanes({key: value for key, value in lanes.items() if key != lane}),
                f"missing test lane {lane}")
        for result in ("failure", "cancelled", "skipped", None):
            refuses(lambda: admit_test_lanes({**lanes, lane: {"result": result}}),
                    f"test lane {lane}={result}")
    receipts = [
        {"system": system, "machine": "arm64" if system == "Darwin" else "x86_64",
         "scenario": scenario, "fault": fault, "shard": shard, "shards": PERSISTENCE_SHARDS,
         "source": "calibration", "inventory": [["before", "FilePublish"]] * 19,
         "completed_cuts": list(range(shard + 1, 20, PERSISTENCE_SHARDS)),
         "drift_states": [False, True], "seconds": 1.0}
        for system in ("Linux", "Darwin")
        for scenario in PERSISTENCE_SCENARIOS
        for fault in ("error", "kill")
        for shard in range(PERSISTENCE_SHARDS)
    ]
    admit_persistence(receipts, "calibration")
    positive += 1
    for index in range(len(receipts)):
        refuses(lambda: admit_persistence(receipts[:index] + receipts[index + 1:], "calibration"),
                f"missing persistence partition {index}")
    refuses(lambda: admit_persistence(receipts + receipts[:1], "calibration"), "duplicate partition")
    for field, value in (
        ("completed_cuts", []), ("completed_cuts", [1, 9]), ("completed_cuts", [1, 9, 17, 17]),
        ("completed_cuts", [2, 10, 18]), ("drift_states", [False]), ("drift_states", [0, 1]),
        ("inventory", []), ("inventory", [["after", "FilePublish"]] * 19),
        ("source", "other"), ("machine", "arm64"), ("shards", 1), ("seconds", 0),
    ):
        refuses(lambda: admit_persistence([{**receipts[0], field: value}, *receipts[1:]], "calibration"),
                f"invalid persistence {field}")
    print(f"CI_GATE_POSITIVES={positive}")
    print(f"CI_GATE_NEGATIVES={negative}")
    print("CI gate calibration: OK")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-check", action="store_true")
    parser.add_argument("--test-lanes", action="store_true")
    parser.add_argument("--persistence-reports", type=Path)
    args = parser.parse_args()
    try:
        if args.self_check:
            self_check()
            return 0
        if args.test_lanes:
            admit_test_lanes(decode(os.environ.get("GRIPSACK_TEST_NEEDS", "")))
            if args.persistence_reports is None:
                raise GateFailure("required persistence receipts directory is missing")
            reports = [decode(path.read_text()) for path in sorted(args.persistence_reports.rglob("*.json"))]
            admit_persistence(reports, os.environ.get("GITHUB_SHA", ""))
            print(f"test aggregate OK: all required lanes and {len(reports)} persistence partitions succeeded")
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
