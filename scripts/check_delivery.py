#!/usr/bin/env python3
"""Delivery-ledger validation and closure checking (handover A1-07).

The ledger (`verification/delivery.json`) is bookkeeping over the six
handover specifications: it cannot prove its own truth, but incomplete
or inconsistent completion claims must fail CI instead of passing by
silence. This checker aggregates — behavior is still established by
the real gates, proofs and platform runs each row cites.

Modes:
  --validate                 inventory validation (every PR): unique
                             IDs, known milestones/scopes, lane/case
                             inventories, evidence schema, no silent
                             removal/downgrade. Pending future work is
                             legal; a row marked `verified` without
                             complete passing evidence is not.
  --close-milestone M        closure claim: every row owned by M plus
                             the global gates verified, lanes resolved,
                             cases covered, commits bound.
  --close-scope S            closure claim for a whole scope
                             (foundation, foundation_extensions,
                             tasks_schedules, semantic_change,
                             artifact_sharing).
  --release REV              bind a closure claim to an exact revision:
                             every cited evidence commit must match.

Exit 0 = claim stands; exit 1 = violations printed to stderr.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

STATUS_VALUES = {"pending", "in_progress", "implemented_unverified", "blocked", "failed", "verified"}
RESULTS = {"pass", "fail", "skipped", "blocked"}
EVIDENCE_FIELDS = ["lane", "date", "entry_points", "environment", "command", "result", "report", "commit", "cases_covered"]
PROOF_HINTS = ("proof", "tlc", "verus", "tlaps", "mutant", "obligation", "theorem", "invariant")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}( dirty=[0-9a-f]+)?$")
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")


class Violations:
    def __init__(self) -> None:
        self.items: list[str] = []

    def add(self, req: str, msg: str) -> None:
        self.items.append(f"{req}: {msg}")

    def report(self, summary: str) -> int:
        if self.items:
            print(f"FAIL {summary} ({len(self.items)} violation(s)):", file=sys.stderr)
            for item in self.items:
                print(f"  - {item}", file=sys.stderr)
            return 1
        print(f"ok: {summary}")
        return 0
def repo_root(directory: Path) -> Path:
    """Report paths are repo-root relative; walk up from the ledger."""
    for candidate in [directory, *directory.parents]:
        if (candidate / ".git").exists():
            return candidate
    return directory


def load_ledger(path: Path) -> dict:
    ledger = json.loads(path.read_text())
    if ledger.get("format") != "gripsack-delivery-ledger":
        raise SystemExit(f"{path}: not a gripsack-delivery-ledger (format={ledger.get('format')!r})")
    return ledger


def evidence_violations(req: dict, ev: dict, idx: int, v: Violations, ledger_dir: Path) -> None:
    rid = req["id"]
    where = f"evidence[{idx}]"
    for field in EVIDENCE_FIELDS:
        if field not in ev:
            v.add(rid, f"{where}: missing field {field!r}")
    if "entry_points" in ev and not (isinstance(ev["entry_points"], list) and ev["entry_points"]):
        v.add(rid, f"{where}: entry_points must be a nonempty list")
    for str_field in ("lane", "environment", "command", "report", "commit"):
        if str_field in ev and not (isinstance(ev[str_field], str) and ev[str_field].strip()):
            v.add(rid, f"{where}: {str_field} must be a nonempty string")
    if ev.get("date") and not DATE_RE.match(ev["date"]):
        v.add(rid, f"{where}: date must be YYYY-MM-DD")
    if ev.get("commit") and not COMMIT_RE.match(ev["commit"]):
        v.add(rid, f"{where}: commit must be a 40-hex sha (optional ' dirty=<sha>' suffix)")
    if ev.get("result") not in RESULTS:
        v.add(rid, f"{where}: result must be one of {sorted(RESULTS)}, got {ev.get('result')!r}")
    report = ev.get("report", "")
    if isinstance(report, str) and report and "://" not in report:
        if not (repo_root(ledger_dir) / report).is_file():
            v.add(rid, f"{where}: local report {report!r} does not exist")
    if ev.get("kind") == "mutant-calibration":
        if not ev.get("intended_property"):
            v.add(rid, f"{where}: mutant calibration must name the intended property")
        if not ev.get("observed_rejection"):
            v.add(rid, f"{where}: mutant calibration must record the observed rejection")
        if ev.get("tool_present") is not True:
            v.add(rid, f"{where}: mutant must fail for its intended property, not a missing tool")
    cases = ev.get("cases_covered")
    if not isinstance(cases, list):
        v.add(rid, f"{where}: cases_covered must be a list")

def coverage_violations(req: dict, v: Violations) -> None:
    rid = req["id"]
    covered: set[str] = set()
    for ev in req.get("evidence_records", []):
        if ev.get("result") != "pass":
            v.add(rid, f"verified row cites evidence with result={ev.get('result')!r} — failed/skipped evidence is not success")
        covered.update(ev.get("cases_covered") or [])
    inventory = req.get("case_and_proof_inventory") or []
    missing = [case for case in inventory if case not in covered]
    if missing:
        v.add(rid, f"conjunctive cases without passing evidence: {missing}")
    if any(hint in " ".join(inventory).lower() for hint in PROOF_HINTS):
        obligations = [ev.get("obligations") for ev in req.get("evidence_records", [])]
        checked = [o.get("checked", 0) for o in obligations if isinstance(o, dict)]
        if not checked or max(checked) < 1:
            v.add(rid, "proof-bearing row cites zero executed proof obligations")


def lane_violations(req: dict, v: Violations) -> None:
    rid = req["id"]
    lanes = req.get("lane_status")
    if not isinstance(lanes, dict):
        return
    declared = set(req.get("required_platform_capability_lanes") or [])
    for lane, state in lanes.items():
        if lane not in declared:
            v.add(rid, f"lane_status names undeclared lane {lane!r}")
        if req["status"] == "verified" and not (state.startswith("verified") or state.startswith("not_required")):
            v.add(rid, f"verified row has unresolved lane {lane!r}: {state!r}")

def history_violations(req: dict, v: Violations) -> None:
    rid = req["id"]
    history = req.get("status_history", [])
    for step in history:
        if step.get("to") == "verified" and step.get("from") not in ("implemented_unverified", "in_progress", "verified"):
            v.add(rid, f"status_history jumps {step.get('from')!r} -> verified without an unverified stage")
        if step.get("from") == "verified" and step.get("to") != "verified" and step.get("waiver") is None:
            v.add(rid, f"downgrade verified -> {step.get('to')!r} requires an owner-approved waiver")
        waiver = step.get("waiver")
        if waiver is not None and not (waiver.get("owner_approved") and waiver.get("rationale")):
            v.add(rid, "waiver requires owner_approved=true plus rationale (no silent downgrade)")
    if any(s.get("waiver") for s in history) and req["status"] == "verified":
        v.add(rid, "waived row cannot simultaneously claim verified; reopen or re-verify with evidence")


def validate(ledger: dict, ledger_dir: Path) -> Violations:
    v = Violations()
    reqs = ledger["requirements"]
    by_id = {r["id"]: r for r in reqs}
    if len(by_id) != len(reqs):
        v.add("ledger", "duplicate requirement IDs")
    if ledger.get("requirement_count") != len(reqs):
        v.add("ledger", f"requirement_count={ledger.get('requirement_count')} but {len(reqs)} rows present — silent removal/addition")
    milestone_ids = {m["id"] for m in ledger.get("milestones", [])}
    if not milestone_ids:
        v.add("ledger", "no milestones registered")
    for m in ledger.get("milestones", []):
        for rid in m.get("required_delivery_ids", []):
            if rid not in by_id:
                v.add(m["id"], f"required delivery {rid} missing from requirements (silent removal)")
        for prereq in m.get("common_prerequisites", []):
            if prereq not in milestone_ids:
                v.add(m["id"], f"unknown prerequisite milestone {prereq!r}")
        for lane in m.get("lane_specific_prerequisites", []):
            target = lane.get("milestone") if isinstance(lane, dict) else lane
            if target not in milestone_ids:
                v.add(m["id"], f"unknown lane prerequisite {lane!r}")
    scope_names = set(ledger.get("closure_scopes", {}))
    global_ids = set(ledger.get("global_gate_ids", []))
    seen_global = [r for r in reqs if r["id"] in global_ids]
    if {r["id"] for r in seen_global} != global_ids:
        v.add("ledger", "global_gate_ids and requirement rows disagree")
    for req in reqs:
        rid = req["id"]
        if req.get("owner_milestone") not in milestone_ids | {"global"}:
            v.add(rid, f"unknown owner_milestone {req.get('owner_milestone')!r}")
        if req.get("closure_scope") not in scope_names | {"all_scopes"}:
            v.add(rid, f"unknown closure_scope {req.get('closure_scope')!r}")
        if req.get("status") not in STATUS_VALUES:
            v.add(rid, f"invalid status {req.get('status')!r}")
        for text_field in ("mandatory_deliverable", "required_acceptance_evidence", "source_document"):
            if not str(req.get(text_field, "")).strip():
                v.add(rid, f"empty {text_field}")
        state = req.get("lane_inventory_state", "")
        if state in ("registered", "live_registration_pending_expansion"):
            if not req.get("required_platform_capability_lanes"):
                v.add(rid, "registered lane inventory declares no lanes")
            if not req.get("case_and_proof_inventory"):
                v.add(rid, "registered case inventory is empty — named cases are conjunctive, zero is not a claim")
        for idx, ev in enumerate(req.get("evidence_records", [])):
            evidence_violations(req, ev, idx, v, ledger_dir)
        if req["status"] == "verified":
            if not req.get("evidence_records"):
                v.add(rid, "verified without any evidence record — a handwritten pass flag is not evidence")
            coverage_violations(req, v)
        lane_violations(req, v)
        history_violations(req, v)
    return v


def close_claim(ledger: dict, milestone: str | None, scope: str | None, release: str | None, ledger_dir: Path) -> Violations:
    v = validate(ledger, ledger_dir)
    reqs = ledger["requirements"]
    by_milestone: dict[str, list[dict]] = {}
    for req in reqs:
        by_milestone.setdefault(req["owner_milestone"], []).append(req)
    global_ids = set(ledger.get("global_gate_ids", []))
    targets: list[dict] = [r for r in reqs if r["id"] in global_ids]
    label: str
    if milestone:
        if milestone not in by_milestone or milestone == "global":
            raise SystemExit(f"unknown milestone {milestone!r}")
        targets += by_milestone[milestone]
        label = f"milestone {milestone}"
        for prereq in next(m for m in ledger["milestones"] if m["id"] == milestone).get("common_prerequisites", []):
            targets += by_milestone.get(prereq, [])
    elif scope:
        if scope not in ledger.get("closure_scopes", {}):
            raise SystemExit(f"unknown closure scope {scope!r}")
        for mid in ledger["closure_scopes"][scope]:
            targets += by_milestone.get(mid, [])
        label = f"closure scope {scope}"
    else:
        raise SystemExit("closure claim needs --close-milestone or --close-scope")
    seen: set[str] = set()
    for req in targets:
        if req["id"] in seen:
            continue
        seen.add(req["id"])
        if req["status"] != "verified":
            v.add(req["id"], f"{label} requires this row verified; status={req['status']!r}")
            continue
        if req.get("lane_inventory_state") != "registered":
            v.add(req["id"], f"{label} requires a fully registered lane/case inventory (state={req.get('lane_inventory_state')!r})")
        for ev in req.get("evidence_records", []):
            if release and ev.get("commit", "").split(" ")[0] != release:
                v.add(req["id"], f"evidence commit {ev.get('commit')!r} does not match claimed release {release}")
    if not any(r["status"] == "verified" for r in targets):
        v.add(label, "claim cites no verified rows at all — empty claims are not closure")
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
        return validate(ledger, args.ledger.parent).report("delivery inventory validation")
    if args.close_milestone or args.close_scope:
        if args.close_milestone and args.close_scope:
            raise SystemExit("pick one: --close-milestone or --close-scope")
        v = close_claim(ledger, args.close_milestone, args.close_scope, args.release, args.ledger.parent)
        return v.report(f"delivery closure claim ({args.close_milestone or args.close_scope})")
    parser.error("choose --validate, --close-milestone or --close-scope")


if __name__ == "__main__":
    sys.exit(main())
