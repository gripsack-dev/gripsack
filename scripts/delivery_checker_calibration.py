#!/usr/bin/env python3
"""Optional delivery admission calibration; never implementation evidence.

Run only on explicit owner request. Synthetic practical receipts may close the
selected native scope, never retired products or the original edition-5 proof
inventory. Missing/wrong selected evidence and altered history must reject.
"""
from __future__ import annotations

import copy
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

from check_delivery import close_claim, validate
from delivery_evidence import Violations, coverage_violations, source_fingerprint
from delivery_scope import (
    BASELINE_SHA256, OPTIONAL_LANES, OPTIONAL_REQUIREMENTS, OUT_OF_SCOPE,
    PRODUCT_RETIRED_LANES, PRODUCT_RETIRED_REQUIREMENTS, RETIRED_LANES,
    RETIRED_MILESTONES, baseline_digest, effective_milestone, effective_requirement,
)

REPO = Path(__file__).resolve().parent.parent
LEDGER = REPO / "verification/delivery.json"
REPORT = "verification/reports/calibration.log"
MARKER = "synthetic calibration runner: 1 passed, 0 failed, 0 skipped"


def row(ledger: dict, identity: str) -> dict:
    return next(item for item in ledger["requirements"] if item["id"] == identity)


def record(req: dict, lane: str, revision: str, kind: str,
           milestone: str | None = None) -> dict:
    active = effective_requirement(req)
    if active is None:
        raise AssertionError("retired/optional rows cannot manufacture practical evidence")
    cases = active.get("case_and_proof_inventory_by_lane", {}).get(
        lane, active["case_and_proof_inventory"]
    )
    result = {
        "kind": kind, "lane": lane, "date": "2026-10-28",
        "entry_points": ["synthetic calibration fixture"],
        "environment": "isolated synthetic Linux checker fixture",
        "tool_versions": "calibration 1", "inputs": "synthetic selected cases",
        "command": "synthetic-calibration", "result": "pass",
        "report": REPORT,
        "report_sha256": hashlib.sha256((MARKER + "\n").encode()).hexdigest(),
        "report_marker": MARKER, "commit": revision,
        "counts": {"executed": 1, "passed": 1, "failed": 0, "skipped": 0},
        "cases_covered": list(cases),
    }
    if milestone is not None:
        result["milestone"] = milestone
    return result


def qualify(req: dict, revision: str) -> None:
    active = effective_requirement(req)
    if active is None:
        raise AssertionError("only selected requirements can be qualified")
    if req["status"] != "verified":
        req.setdefault("status_history", []).append({
            "from": req["status"], "to": "verified", "date": "2026-10-28",
        })
    req["status"] = "verified"
    # Preserve historical lane states; only selected lanes are qualified here.
    req.setdefault("lane_status", {}).update({
        lane: "verified" for lane in active["required_platform_capability_lanes"]
    })
    req["evidence_records"] = [
        record(req, lane, revision, kind)
        for lane in active["required_platform_capability_lanes"]
        for kind in active["evidence_kinds"]
    ]


def admit(violations: Violations, label: str) -> None:
    if violations.items:
        raise AssertionError(f"{label} unexpectedly rejected:\n" + "\n".join(violations.items))


def reject(violations: Violations, phrase: str) -> None:
    if not any(phrase in item for item in violations.items):
        raise AssertionError(f"expected rejection {phrase!r}, got {violations.items!r}")


