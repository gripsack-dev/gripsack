#!/usr/bin/env python3
"""Calibrate the delivery gate against completed and falsified claims.

A synthetic fully evidenced H0/A0 ledger must close, while each
mutation must fail for its named property. Pending future rows remain
legal in inventory mode and cannot close an unrelated scope.
"""
from __future__ import annotations

import copy
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CHECKER = REPO / "scripts/check_delivery.py"
LEDGER = REPO / "verification/delivery.json"
REVISION = "a" * 40
REPORT = "verification/reports/calibration.log"
MARKER = "fixture runner: 1 passed, 0 failed, 0 skipped"


def run(path: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(CHECKER), "--ledger", str(path), *args],
        text=True, capture_output=True,
    )


def row(ledger: dict, identity: str) -> dict:
    return next(entry for entry in ledger["requirements"] if entry["id"] == identity)


def record(req: dict, lane: str, milestone: str | None = None) -> dict:
    data = {
        "kind": "runner",
        "lane": lane,
        "date": "2026-09-24",
        "entry_points": ["calibration fixture command"],
        "environment": "calibration fixture on Linux",
        "tool_versions": "fixture-runner 1.0",
        "inputs": "fixture case set v1",
        "command": "fixture-runner --case example",
        "result": "pass",
        "report": REPORT,
        "report_sha256": hashlib.sha256((MARKER + "\n").encode()).hexdigest(),
        "report_marker": MARKER,
        "commit": REVISION,
        "counts": {"executed": 1, "passed": 1, "failed": 0, "skipped": 0},
        "cases_covered": req["case_and_proof_inventory"],
    }
    if milestone:
        data["milestone"] = milestone
    if req["id"] == "G-03" or req["id"] == "A1-04":
        data["obligations"] = {"expected": 1, "checked": 1, "failed": 0, "skipped": 0}
    return data


def fixture(tmp: Path) -> tuple[dict, Path]:
    global REVISION
    subprocess.run(["git", "init", "-q", str(tmp)], check=True)
    (tmp / "scripts").mkdir()
    (tmp / "scripts/calibration.py").write_text("print('synthetic source')\n")
    subprocess.run(["git", "-C", str(tmp), "add", "scripts/calibration.py"], check=True)
    subprocess.run(
        ["git", "-C", str(tmp), "-c", "user.name=Checker Calibration",
         "-c", "user.email=checker@localhost", "commit",
         "-q", "-m", "synthetic evidence source"], check=True,
    )
    REVISION = subprocess.run(
        ["git", "-C", str(tmp), "rev-parse", "HEAD"],
        text=True, capture_output=True, check=True,
    ).stdout.strip()
    (tmp / "verification/reports").mkdir(parents=True)
    (tmp / REPORT).write_text(MARKER + "\n")
    ledger = json.loads(LEDGER.read_text())
    # Synthetic support matrix ONLY for a checker fixture. The real
    # ledger must retain the handover's unresolved platform lanes.
    for req in ledger["requirements"]:
        if req.get("lane_inventory_state") != "registered":
            req["lane_inventory_state"] = "registered"
            req["required_platform_capability_lanes"] = ["pure"]
            req["case_and_proof_inventory"] = [req["required_acceptance_evidence"]]
        req["evidence_kinds"] = ["runner"]
    for identity in ("H0-01", "H0-02", "A0-01"):
        req = row(ledger, identity)
        previous = req["status"]
        if previous != "verified":
            req.setdefault("status_history", []).append(
                {"from": previous, "to": "verified", "date": "2026-09-24"}
            )
        req["status"] = "verified"
        req["lane_status"] = {lane: "verified" for lane in req["required_platform_capability_lanes"]}
        req["evidence_records"] = [record(req, lane) for lane in req["lane_status"]]
    for req in ledger["requirements"]:
        if req["id"] in ledger["global_gate_ids"]:
            req["evidence_records"] = [
                record(req, lane, milestone)
                for milestone in ("H0", "A0")
                for lane in req["required_platform_capability_lanes"]
            ]
    path = tmp / "verification/delivery.json"
    path.write_text(json.dumps(ledger))
    return ledger, path


