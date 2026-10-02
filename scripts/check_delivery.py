#!/usr/bin/env python3
"""Validate the edition-5 delivery inventory and source-bound closure.

Inventory mode permits pending future rows but protects all 178 imported
requirements. Closure mode additionally requires every prerequisite and
global gate, every declared lane and case, real runner report bytes,
positive execution/proof counts and an exact source revision. A ledger
attests to evidence completeness; it does not prove the behavior of a
runner or a publisher.

    python3 scripts/check_delivery.py --validate
    python3 scripts/check_delivery.py --close-milestone A0 --release <40-hex-sha>
    python3 scripts/check_delivery.py --close-scope foundation --release <40-hex-sha>
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

from delivery_evidence import (
    COMMIT_RE,
    EVIDENCE_KINDS,
    SOURCE_ROOTS,
    STATUS_VALUES,
    Violations,
    coverage_violations,
    evidence_violations,
    history_violations,
    lane_violations,
    repo_root,
    proof_catalog_violations,
    proof_row,
    source_binding_violations,
    source_fingerprint,
)

# Bundle edition 5's 178 mandatory rows: id, owner, scope, source,
# deliverable and required evidence. An amendment changes this constant
# explicitly in review; status, evidence and lane registration do not.
INVENTORY_SHA256 = "7a7c717ddd7ae526efc952a07268345a0333c20abf2abef5a0fcc64f0093c699"


def load_ledger(path: Path) -> dict:
    ledger = json.loads(path.read_text())
    if ledger.get("format") != "gripsack-delivery-ledger" or ledger.get("format_version") != 2:
        raise SystemExit(f"{path}: expected gripsack-delivery-ledger format version 2")
    return ledger



def validate(ledger: dict, ledger_dir: Path) -> Violations:
    v = Violations()
    root = repo_root(ledger_dir)
    reqs = ledger["requirements"]
    by_id = {r["id"]: r for r in reqs}
    if len(reqs) != 178 or ledger.get("requirement_count") != len(reqs) or len(by_id) != len(reqs):
        v.add("ledger", "missing, duplicated or added requirement rows: edition 5 requires 178 unique IDs")
    protected = ("id", "owner_milestone", "closure_scope", "source_document",
                 "mandatory_deliverable", "required_acceptance_evidence")
    source = [
        {key: row[key] for key in protected}
        for row in sorted(reqs, key=lambda row: row["id"])
    ]
    fingerprint = hashlib.sha256(json.dumps(source, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if fingerprint != INVENTORY_SHA256:
        v.add("ledger", "protected requirement inventory changed: owner amendment and reviewed fingerprint update required")
    milestone_ids = {m["id"] for m in ledger.get("milestones", [])}
    if len(milestone_ids) != len(ledger.get("milestones", [])):
        v.add("ledger", "duplicate milestone IDs")
    prerequisites: dict[str, set[str]] = {}
    for milestone in ledger.get("milestones", []):
        mid = milestone["id"]
        required = milestone.get("required_delivery_ids", [])
        actual = {r["id"] for r in reqs if r["owner_milestone"] == mid}
        if milestone.get("status") not in STATUS_VALUES:
            v.add(mid, f"invalid milestone status {milestone.get('status')!r}")
        if len(required) != len(set(required)) or set(required) != actual:
            v.add(mid, f"required delivery IDs differ from registered owner rows: {sorted(actual ^ set(required))}")
        prereqs = milestone.get("common_prerequisites", []) + [
            item.get("milestone") if isinstance(item, dict) else item
            for item in milestone.get("lane_specific_prerequisites", [])
        ]
        prerequisites[mid] = set(prereqs)
        for prereq in prereqs:
            if prereq not in milestone_ids or prereq == mid:
                v.add(mid, f"unknown or self-referential prerequisite milestone {prereq!r}")
    remaining = set(prerequisites)
    while remaining:
        ready = {mid for mid in remaining if not (prerequisites[mid] & remaining)}
        if not ready:
            v.add("ledger", f"cyclic milestone prerequisites: {sorted(remaining)}")
            break
        remaining -= ready
    scope_names = set(ledger.get("closure_scopes", {}))
    scoped_milestones = [
        mid for mids in ledger.get("closure_scopes", {}).values() for mid in mids
    ]
    if set(scoped_milestones) != milestone_ids or len(scoped_milestones) != len(milestone_ids):
        v.add("ledger", "closure scopes must partition the registered milestones")
    global_ids = set(ledger.get("global_gate_ids", []))
    if len(global_ids) != 8 or {r["id"] for r in reqs if r["owner_milestone"] == "global"} != global_ids:
        v.add("ledger", "global gate inventory differs from the eight required IDs")
    for req in reqs:
        rid = req["id"]
        if req.get("owner_milestone") not in milestone_ids | {"global"}:
            v.add(rid, f"unknown owner_milestone {req.get('owner_milestone')!r}")
        if req.get("closure_scope") not in scope_names | {"all_scopes"}:
            v.add(rid, f"unknown closure_scope {req.get('closure_scope')!r}")
        if req.get("status") not in STATUS_VALUES:
            v.add(rid, f"invalid status {req.get('status')!r}")
        state = req.get("lane_inventory_state")
        if state in ("registered", "live_registration_pending_expansion"):
            if not req.get("required_platform_capability_lanes") or not req.get("case_and_proof_inventory"):
                v.add(rid, "registered lanes/cases must be nonempty")
        elif state != "imported_pending_live_registration":
            v.add(rid, f"unknown lane inventory state {state!r}")
        kinds = req.get("evidence_kinds")
        if (not isinstance(kinds, list) or not kinds
                or not all(isinstance(kind, str) and kind in EVIDENCE_KINDS for kind in kinds)
                or len(set(kinds)) != len(kinds)):
            v.add(rid, "evidence_kinds must name unique supported runner/formal/review kinds")
        records = req.get("evidence_records", [])
        declared = set(req.get("required_platform_capability_lanes") or [])
        by_lane = req.get("case_and_proof_inventory_by_lane")
        if by_lane is not None:
            if not isinstance(by_lane, dict) or set(by_lane) != declared:
                v.add(rid, "per-lane case inventory must name exactly the declared lanes")
            elif any(
                not isinstance(cases, list) or not cases
                or not all(isinstance(case, str) and case.strip() for case in cases)
                or len(set(cases)) != len(cases)
                for cases in by_lane.values()
            ):
                v.add(rid, "per-lane case inventories must contain unique named cases")
            elif {case for cases in by_lane.values() for case in cases} != set(
                req.get("case_and_proof_inventory") or []
            ):
                v.add(rid, "per-lane case inventories must account for every registered case")
        for idx, ev in enumerate(records):
            if ev.get("lane") not in declared:
                v.add(rid, f"evidence[{idx}]: undeclared platform/capability lane {ev.get('lane')!r}")
            if req["owner_milestone"] == "global" and ev.get("milestone") not in milestone_ids:
                v.add(rid, f"evidence[{idx}]: global gate needs a known milestone")
            evidence_violations(req, ev, idx, v, root)
        lane_violations(req, v)
        if req["status"] == "verified":
            if not records:
                v.add(rid, "verified without evidence records: a handwritten pass flag is not evidence")
            if req["owner_milestone"] == "global":
                for mid in sorted(milestone_ids):
                    selected = [ev for ev in records if ev.get("milestone") == mid]
                    coverage_violations(req, selected, v, declared, mid)
            else:
                coverage_violations(req, records, v, declared)
        history_violations(req, v)
    if by_id.get("H0-02", {}).get("status") == "verified":
        unresolved = [
            r["id"] for r in reqs
            if r.get("lane_inventory_state") != "registered"
            or not r.get("required_platform_capability_lanes")
            or not r.get("case_and_proof_inventory")
            or not r.get("evidence_kinds")
        ]
        if unresolved:
            v.add("H0-02", f"complete support/case/proof/evidence-kind inventory missing for {len(unresolved)} rows: {unresolved[:12]}")
        for req in reqs:
            if proof_row(req):
                if req["owner_milestone"] == "global":
                    for mid in sorted(milestone_ids):
                        proof_catalog_violations(req, v, mid)
                else:
                    proof_catalog_violations(req, v)
    return v



def close_claim(ledger: dict, milestone: str | None, scope: str | None, release: str | None, ledger_dir: Path) -> Violations:
    v = validate(ledger, ledger_dir)
    if not release or not COMMIT_RE.fullmatch(release):
        v.add("claim", "--release <exact 40-hex source revision> is required for closure")
    root = repo_root(ledger_dir)
    checkout = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=root,
        text=True, capture_output=True, check=False,
    )
    if checkout.returncode != 0 or release != checkout.stdout.strip():
        v.add("claim", f"claimed release {release!r} does not match the checkout's implementation revision")
    dirty = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=all", "--", *SOURCE_ROOTS],
        cwd=root, text=True, capture_output=True, check=False,
    )
    if dirty.returncode != 0 or dirty.stdout.strip():
        v.add("claim", "uncommitted source changes cannot be bound to a release")
    fingerprints: dict[str, str | None] = {}
    if release and COMMIT_RE.fullmatch(release):
        fingerprints[release] = source_fingerprint(root, release)
    if not fingerprints.get(release or ""):
        v.add("claim", "release has no tracked source tree to bind evidence to")
    milestones = {m["id"]: m for m in ledger["milestones"]}
    if milestone:
        if milestone not in milestones:
            raise SystemExit(f"unknown milestone {milestone!r}")
        claimed = {milestone}
        label = f"milestone {milestone}"
    elif scope:
        if scope not in ledger.get("closure_scopes", {}):
            raise SystemExit(f"unknown closure scope {scope!r}")
        claimed = set(ledger["closure_scopes"][scope])
        label = f"closure scope {scope}"
    else:
        raise SystemExit("closure claim needs --close-milestone or --close-scope")
    # Full milestone/scope closure includes all common and lane-specific
    # prerequisites; partial platform slices are not closure claims.
    pending = list(claimed)
    while pending:
        current = milestones[pending.pop()]
        prereqs = current.get("common_prerequisites", []) + [
            p.get("milestone") if isinstance(p, dict) else p
            for p in current.get("lane_specific_prerequisites", [])
        ]
        for prereq in prereqs:
            if prereq in milestones and prereq not in claimed:
                claimed.add(prereq)
                pending.append(prereq)
    verified_rows = 0
    for req in ledger["requirements"]:
        rid = req["id"]
        if req["owner_milestone"] == "global":
            if req.get("lane_inventory_state") != "registered":
                v.add(rid, f"{label} has unresolved global-gate inventory")
            for mid in sorted(claimed):
                records = [
                    ev for ev in req.get("evidence_records", [])
                    if ev.get("milestone") == mid
                ]
                if not records:
                    v.add(rid, f"{label}: no per-milestone attestation for {mid}")
                coverage_violations(
                    req, records, v,
                    set(req.get("required_platform_capability_lanes") or []),
                    mid,
                )
                for ev in records:
                    if ev.get("result") != "pass":
                        v.add(rid, f"{mid}: failed/skipped required global gate")
                    source_binding_violations(ev, release, root, fingerprints, v, f"{rid}/{mid}")
            continue
        if req["owner_milestone"] not in claimed:
            continue
        if req["status"] != "verified":
            v.add(rid, f"{label} requires this row verified; status={req['status']!r}")
            continue
        verified_rows += 1
        for ev in req.get("evidence_records", []):
            if ev.get("result") != "pass":
                v.add(rid, "failed/skipped required acceptance evidence")
            source_binding_violations(ev, release, root, fingerprints, v, rid)
    if verified_rows == 0:
        v.add(label, "claim cites no verified delivery rows — empty claims are not closure")
    return v


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ledger", default="verification/delivery.json", type=Path)
    parser.add_argument("--validate", action="store_true", help="inventory validation mode")
    parser.add_argument("--close-milestone")
    parser.add_argument("--close-scope")
    parser.add_argument("--release", help="40-hex sha a closure claim is bound to")
    args = parser.parse_args()

    ledger = load_ledger(args.ledger)
    if args.validate:
        v = validate(ledger, args.ledger.parent)
        # CI validates inventory on every PR and any declared completion
        # claim. Future pending scopes never block an unrelated release.
        for item in ledger["milestones"]:
            if item.get("status") == "verified":
                closure = close_claim(
                    ledger, item["id"], None, item.get("closure_revision"), args.ledger.parent
                )
                v.items.extend(closure.items)
        for scope, revision in ledger.get("closure_claims", {}).items():
            closure = close_claim(ledger, None, scope, revision, args.ledger.parent)
            v.items.extend(closure.items)
        return v.report("delivery inventory and declared closure claims")
    if args.close_milestone or args.close_scope:
        if args.close_milestone and args.close_scope:
            raise SystemExit("pick one: --close-milestone or --close-scope")
        v = close_claim(ledger, args.close_milestone, args.close_scope, args.release, args.ledger.parent)
        return v.report(f"delivery closure claim ({args.close_milestone or args.close_scope})")
    parser.error("choose --validate, --close-milestone or --close-scope")


if __name__ == "__main__":
    sys.exit(main())
