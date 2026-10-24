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
from delivery_evidence import Violations, coverage_violations, proof_catalog_digest
from delivery_scope import (
    AMENDMENT_ID, OUT_OF_SCOPE, FUZZ_CASE_AMENDMENTS, FUZZ_RETAINED_CASES,
    FUZZ_LANE_REQUIREMENTS, X64_CASE_AMENDMENTS, effective_requirement,
)

REPO = Path(__file__).resolve().parent.parent
CHECKER = REPO / "scripts/check_delivery.py"
LEDGER = REPO / "verification/delivery.json"
REVISION = "a" * 40
REPORT = "verification/reports/calibration.log"
MARKER = "fixture runner: 1 passed, 0 failed, 0 skipped; proof: ProofFixture"


def run(path: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(CHECKER), "--ledger", str(path), *args],
        text=True, capture_output=True,
    )


def row(ledger: dict, identity: str) -> dict:
    return next(entry for entry in ledger["requirements"] if entry["id"] == identity)


def record(req: dict, lane: str, milestone: str | None = None, kind: str = "runner") -> dict:
    data = {
        "kind": kind,
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
        "cases_covered": (effective_requirement(req) or req)["case_and_proof_inventory"],
    }
    if milestone:
        data["milestone"] = milestone
    if kind == "formal":
        names = (req["proof_obligations_by_milestone"][milestone]["names"]
                 if req["owner_milestone"] == "global" else req["proof_obligation_inventory"])
        data["obligations"] = {"expected": 1, "checked": 1, "failed": 0,
                               "skipped": 0, "proof_ids": names,
                               "catalog_sha256": proof_catalog_digest(req, milestone)}
    return data