def check(path: Path, ledger: dict, phrase: str, *args: str) -> None:
    path.write_text(json.dumps(ledger))
    outcome = run(path, *args)
    if outcome.returncode == 0 or phrase not in outcome.stderr:
        raise AssertionError(
            f"expected rejection containing {phrase!r}, got rc={outcome.returncode}\n"
            f"stdout={outcome.stdout}\nstderr={outcome.stderr}"
        )


def main() -> int:
    if run(LEDGER, "--validate").returncode != 0:
        raise AssertionError("real delivery inventory fails validation")
    with tempfile.TemporaryDirectory() as directory:
        original, path = fixture(Path(directory))
        for milestone in ("H0", "A0"):
            outcome = run(path, "--close-milestone", milestone, "--release", REVISION)
            if outcome.returncode != 0:
                raise AssertionError(f"fully evidenced {milestone} fixture failed:\n{outcome.stderr}")
        print("  valid source-bound H0/A0 fixture closes; future rows remain pending")

        missing = copy.deepcopy(original)
        missing["requirements"] = [r for r in missing["requirements"] if r["id"] != "H0-01"]
        check(path, missing, "missing, duplicated or added requirement rows", "--validate")
        print("  missing required row rejected")
        cycle = copy.deepcopy(original)
        next(m for m in cycle["milestones"] if m["id"] == "H0")["common_prerequisites"].append("A0")
        check(path, cycle, "cyclic milestone prerequisites", "--validate")
        print("  cyclic prerequisite rejected")

        fake = copy.deepcopy(original)
        row(fake, "A0-01")["evidence_records"] = []
        check(path, fake, "no passing source-bound runner report", "--close-milestone", "A0", "--release", REVISION)
        print("  fake verified row without a runner report rejected")

        skipped = copy.deepcopy(original)
        target = row(skipped, "B0-02")
        target["status"] = "verified"
        target["lane_status"] = {lane: "blocked" for lane in target["required_platform_capability_lanes"]}
        target["evidence_records"] = [record(target, target["required_platform_capability_lanes"][0])]
        check(path, skipped, "missing or unresolved required lanes", "--validate")
        print("  blocked Mac lane rejected")

        zero = copy.deepcopy(original)
        g03 = row(zero, "G-03")["evidence_records"][0]
        g03["obligations"]["expected"] = g03["obligations"]["checked"] = 0
        check(path, zero, "zero executed proof obligations", "--close-milestone", "H0", "--release", REVISION)
        print("  zero proof obligations rejected")

        failed = copy.deepcopy(original)
        ev = row(failed, "A0-01")["evidence_records"][0]
        ev["result"] = "fail"
        ev["counts"] = {"executed": 1, "passed": 0, "failed": 1, "skipped": 0}
        check(path, failed, "failed/skipped required acceptance evidence", "--close-milestone", "A0", "--release", REVISION)
        print("  failed positive case rejected")

        mutant = copy.deepcopy(original)
        ev = row(mutant, "A0-01")["evidence_records"][0]
        ev.update(kind="mutant-calibration", intended_property="wrong target",
                  observed_rejection="compiler missing", tool_present=False)
        check(path, mutant, "missing tool", "--validate")
        print("  mutant failing for a missing tool rejected")

        stale = copy.deepcopy(original)
        row(stale, "A0-01")["evidence_records"][0]["commit"] = "0" * 40
        check(path, stale, "mismatched implementation", "--close-milestone", "A0", "--release", REVISION)
        print("  evidence from a different revision rejected")

        downgraded = copy.deepcopy(original)
        req = row(downgraded, "A0-01")
        req["status"] = "implemented_unverified"
        req.setdefault("status_history", []).append(
            {"from": "verified", "to": "implemented_unverified", "date": "2026-09-24"}
        )
        check(path, downgraded, "invalid_evidence detail", "--validate")
        print("  unexplained downgrade rejected")

        markdown = copy.deepcopy(original)
        ev = row(markdown, "A0-01")["evidence_records"][0]
        ev["kind"] = "review"
        ev["report"] = "plan/0049-handover-bundle-import.md"
        check(path, markdown, "no passing source-bound runner report", "--close-milestone", "A0", "--release", REVISION)
        print("  Markdown-only completion evidence rejected")

        missing_lane = copy.deepcopy(original)
        req = row(missing_lane, "A0-01")
        req["lane_status"].pop("pure")
        req["evidence_records"] = [ev for ev in req["evidence_records"] if ev["lane"] != "pure"]
        check(path, missing_lane, "missing or unresolved required lanes", "--validate")
        print("  unreported required lane rejected")

        altered = copy.deepcopy(original)
        ev = row(altered, "A0-01")["evidence_records"][0]
        ev["report_sha256"] = "0" * 64
        check(path, altered, "SHA-256 does not match actual bytes", "--validate")
        print("  changed runner report bytes rejected")
        inflated = copy.deepcopy(original)
        ev = row(inflated, "A0-01")["evidence_records"][0]
        ev["counts"]["executed"] = ev["counts"]["passed"] = 7
        check(path, inflated, "report_marker does not contain its claimed passed count", "--validate")
        print("  inflated counts absent from the runner report rejected")

        claimed = copy.deepcopy(original)
        next(m for m in claimed["milestones"] if m["id"] == "B0")["status"] = "verified"
        check(path, claimed, "--release <exact 40-hex", "--validate")
        print("  CI rejects a declared incomplete milestone")

        path.write_text(json.dumps(original))
        future = run(path, "--close-scope", "foundation", "--release", REVISION)
        if future.returncode == 0 or "requires this row verified" not in future.stderr:
            raise AssertionError("foundation scope closed despite pending B0/A1 rows")
        print("  pending future scope cannot close")

        # Evidence-only commit: source content is unchanged, so a
        # documented digest match may reuse the earlier runner report.
        tmp = Path(directory)
        (tmp / "plan").mkdir()
        (tmp / "plan/evidence.md").write_text("reviewed runner receipt\n")
        subprocess.run(["git", "-C", directory, "add", "plan/evidence.md"], check=True)
        subprocess.run(
            ["git", "-C", directory, "-c", "user.name=Checker Calibration",
             "-c", "user.email=checker@localhost", "commit", "-q",
             "-m", "archive evidence only"], check=True,
        )
        next_revision = subprocess.check_output(
            ["git", "-C", directory, "rev-parse", "HEAD"], text=True,
        ).strip()
        tested_tree = subprocess.check_output(
            ["git", "-C", directory, "ls-tree", "-r", "--full-tree",
             "-z", REVISION, "--", "scripts"],
        )
        reused = copy.deepcopy(original)
        for req in reused["requirements"]:
            for ev in req.get("evidence_records", []):
                ev["source_reuse_sha256"] = hashlib.sha256(tested_tree).hexdigest()
                ev["source_reuse_reason"] = "only the evidence plan changed; tracked source trees agree"
        path.write_text(json.dumps(reused))
        accepted = run(path, "--close-milestone", "A0", "--release", next_revision)
        if accepted.returncode != 0:
            raise AssertionError(f"identical-source reuse rejected:\n{accepted.stderr}")
        print("  identical-source evidence reuse across an evidence-only commit accepted")

        (tmp / "scripts/calibration.py").write_text("print('changed behavior')\n")
        subprocess.run(["git", "-C", directory, "add", "scripts/calibration.py"], check=True)
        subprocess.run(
            ["git", "-C", directory, "-c", "user.name=Checker Calibration",
             "-c", "user.email=checker@localhost", "commit", "-q",
             "-m", "change production source"], check=True,
        )
        changed_revision = subprocess.check_output(
            ["git", "-C", directory, "rev-parse", "HEAD"], text=True,
        ).strip()
        check(path, reused, "source trees differ", "--close-milestone", "A0", "--release", changed_revision)
        print("  changed-source reuse rejected despite an unchanged evidence receipt")
    print("delivery checker calibration: 15 negative cases rejected, valid fixtures accepted")
    return 0

if __name__ == "__main__":
    sys.exit(main())
