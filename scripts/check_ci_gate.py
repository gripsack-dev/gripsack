#!/usr/bin/env python3
"""Admit essential CI results and explicitly selected optional campaigns.

Unselected campaigns must be skipped, never represented as passing evidence.
Selected campaigns must succeed, including complete persistence receipts.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

REQUIRED_JOBS = ("test", "docs", "audit")
OPTIONAL_JOBS = ("fuzz",)
TEST_LANES = ("core", "e2e")
OPTIONAL_TEST_LANES = ("e2e-persistence", "formal", "gnu", "mutants")
PERSISTENCE_SCENARIOS = (
    "apply-deploy", "apply-prune", "rollback-deploy", "rollback-prune",
    "apply-deploy-copy", "apply-prune-copy",
)
PERSISTENCE_SHARDS = 8
MAX_RESULTS_BYTES = 64 * 1024
EVENTS = ("push", "pull_request", "workflow_dispatch")


class GateFailure(ValueError):
    pass



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


def selection(raw: str, allowed: tuple[str, ...]) -> tuple[str, ...]:
    names = tuple(raw.split(",")) if raw else ()
    if len(set(names)) != len(names) or set(names) - set(allowed):
        raise GateFailure(f"invalid optional gate selection: {raw!r}")
    return names


def admit_jobs(results: dict, required: tuple[str, ...],
               optional: tuple[str, ...], selected: tuple[str, ...]) -> None:
    if len(set(selected)) != len(selected) or set(selected) - set(optional):
        raise GateFailure("unknown or duplicate selected gates")
    if set(results) != set(required + optional):
        raise GateFailure("job inventory mismatch")
    for name, record in results.items():
        expected = "success" if name in required or name in selected else "skipped"
        actual = record.get("result") if isinstance(record, dict) else None
        if actual != expected:
            raise GateFailure(f"{name}: expected {expected}, got {actual!r}")


def admit(results: dict, event: str, selected: tuple[str, ...] = ()) -> None:
    if event not in EVENTS:
        raise GateFailure(f"unsupported CI event {event!r}")
    if selected and event != "workflow_dispatch":
        raise GateFailure("optional campaigns require explicit manual selection")
    admit_jobs(results, REQUIRED_JOBS, OPTIONAL_JOBS, selected)


def admit_test_lanes(results: dict, selected: tuple[str, ...] = ()) -> None:
    admit_jobs(results, TEST_LANES, OPTIONAL_TEST_LANES, selected)


def admit_persistence_partition(report: dict, source: str) -> tuple:
    """Admit one completed partition; aggregate inventory agreement is separate."""
    if not source:
        raise GateFailure("missing persistence source revision")
    if not isinstance(report, dict):
        raise GateFailure("malformed persistence receipt")
    key = tuple(report.get(field) for field in ("system", "scenario", "fault", "shard"))
    if any(not isinstance(value, str) for value in key[:3]) or type(key[3]) is not int:
        raise GateFailure("malformed persistence partition identity")
    if (key[0] != "Linux" or key[1] not in PERSISTENCE_SCENARIOS
            or key[2] not in ("error", "kill") or key[3] not in range(PERSISTENCE_SHARDS)):
        raise GateFailure(f"unexpected persistence partition: {key}")
    if report.get("source") != source or report.get("shards") != PERSISTENCE_SHARDS:
        raise GateFailure(f"wrong persistence revision or partition count: {key}")
    if report.get("machine") != "x86_64":
        raise GateFailure(f"wrong persistence native architecture: {key}")
    inventory = report.get("inventory")
    if (not isinstance(inventory, list) or len(inventory) < PERSISTENCE_SHARDS
            or any(not isinstance(row, list) or len(row) != 2
                   or any(not isinstance(value, str) or not value for value in row)
                   for row in inventory)):
        raise GateFailure(f"missing or vacuous persistence inventory: {key}")
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
    return key


def admit_persistence(reports: list[dict], source: str) -> None:
    """Require all 96 Linux fault/cut partitions, not matrix roll-up alone."""
    if not source:
        raise GateFailure("missing persistence source revision")
    expected = {
        (system, scenario, fault, shard)
        for system in ("Linux",)
        for scenario in PERSISTENCE_SCENARIOS
        for fault in ("error", "kill")
        for shard in range(PERSISTENCE_SHARDS)
    }
    seen = set()
    inventories = {}
    for report in reports:
        key = admit_persistence_partition(report, source)
        if key in seen:
            raise GateFailure(f"unexpected or duplicate persistence partition: {key}")
        seen.add(key)
        inventory = report["inventory"]
        # Fault mode and shard cannot change the baseline trace on a platform.
        baseline = inventories.setdefault(key[:2], inventory)
        if inventory != baseline:
            raise GateFailure(f"persistence trace inventory changed between partitions: {key}")
    if seen != expected:
        raise GateFailure(f"missing required persistence partitions: {sorted(expected - seen)!r}")


def self_check() -> None:
    positive = 0
    negative = 0

    def refuses(check, label):
        nonlocal negative
        try:
            check()
        except GateFailure:
            negative += 1
        else:
            raise GateFailure(f"calibration admitted {label}")

    # Every optional subset: an omitted or unsuccessful selected gate must fail.
    for required, optional in (
        (REQUIRED_JOBS, OPTIONAL_JOBS), (TEST_LANES, OPTIONAL_TEST_LANES),
    ):
        for mask in range(1 << len(optional)):
            selected = tuple(name for index, name in enumerate(optional) if mask & (1 << index))
            results = {name: {"result": "success" if name in required + selected else "skipped"}
                       for name in required + optional}
            admit_jobs(results, required, optional, selected)
            positive += 1
            for name in results:
                refuses(lambda: admit_jobs({key: value for key, value in results.items() if key != name},
                                            required, optional, selected), f"missing {name}")
                for outcome in ("failure", "cancelled", None, "unknown",
                                "skipped" if name in required + selected else "success"):
                    refuses(lambda: admit_jobs({**results, name: {"result": outcome}},
                                                required, optional, selected), f"{name}={outcome}")
            refuses(lambda: admit_jobs({**results, "unknown": {"result": "success"}},
                                        required, optional, selected), "unexpected gate")
            refuses(lambda: admit_jobs(results, required, optional, ("unknown",)), "unknown selection")
            if optional:
                refuses(lambda: admit_jobs(results, required, optional, (optional[0], optional[0])),
                        "duplicate selection")
    for event in ("push", "pull_request"):
        refuses(lambda: admit({}, event, ("fuzz",)), "automatic optional campaign")
    refuses(lambda: admit({}, "unknown"), "unknown event")
    for raw in ('[]', '{"test":{},"test":{}}', '{broken', ' ' * (MAX_RESULTS_BYTES + 1)):
        refuses(lambda: decode(raw), "invalid envelope")
    for raw in ("unknown", "fuzz,fuzz", ",fuzz"):
        refuses(lambda: selection(raw, OPTIONAL_JOBS), "invalid selection")
    receipts = [
        {"system": system, "machine": "x86_64",
         "scenario": scenario, "fault": fault, "shard": shard, "shards": PERSISTENCE_SHARDS,
         "source": "calibration", "inventory": [["before", "FilePublish"]] * 19,
         "completed_cuts": list(range(shard + 1, 20, PERSISTENCE_SHARDS)),
         "drift_states": [False, True], "seconds": 1.0}
        for system in ("Linux",)
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
        ("system", "Darwin"),
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
    parser.add_argument("--selected", default="")
    parser.add_argument("--persistence-reports", type=Path)
    args = parser.parse_args()
    try:
        if args.self_check:
            self_check()
            return 0
        if args.test_lanes:
            selected = selection(args.selected, OPTIONAL_TEST_LANES)
            if selected and os.environ.get("GRIPSACK_CI_EVENT") != "workflow_dispatch":
                raise GateFailure("optional campaigns require explicit manual selection")
            admit_test_lanes(decode(os.environ.get("GRIPSACK_TEST_NEEDS", "")), selected)
            if "e2e-persistence" in selected:
                if args.persistence_reports is None:
                    raise GateFailure("selected persistence receipts directory is missing")
                reports = [decode(path.read_text()) for path in sorted(args.persistence_reports.rglob("*.json"))]
                admit_persistence(reports, os.environ.get("GITHUB_SHA", ""))
            elif args.persistence_reports is not None:
                raise GateFailure("persistence receipts supplied without selecting persistence")
            print("test aggregate OK: every selected lane succeeded")
            return 0
        selected = selection(args.selected, OPTIONAL_JOBS)
        admit(decode(os.environ.get("GRIPSACK_CI_RESULTS", "")),
              os.environ.get("GRIPSACK_CI_EVENT", ""), selected)
    except GateFailure as error:
        print(f"CI gate refused: {error}", file=sys.stderr)
        return 1
    print(json.dumps({
        "format": "gripsack-ci-gate", "version": 2,
        "source": os.environ.get("GITHUB_SHA"),
        "required_jobs": list(REQUIRED_JOBS + selected),
        "not_run": sorted(set(OPTIONAL_JOBS) - set(selected)),
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