def fixture(tmp: Path) -> tuple[dict, Path]:
    global REVISION, MARKER
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
    ledger = json.loads(LEDGER.read_text())
    # This scratch ledger includes real in-progress rows with source-bound
    # evidence. Preserve their report bytes exactly: calibration must
    # isolate H0/A0 closure, not fail because its temp repo lost A1 logs.
    for requirement in ledger["requirements"]:
        for evidence in requirement.get("evidence_records", []):
            report = Path(evidence["report"])
            target = tmp / report
            if target.is_file():
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((REPO / report).read_bytes())
    # Preserve the exact original inventories and all required evidence kinds.
    # Only missing proof catalogs get synthetic names in this checker fixture;
    # this supplies no real implementation evidence.
    for req in ledger["requirements"]:
        if "formal" in req["evidence_kinds"] and req["owner_milestone"] != "global":
            req.setdefault("proof_obligation_inventory", [f"ProofFixture{req['id']}"])
            req.setdefault("proof_expected_minimum", 1)
    # H0 and A0 have distinct catalogs; neither can borrow the other's runner.
    proof = row(ledger, "G-03")
    proof["proof_obligations_by_milestone"] = {
        item["id"]: {
            "names": [f"ProofFixture{item['id']}"],
            "minimum": 1,
        }
        for item in ledger["milestones"]
    }
    MARKER = (
        "fixture runner: 1 passed, 0 failed, 0 skipped; proof_checked=1; "
        + "; ".join(
            f"proof: {entry['names'][0]}; catalog: {proof_catalog_digest(proof, mid)}"
            for mid, entry in proof["proof_obligations_by_milestone"].items()
        )
    )
    (tmp / REPORT).write_text(MARKER + "\n")
    for identity in ("H0-01", "H0-02", "A0-01"):
        req = row(ledger, identity)
        previous = req["status"]
        if previous != "verified":
            req.setdefault("status_history", []).append(
                {"from": previous, "to": "verified", "date": "2026-09-24"}
            )
        req["status"] = "verified"
        req["lane_status"] = {lane: "verified" for lane in req["required_platform_capability_lanes"]}
        req["evidence_records"] = [
            record(req, lane, kind=kind)
            for lane in req["lane_status"] for kind in req["evidence_kinds"]
        ]
    for req in ledger["requirements"]:
        if req["id"] in ledger["global_gate_ids"]:
            req["evidence_records"] = [
                record(req, lane, milestone, kind)
                for milestone in ("H0", "A0")
                for lane in req["required_platform_capability_lanes"]
                for kind in req["evidence_kinds"]
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


def scope_fixture(ledger: dict, path: Path, identity: str) -> dict:
    """Synthetic passing applicable evidence, never a real qualification."""
    result = copy.deepcopy(ledger)
    req = row(result, identity)
    active = effective_requirement(req)
    if req["status"] != "verified":
        req.setdefault("status_history", []).append(
            {"from": req["status"], "to": "verified", "date": "2026-10-23"}
        )
    req["status"] = "verified"
    req["lane_status"] = {lane: "verified" for lane in active["required_platform_capability_lanes"]}
    req["evidence_records"] = [
        record(req, lane, kind=kind)
        for lane in req["lane_status"] for kind in req["evidence_kinds"]
    ]
    if "formal" in req["evidence_kinds"]:
        floor = req["proof_expected_minimum"]
        marker = (
            f"fixture runner: 1 passed, 0 failed, 0 skipped; proof_checked={floor}; "
            + "; ".join(req["proof_obligation_inventory"])
            + f"; catalog: {proof_catalog_digest(req)}"
        )
        report = f"verification/reports/scope-{identity}.log"
        data = (marker + "\n").encode()
        (path.parent.parent / report).write_bytes(data)
        for ev in req["evidence_records"]:
            if ev["kind"] == "formal":
                ev.update(report=report, report_marker=marker,
                          report_sha256=hashlib.sha256(data).hexdigest())
                ev["obligations"]["expected"] = ev["obligations"]["checked"] = floor
    path.write_text(json.dumps(result))
    accepted = run(path, "--validate")
    if accepted.returncode:
        raise AssertionError(f"projected {identity} fixture rejected:\n{accepted.stderr}")
    return result


def scope_calibration(original: dict, path: Path) -> None:
    # Exercise evidence admission, not just matching policy strings. All twelve
    # reviewed rows plus standalone fuzz lanes retain their non-fuzz obligations.
    retained_clauses = {
        "B6-03": ("native execution", "unsupported host/target/toolchain"),
        "A1-07": ("including fuzz and future non-crates members", "schema/parser acceptance"),
        "A1-08": ("full hostile context/edge corpus", "production-kernel phase-grant mutant"),
        "A3-03": ("crossprocess positive/negative wire", "length and identity admission"),
        "A4-01": ("collision/link/long-prefix", "Verus compatibility/closure mutations",
                  "Apple Silicon incompatible-tag counterexamples", "unsupported architectures"),
        "B1-03": ("Go race fixtures", "real Linux worker paths"),
        "B2-02": ("actual decoder limits", "real Rust/Go/worker fixtures"),
        "B5-04": ("Go race", "formal", "protected CI evidence"),
        "E5-03": ("proof obligations are nonempty", "semantic mutants"),
        "C3-02": ("bounded regression fixtures", "foreign drift"),
        "C5-02": ("shrinking graph oracles", "silent truncation mutants", "TLC/TLAPS"),
        "D2-05": ("bounded adversarial archive and registry descriptors", "actual production path"),
        "D6-03": ("Verus/TLC/TLAPS", "semantic mutants", "with attribution"),
    }
    identities = (set(FUZZ_CASE_AMENDMENTS) | set(FUZZ_RETAINED_CASES)
                  | FUZZ_LANE_REQUIREMENTS | {"B6-03"})
    for identity in sorted(identities):
        qualified = scope_fixture(original, path, identity)
        req = row(qualified, identity)
        active = effective_requirement(req)
        original_req = row(original, identity)
        if identity in FUZZ_CASE_AMENDMENTS or identity == "B6-03":
            # Scoped receipts do not assert satisfaction of the original
            # cross-architecture/fuzz clauses, even on the same retained lanes.
            violations = Violations()
            coverage_violations(original_req, req["evidence_records"], violations,
                                set(active["required_platform_capability_lanes"]))
            if not any("conjunctive cases without passing evidence" in item
                       for item in violations.items):
                raise AssertionError(f"{identity}: projection falsely satisfied original acceptance")
            changes = (X64_CASE_AMENDMENTS if identity == "B6-03" else FUZZ_CASE_AMENDMENTS)
            required_cases = [case for case in changes[identity].values() if case is not None]
        elif identity in FUZZ_RETAINED_CASES:
            required_cases = [original_req["case_and_proof_inventory"][index]
                              for index in FUZZ_RETAINED_CASES[identity]]
        else:
            required_cases = active["case_and_proof_inventory"]
        for case in required_cases:
            missing = copy.deepcopy(qualified)
            for ev in row(missing, identity)["evidence_records"]:
                ev["cases_covered"] = [value for value in ev["cases_covered"] if value != case]
            check(path, missing, "conjunctive cases without passing evidence", "--validate")
        # Independent semantic witnesses catch accidental weakening of the
        # projection itself: if a retained clause vanishes, deleting its
        # evidence is a no-op and this expected rejection fails.
        for clause in retained_clauses.get(identity, ()):
            missing = copy.deepcopy(qualified)
            for ev in row(missing, identity)["evidence_records"]:
                ev["cases_covered"] = [case for case in ev["cases_covered"] if clause not in case]
            check(path, missing, "conjunctive cases without passing evidence", "--validate")
        excluded_lane = ("cross-target" if identity == "B6-03"
                         else "fuzz" if identity in FUZZ_LANE_REQUIREMENTS else None)
        if excluded_lane is not None:
            forged = copy.deepcopy(qualified)
            row(forged, identity)["lane_status"][excluded_lane] = "verified"
            check(path, forged, "not verified", "--validate")
        # Formal/model lanes and Go race cannot be omitted with fuzz.
        for lane in active["required_platform_capability_lanes"]:
            missing = copy.deepcopy(qualified)
            row(missing, identity)["evidence_records"] = [
                ev for ev in req["evidence_records"] if ev["lane"] != lane
            ]
            check(path, missing, "no passing source-bound", "--validate")
        if "formal" in req["evidence_kinds"]:
            missing = copy.deepcopy(qualified)
            row(missing, identity)["evidence_records"] = [
                ev for ev in req["evidence_records"] if ev["kind"] != "formal"
            ]
            check(path, missing, "no passing formal evidence", "--validate")

    for amendment_index, field, value in (
        (1, "retired_requirement_lanes", {"B6-03": ["cross-target", "linux-amd64"]}),
        (2, "excluded_requirement_lanes", {"B1-03": ["protocol-fuzz-race"]}),
        (2, "disposition", "verified"),
        (2, "reviewed_without_exclusion", {}),
    ):
        forged = copy.deepcopy(original)
        forged["scope_amendments"][amendment_index][field] = value
        check(path, forged, "missing or unapproved scope amendment", "--validate")
    for identity in dict.fromkeys(("B6-03", *FUZZ_CASE_AMENDMENTS, *FUZZ_RETAINED_CASES)):
        changed = copy.deepcopy(original)
        cases = row(changed, identity)["case_and_proof_inventory"]
        cases[0], cases[1] = cases[1], cases[0]
        check(path, changed, "original lane/case/evidence-kind/prerequisite inventory changed", "--validate")
        changed = copy.deepcopy(original)
        row(changed, identity)["mandatory_deliverable"] += " (waived)"
        check(path, changed, "protected requirement inventory changed", "--validate")
    forged = copy.deepcopy(original)
    forged["scope_amendments"][2]["case_projection"]["A3-03"][0]["original_index"] = 2
    check(path, forged, "missing or unapproved scope amendment", "--validate")
    forged = copy.deepcopy(original)
    forged["scope_amendments"][2]["case_projection"]["B1-03"][0]["required_case"] = "Go race waived"
    check(path, forged, "missing or unapproved scope amendment", "--validate")
    print("  fixed x64/fuzz projection admits scoped evidence, not original fuzz/cross results; every retained mixed case/lane/proof remains mandatory")


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
        scope_calibration(original, path)

        # E0-01 is a review-only inventory contract. Its actual source-
        # bound review may be sufficient without inventing a runner.
        review_only = copy.deepcopy(original)
        reviewed = row(review_only, "E0-01")
        previous = reviewed["status"]
        reviewed["status"] = "verified"
        reviewed["evidence_kinds"] = ["review"]
        reviewed.setdefault("status_history", []).append(
            {"from": previous, "to": "verified", "date": "2026-09-24"}
        )
        reviewed["lane_status"] = {
            lane: "verified" for lane in reviewed["required_platform_capability_lanes"]
        }
        reviewed["evidence_records"] = [
            record(reviewed, lane, kind="review") for lane in reviewed["lane_status"]
        ]
        path.write_text(json.dumps(review_only))
        accepted = run(path, "--validate")
        if accepted.returncode:
            raise AssertionError(f"review-only verified row rejected:\n{accepted.stderr}")
        print("  review-only source-bound row admits its named cases without a fake runner")

        missing = copy.deepcopy(original)
        missing["requirements"] = [r for r in missing["requirements"] if r["id"] != "H0-01"]
        check(path, missing, "missing, duplicated or added requirement rows", "--validate")
        print("  missing required row rejected")
        cycle = copy.deepcopy(original)
        next(m for m in cycle["milestones"] if m["id"] == "H0")["common_prerequisites"].append("A0")
        check(path, cycle, "cyclic milestone prerequisites", "--validate")
        print("  cyclic prerequisite rejected")

        unknown_kind = copy.deepcopy(original)
        row(unknown_kind, "A0-01")["evidence_kinds"] = ["runner", "not-a-proof"]
        check(path, unknown_kind, "evidence_kinds must name unique supported", "--validate")
        print("  undeclared evidence-kind vocabulary rejected")

        fake = copy.deepcopy(original)
        row(fake, "A0-01")["evidence_records"] = []
        check(path, fake, "no passing source-bound runner report", "--close-milestone", "A0", "--release", REVISION)
        print("  fake verified row without a runner report rejected")

        for identity in sorted(OUT_OF_SCOPE):
            skipped = copy.deepcopy(original)
            target = row(skipped, identity)
            target["status"] = "verified"
            target["lane_status"] = {lane: "verified" for lane in target["required_platform_capability_lanes"]}
            target["evidence_records"] = [record(target, lane) for lane in target["lane_status"]]
            check(path, skipped, f"out_of_scope under {AMENDMENT_ID} cannot count as verified", "--validate")
        print("  every wholly Mac row remains out of scope, never verified even with fabricated passing receipts")

        forged = copy.deepcopy(original)
        forged["scope_amendments"][0]["id"] = "PLATFORM-ANYTHING-APPROVED"
        check(path, forged, "missing or unapproved scope amendment", "--validate")
        forged = copy.deepcopy(original)
        forged["scope_amendments"][0]["out_of_scope_requirements"]["B0-01"] = "out_of_scope"
        check(path, forged, "missing or unapproved scope amendment", "--validate")
        forged = copy.deepcopy(original)
        forged["scope_amendments"].append(copy.deepcopy(forged["scope_amendments"][0]))
        check(path, forged, "missing or unapproved scope amendment", "--validate")
        absent = copy.deepcopy(original)
        absent.pop("scope_amendments")
        check(path, absent, "missing or unapproved scope amendment", "--validate")
        print("  forged, expanded and duplicate owner amendments rejected")

        for identity, field in (
            ("B0-01", "required_platform_capability_lanes"),
            ("E2-02", "case_and_proof_inventory"),
            ("A1-01", "evidence_kinds"),
        ):
            dropped = copy.deepcopy(original)
            row(dropped, identity)[field].pop(0)
            check(path, dropped, "original lane/case/evidence-kind/prerequisite inventory changed", "--validate")
        dropped = copy.deepcopy(original)
        next(m for m in dropped["milestones"] if m["id"] == "B1")["lane_specific_prerequisites"] = []
        check(path, dropped, "original lane/case/evidence-kind/prerequisite inventory changed", "--validate")
        empty = copy.deepcopy(original)
        empty["closure_scopes"]["foundation"] = []
        check(path, empty, "original lane/case/evidence-kind/prerequisite inventory changed", "--validate")
        print("  dropped Linux lanes, cases, proof kinds, worker prerequisites and empty scopes rejected")

        missing_case_lane = copy.deepcopy(original)
        row(missing_case_lane, "E0-02")["case_and_proof_inventory_by_lane"].pop("launchd")
        check(path, missing_case_lane, "per-lane case inventory must name exactly the declared lanes",
              "--validate")
        print("  Mac launchd case lane cannot be silently unregistered")

        dropped_mac_case = copy.deepcopy(original)
        row(dropped_mac_case, "E0-02")["case_and_proof_inventory_by_lane"]["launchd"].pop()
        check(path, dropped_mac_case, "per-lane case inventories must account for every registered case",
              "--validate")
        print("  named Mac acceptance cannot disappear from all lanes")

        qualified_os_lanes = copy.deepcopy(original)
        e0 = row(qualified_os_lanes, "E0-02")
        e0["status_history"].append(
            {"from": "in_progress", "to": "verified", "date": "2026-09-26"}
        )
        e0["status"] = "verified"
        # Preserve launchd's historical blocked lane; only Linux is evidenced.
        e0["lane_status"]["systemd-linux"] = "verified"
        e0["evidence_records"] = [record(e0, "systemd-linux")]
        e0["evidence_records"][0]["cases_covered"] = e0["case_and_proof_inventory_by_lane"]["systemd-linux"]
        path.write_text(json.dumps(qualified_os_lanes))
        good_os_lanes = run(path, "--validate")
        if good_os_lanes.returncode:
            raise AssertionError(f"active Linux manager lane should pass:\n{good_os_lanes.stderr}")
        print("  Linux manager qualifies without changing historical blocked launchd evidence")

        borrowed_mac_case = copy.deepcopy(qualified_os_lanes)
        e0 = row(borrowed_mac_case, "E0-02")
        e0["evidence_records"][0]["cases_covered"] = e0["case_and_proof_inventory_by_lane"]["launchd"]
        check(path, borrowed_mac_case, "systemd-linux: conjunctive cases without passing evidence", "--validate")
        print("  retired launchd cases cannot replace active Linux manager evidence")

        # Mac parser/preview clauses in common lanes are explicitly retired;
        # the by-lane systemd byte/admission inventory remains conjunctive.
        common = copy.deepcopy(original)
        e2 = row(common, "E2-02")
        active = effective_requirement(e2)
        e2.setdefault("status_history", []).append({"from": e2["status"], "to": "verified", "date": "2026-10-08"})
        e2["status"] = "verified"
        e2["lane_status"] = {lane: "verified" for lane in active["required_platform_capability_lanes"]}
        e2["evidence_records"] = [record(e2, lane) for lane in e2["lane_status"]]
        for ev in e2["evidence_records"]:
            ev["cases_covered"] = active["case_and_proof_inventory_by_lane"][ev["lane"]]
        path.write_text(json.dumps(common))
        accepted = run(path, "--validate")
        if accepted.returncode:
            raise AssertionError(f"Linux/common byte translation fixture failed:\n{accepted.stderr}")
        dropped_common = copy.deepcopy(common)
        ev = next(ev for ev in row(dropped_common, "E2-02")["evidence_records"] if ev["lane"] == "pure")
        ev["cases_covered"] = ev["cases_covered"][1:]
        check(path, dropped_common, "pure: conjunctive cases without passing evidence", "--validate")
        print("  common-lane Mac portions retired explicitly; Linux decoder cases cannot disappear")

        # Full B0 closure still requires H0/global gates and every Linux/common
        # B0 row, but never asks for B0-02 or stale retired-lane source identity.
        linux_b0 = copy.deepcopy(original)
        for req in linux_b0["requirements"]:
            if req["owner_milestone"] != "B0" or req["id"] in OUT_OF_SCOPE:
                continue
            active = effective_requirement(req)
            if req["status"] != "verified":
                req.setdefault("status_history", []).append({"from": req["status"], "to": "verified", "date": "2026-10-08"})
            req["status"] = "verified"
            req.setdefault("lane_status", {}).update({lane: "verified" for lane in active["required_platform_capability_lanes"]})
            req["evidence_records"] = [
                record(req, lane, kind=kind)
                for lane in active["required_platform_capability_lanes"] for kind in req["evidence_kinds"]
            ]
            if req["id"] == "B0-03":
                historical_mac = record(req, "macos")
                historical_mac["commit"] = "0" * 40
                req["evidence_records"].append(historical_mac)
        for req in linux_b0["requirements"]:
            if req["id"] in linux_b0["global_gate_ids"]:
                req["evidence_records"].extend(
                    record(req, lane, "B0", kind)
                    for lane in req["required_platform_capability_lanes"] for kind in req["evidence_kinds"]
                )
        path.write_text(json.dumps(linux_b0))
        accepted = run(path, "--close-milestone", "B0", "--release", REVISION)
        if accepted.returncode:
            raise AssertionError(f"Linux B0 closure should not require retired Mac qualification:\n{accepted.stderr}")
        missing_linux = copy.deepcopy(linux_b0)
        row(missing_linux, "B0-01")["evidence_records"] = []
        check(path, missing_linux, "no passing source-bound runner report", "--close-milestone", "B0", "--release", REVISION)
        print("  B0 closure excludes retired Mac prerequisites, not Linux qualification")

        zero = copy.deepcopy(original)
        g03 = next(ev for ev in row(zero, "G-03")["evidence_records"]
                   if ev["kind"] == "formal" and ev["milestone"] == "H0")
        g03["obligations"]["expected"] = g03["obligations"]["checked"] = 0
        check(path, zero, "zero executed proof obligations", "--close-milestone", "H0", "--release", REVISION)
        print("  zero proof obligations rejected")

        runner_is_not_proof = copy.deepcopy(original)
        g03 = row(runner_is_not_proof, "G-03")
        g03["evidence_records"] = [
            ev for ev in g03["evidence_records"]
            if not (ev["milestone"] == "H0" and ev["kind"] == "formal")
        ]
        check(path, runner_is_not_proof, "no passing formal evidence",
              "--close-milestone", "H0", "--release", REVISION)
        print("  proof row with runner and review but no formal evidence rejected")

        proof_kind_swap = copy.deepcopy(original)
        g03_formal = next(ev for ev in row(proof_kind_swap, "G-03")["evidence_records"]
                          if ev["milestone"] == "H0" and ev["kind"] == "formal")
        g03_formal["kind"] = "runner"
        check(path, proof_kind_swap, "no passing formal evidence",
              "--close-milestone", "H0", "--release", REVISION)
        print("  runner carrying self-reported obligations cannot impersonate formal evidence")

        unregistered_proof = copy.deepcopy(original)
        row(unregistered_proof, "G-03").pop("proof_obligations_by_milestone")
        check(path, unregistered_proof, "missing named proof obligation inventory",
              "--validate")
        print("  H0-02 cannot close before proof inventories are named")

        wrong_proof = copy.deepcopy(original)
        row(wrong_proof, "G-03")["proof_obligations_by_milestone"]["H0"]["names"] = ["GhostProof"]
        check(path, wrong_proof, "named proof obligations without formal evidence",
              "--close-milestone", "H0", "--release", REVISION)
        print("  wrong formal proof name cannot cover a frozen obligation")

        missing_review = copy.deepcopy(original)
        g03 = row(missing_review, "G-03")
        g03["evidence_records"] = [
            ev for ev in g03["evidence_records"]
            if not (ev["milestone"] == "H0" and ev["kind"] == "review")
        ]
        check(path, missing_review, "no passing review evidence",
              "--close-milestone", "H0", "--release", REVISION)
        print("  required review kind cannot be replaced by runner plus formal evidence")

        unreported_proof = copy.deepcopy(original)
        formal = next(ev for ev in row(unreported_proof, "G-03")["evidence_records"]
                      if ev["milestone"] == "H0" and ev["kind"] == "formal")
        formal["obligations"]["proof_ids"] = ["GhostProof"]
        check(path, unreported_proof, "absent from the actual report", "--validate")
        print("  invented proof ID absent from real report bytes rejected")

        stale_catalog = copy.deepcopy(original)
        formal = next(ev for ev in row(stale_catalog, "G-03")["evidence_records"]
                      if ev["milestone"] == "H0" and ev["kind"] == "formal")
        formal["obligations"]["catalog_sha256"] = "0" * 64
        check(path, stale_catalog, "formal proof catalog digest mismatch",
              "--close-milestone", "H0", "--release", REVISION)
        print("  formal evidence from a different declared proof catalog rejected")

        other_milestone = copy.deepcopy(original)
        g03 = row(other_milestone, "G-03")
        h0 = next(ev for ev in g03["evidence_records"]
                  if ev["milestone"] == "H0" and ev["kind"] == "formal")
        a0 = next(ev for ev in g03["evidence_records"]
                  if ev["milestone"] == "A0" and ev["kind"] == "formal")
        h0["obligations"] = copy.deepcopy(a0["obligations"])
        check(path, other_milestone, "formal proof catalog digest mismatch",
              "--close-milestone", "H0", "--release", REVISION)
        print("  A0 formal proof cannot replace H0's milestone-bound catalog")

        missing_future_catalog = copy.deepcopy(original)
        row(missing_future_catalog, "G-03")["proof_obligations_by_milestone"].pop("E3")
        check(path, missing_future_catalog, "G-03/E3: missing named proof obligation inventory",
              "--validate")
        print("  H0 inventory closure needs even future milestone proof catalogs")

        raised_floor = copy.deepcopy(original)
        row(raised_floor, "G-03")["proof_obligations_by_milestone"]["H0"]["minimum"] = 2
        check(path, raised_floor, "zero executed proof obligations",
              "--close-milestone", "H0", "--release", REVISION)
        print("  observed proof count below frozen expected minimum rejected")

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

        forged_proof_count = copy.deepcopy(original)
        formal = next(ev for ev in row(forged_proof_count, "G-03")["evidence_records"]
                      if ev["milestone"] == "H0" and ev["kind"] == "formal")
        formal["obligations"]["expected"] = formal["obligations"]["checked"] = 7
        check(path, forged_proof_count, "report_marker does not contain its claimed proof obligation count",
              "--close-milestone", "H0", "--release", REVISION)
        print("  proof count cannot be borrowed from digest digits or a runner pass count")

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

        # Each root changes independently relative to the accepted baseline.
        # In particular helper/bridge code and CI policy are behavior, not
        # evidence-only metadata. Keep the original receipt and digest intact.
        for source_path in (
            "tools/conda-helper/calibration.rs",
            "tools/buildkit-bridge/calibration.go",
            ".ci/pilot.json",
            "scripts/calibration.py",
        ):
            source = tmp / source_path
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("changed behavior\n")
            subprocess.run(["git", "-C", directory, "add", source_path], check=True)
            subprocess.run(
                ["git", "-C", directory, "-c", "user.name=Checker Calibration",
                 "-c", "user.email=checker@localhost", "commit", "-q",
                 "-m", f"change behavior in {source_path}"], check=True,
            )
            changed_revision = subprocess.check_output(
                ["git", "-C", directory, "rev-parse", "HEAD"], text=True,
            ).strip()
            check(path, reused, "source trees differ", "--close-milestone",
                  "A0", "--release", changed_revision)
            print(f"  changed-source reuse rejected for {source_path}")
            # Restore the baseline tree with an ordinary removal commit so
            # the next negative cannot pass merely due to a prior mutation.
            if source_path == "scripts/calibration.py":
                continue
            source.unlink()
            subprocess.run(["git", "-C", directory, "add", source_path], check=True)
            subprocess.run(
                ["git", "-C", directory, "-c", "user.name=Checker Calibration",
                 "-c", "user.email=checker@localhost", "commit", "-q",
                 "-m", f"restore baseline after {source_path}"], check=True,
            )
    print("delivery checker calibration: active Linux fixtures accepted; scope/evidence forgeries rejected")
    return 0

if __name__ == "__main__":
    sys.exit(main())