def git(root: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def commit(root: Path, path: str) -> str:
    git(root, "add", path)
    git(root, "-c", "user.name=Checker Calibration", "-c",
        "user.email=checker@localhost", "commit", "-q", "-m", "synthetic fixture change")
    return git(root, "rev-parse", "HEAD")


def fixture(tmp: Path) -> tuple[dict, str]:
    git(tmp, "init", "-q")
    (tmp / "scripts").mkdir()
    (tmp / "scripts/calibration.py").write_text("print('synthetic source')\n")
    revision = commit(tmp, "scripts/calibration.py")
    ledger = json.loads(LEDGER.read_text())
    # Historical report bytes must remain real even though this isolated copy
    # replaces selected receipts with synthetic ones. Never touch the live ledger.
    for req in ledger["requirements"]:
        for evidence in req.get("evidence_records", []):
            report = Path(evidence["report"])
            target = tmp / report
            target.parent.mkdir(parents=True, exist_ok=True)
            if not target.is_file():
                target.write_bytes((REPO / report).read_bytes())
    (tmp / REPORT).parent.mkdir(parents=True, exist_ok=True)
    (tmp / REPORT).write_text(MARKER + "\n")
    for req in ledger["requirements"]:
        active = effective_requirement(req)
        if active is None:
            continue
        if req["owner_milestone"] == "H0":
            qualify(req, revision)
        if req["owner_milestone"] == "global":
            req["evidence_records"] = [
                record(req, lane, revision, kind, milestone)
                for milestone in ("H0", "A1")
                for lane in active["required_platform_capability_lanes"]
                for kind in active["evidence_kinds"]
            ]
    return ledger, revision


def scope_calibration(original: dict, tmp: Path, revision: str) -> None:
    before = copy.deepcopy(original)
    excluded = OUT_OF_SCOPE | PRODUCT_RETIRED_REQUIREMENTS | OPTIONAL_REQUIREMENTS
    assert baseline_digest(original) == BASELINE_SHA256
    for req in original["requirements"]:
        active = effective_requirement(req)
        if req["id"] in excluded:
            assert active is None
            continue
        assert active is not None
        assert active["case_and_proof_inventory"]
        lanes = set(active["required_platform_capability_lanes"])
        assert lanes and not lanes & (RETIRED_LANES | PRODUCT_RETIRED_LANES | OPTIONAL_LANES)
        assert "formal" not in active["evidence_kinds"]
        records = [record(req, lane, revision, kind)
                   for lane in lanes for kind in active["evidence_kinds"]]
        coverage = Violations()
        coverage_violations(active, records, coverage, lanes)
        admit(coverage, req["id"])
        # Every retained conjunct is independently necessary, without pinning
        # incidental prose, source text, function spelling or implementation wiring.
        for case in active["case_and_proof_inventory"]:
            missing = copy.deepcopy(records)
            for ev in missing:
                ev["cases_covered"] = [value for value in ev["cases_covered"] if value != case]
            coverage = Violations()
            coverage_violations(active, missing, coverage, lanes)
            reject(coverage, "conjunctive cases without passing evidence")
        for lane in lanes:
            coverage = Violations()
            coverage_violations(active, [ev for ev in records if ev["lane"] != lane], coverage, lanes)
            reject(coverage, "no passing")
        for kind in active["evidence_kinds"]:
            coverage = Violations()
            coverage_violations(active, [ev for ev in records if ev["kind"] != kind], coverage, lanes)
            reject(coverage, "no passing")
    assert original == before, "projection mutated historical inventory or evidence"

    # OCI Distribution artifacts are not image production. Its full original
    # native transport cases remain required, including private registry clients.
    for identity in ("D3-03", "D3-04"):
        req = row(original, identity)
        active = effective_requirement(req)
        assert active["case_and_proof_inventory"] == req["case_and_proof_inventory"]
        assert "private-registry" in active["required_platform_capability_lanes"]

    # Timing campaign exclusion must not remove ordinary systemd lifecycle
    # admission. E5 qualifies without platform-time, not without manager evidence.
    for identity in ("A1-01", "A3-03", "C4-05", "D3-03", "E2-01", "E2-02",
                     "E3-02", "E3-04", "E4-04", "E5-01", "E5-03"):
        selected = copy.deepcopy(original)
        req = row(selected, identity)
        qualify(req, revision)
        admit(validate(selected, tmp / "verification"), f"selected {identity}")
        req["evidence_records"][0]["cases_covered"] = []
        # Other kinds can cover the same case, so remove that lane entirely to
        # prove integration rejects borrowed evidence from a different lane.
        lane = req["evidence_records"][0]["lane"]
        req["evidence_records"] = [ev for ev in req["evidence_records"] if ev["lane"] != lane]
        reject(validate(selected, tmp / "verification"), "no passing")

    # Original formal inventory has not been satisfied by practical receipts.
    req = row(original, "A1-01")
    active = effective_requirement(req)
    records = [record(req, lane, revision, kind)
               for lane in active["required_platform_capability_lanes"]
               for kind in active["evidence_kinds"]]
    full = Violations()
    coverage_violations(req, records, full, set(req["required_platform_capability_lanes"]))
    reject(full, "no passing formal evidence")

    for milestone in RETIRED_MILESTONES:
        reject(close_claim(original, milestone, None, revision, tmp / "verification"),
               "cannot be claimed as qualified")
    # There is no all-retired scope in the original inventory. A forged one
    # must reject both its baseline change and its attempted empty qualification.
    empty = copy.deepcopy(original)
    empty["closure_scopes"]["foundation"] = ["A0", "B0"]
    reject(validate(empty, tmp / "verification"), "original lane/case/evidence-kind/prerequisite inventory changed")
    native = copy.deepcopy(original)
    for req in native["requirements"]:
        if req["owner_milestone"] == "A1":
            qualify(req, revision)
    admit(close_claim(native, "A1", None, revision, tmp / "verification"), "native A1 without Brew/formal")
    assert effective_milestone(next(m for m in original["milestones"] if m["id"] == "A1"))["common_prerequisites"] == ["H0"]
    assert row(native, "A0-01")["status"] != "verified"
    row(native, "H0-01")["evidence_records"] = []
    reject(close_claim(native, "A1", None, revision, tmp / "verification"), "no passing")


def negative_calibration(original: dict, tmp: Path, revision: str) -> None:
    def inventory(mutated: dict, phrase: str) -> None:
        reject(validate(mutated, tmp / "verification"), phrase)

    for identity, field in (("B0-01", "required_platform_capability_lanes"),
                            ("E2-02", "case_and_proof_inventory"),
                            ("A1-01", "evidence_kinds")):
        changed = copy.deepcopy(original)
        row(changed, identity)[field].pop(0)
        inventory(changed, "original lane/case/evidence-kind/prerequisite inventory changed")
    missing = copy.deepcopy(original)
    missing["requirements"] = [r for r in missing["requirements"] if r["id"] != "H0-01"]
    inventory(missing, "missing, duplicated or added requirement rows")
    changed = copy.deepcopy(original)
    row(changed, "B0-01")["mandatory_deliverable"] += " waived"
    inventory(changed, "protected requirement inventory changed")
    for index in range(len(original["scope_amendments"])):
        forged = copy.deepcopy(original)
        forged["scope_amendments"][index]["id"] = "unapproved"
        inventory(forged, "missing or unapproved scope amendment")
    forged = copy.deepcopy(original)
    forged.pop("owner_additions")
    inventory(forged, "missing or unapproved separate services owner addition")
    forged = copy.deepcopy(original)
    forged["owner_additions"][0]["status"] = "verified"
    inventory(forged, "missing or unapproved separate services owner addition")
    for identity in sorted(PRODUCT_RETIRED_REQUIREMENTS - {"B0-01"}):
        forged = copy.deepcopy(original)
        row(forged, identity)["status"] = "verified"
        inventory(forged, "retired product")
    for identity in sorted(OUT_OF_SCOPE):
        forged = copy.deepcopy(original)
        row(forged, identity)["status"] = "verified"
        inventory(forged, "cannot count as verified")

    selected = copy.deepcopy(original)
    qualify(row(selected, "E0-02"), revision)
    row(selected, "E0-02")["evidence_records"][0]["cases_covered"] = row(selected, "E0-02")["case_and_proof_inventory_by_lane"]["launchd"]
    inventory(selected, "systemd-linux: conjunctive cases without passing evidence")
    for field, value, phrase in (
        ("report_sha256", "0" * 64, "SHA-256 does not match actual bytes"),
        ("report_marker", "invented report", "report_marker"),
        ("counts", {"executed": 7, "passed": 7, "failed": 0, "skipped": 0}, "claimed passed count"),
    ):
        changed = copy.deepcopy(original)
        row(changed, "H0-01")["evidence_records"][0][field] = value
        inventory(changed, phrase)
    for kind in ("runner", "review"):
        changed = copy.deepcopy(original)
        req = row(changed, "G-01")
        req["evidence_records"] = [ev for ev in req["evidence_records"]
                                   if not (ev["kind"] == kind and ev["milestone"] == "H0")]
        reject(close_claim(changed, "H0", None, revision, tmp / "verification"), "no passing")
    stale = copy.deepcopy(original)
    row(stale, "H0-01")["evidence_records"][0]["commit"] = "0" * 40
    reject(close_claim(stale, "H0", None, revision, tmp / "verification"), "mismatched implementation")
    failed = copy.deepcopy(original)
    ev = row(failed, "H0-01")["evidence_records"][0]
    ev["result"] = "fail"
    ev["counts"] = {"executed": 1, "passed": 0, "failed": 1, "skipped": 0}
    reject(close_claim(failed, "H0", None, revision, tmp / "verification"), "failed/skipped required acceptance evidence")
    unverified = copy.deepcopy(original)
    req = row(unverified, "H0-01")
    req["status"] = "implemented_unverified"
    req["status_history"].append({"from": "verified", "to": "implemented_unverified", "date": "2026-10-28"})
    inventory(unverified, "invalid_evidence detail")
    reject(close_claim(original, None, "foundation", revision, tmp / "verification"), "requires this row verified")


def source_calibration(original: dict, tmp: Path, revision: str) -> None:
    fingerprint = source_fingerprint(tmp, revision)
    (tmp / "verification/evidence-note.txt").write_text("synthetic evidence-only change\n")
    next_revision = commit(tmp, "verification/evidence-note.txt")
    reused = copy.deepcopy(original)
    for req in reused["requirements"]:
        if effective_requirement(req) is not None:
            for ev in req.get("evidence_records", []):
                ev["source_reuse_sha256"] = fingerprint
                ev["source_reuse_reason"] = "only evidence changed; tracked implementation roots agree"
    admit(close_claim(reused, "H0", None, next_revision, tmp / "verification"), "unchanged-source reuse")
    for name in ("tools/conda-helper/calibration.rs", ".ci/pilot.json", "scripts/calibration.py"):
        source = tmp / name
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_text("changed behavior\n")
        changed_revision = commit(tmp, name)
        reject(close_claim(reused, "H0", None, changed_revision, tmp / "verification"), "source trees differ")
        if name != "scripts/calibration.py":
            source.unlink()
            commit(tmp, name)


def main() -> int:
    actual = json.loads(LEDGER.read_text())
    admit(validate(actual, LEDGER.parent), "actual source inventory")
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        original, revision = fixture(tmp)
        admit(close_claim(original, "H0", None, revision, tmp / "verification"), "synthetic practical H0")
        scope_calibration(original, tmp, revision)
        negative_calibration(original, tmp, revision)
        source_calibration(original, tmp, revision)
    print("delivery calibration: selected practical evidence accepted; retired claims, missing/wrong evidence and source/history forgeries rejected; optional assurance is not product completion")
    return 0


if __name__ == "__main__":
    sys.exit(main())
