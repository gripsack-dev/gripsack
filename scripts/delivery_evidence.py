#!/usr/bin/env python3
"""Runner evidence, lane coverage and source identity for delivery closure.

Pure admission checks plus bounded reads of repo-local reports. The
caller in check_delivery.py owns milestone/closure aggregation.
"""
from __future__ import annotations

import hashlib
import re
import subprocess
import sys
from pathlib import Path

STATUS_VALUES = {"pending", "in_progress", "implemented_unverified", "blocked", "failed", "verified"}
RESULTS = {"pass", "fail", "skipped", "blocked"}
EVIDENCE_FIELDS = ("lane", "date", "entry_points", "environment", "command", "result",
                   "report", "report_sha256", "commit", "cases_covered", "kind")
# Named proof deliveries, not a keyword heuristic: e.g. G-02 discusses
# obligation counts but is not itself a new theorem. Other new proof
# families must be added here with the corresponding required row.
PROOF_ROWS = {
    "G-03", "A1-04", "A1-11", "A2-02", "A2-06", "A2-P-03",
    "A3-03", "A4-01", "A5-04", "B1-04", "B2-03", "E1-05",
    "E1-07", "E3-05", "E5-03", "C1-04", "C1-06", "C2-04",
    "C4-05", "D1-04", "D4-05", "D6-03",
}
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^[0-9a-f]{64}$")
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
# Tracked behavior-bearing inputs. Evidence-only commits may reuse a
# runner report if these trees are byte-identical at both revisions.
# Adding a new source root requires updating this inventory in review.
SOURCE_ROOTS = (
    "Cargo.toml", "Cargo.lock", "Dockerfile", "docker-compose.yml",
    "crates", "typescript", "schema", "scripts", "specs", "e2e",
    "examples", "fuzz", ".github/workflows", "verification/buildkit-qualification",
)

MAX_REPORT_BYTES = 16 * 1024 * 1024


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
    """Only a ledger inside a checkout may resolve local evidence."""
    for candidate in [directory, *directory.parents]:
        if (candidate / ".git").exists():
            return candidate.resolve()
    raise ValueError(f"{directory}: delivery ledger is outside a repository")


def evidence_violations(req: dict, ev: dict, idx: int, v: Violations, root: Path) -> None:
    rid = req["id"]
    where = f"evidence[{idx}]"
    for field in EVIDENCE_FIELDS:
        if field not in ev:
            v.add(rid, f"{where}: missing field {field!r}")
    for field in ("lane", "environment", "command", "report", "report_sha256", "commit"):
        if not isinstance(ev.get(field), str) or not ev[field].strip():
            v.add(rid, f"{where}: {field} must be a nonempty string")
    if not isinstance(ev.get("entry_points"), list) or not ev["entry_points"] or not all(
        isinstance(point, str) and point.strip() for point in ev["entry_points"]
    ):
        v.add(rid, f"{where}: entry_points must name production paths")
    if not isinstance(ev.get("cases_covered"), list) or not all(
        isinstance(case, str) and case.strip() for case in ev["cases_covered"]
    ):
        v.add(rid, f"{where}: cases_covered must be an array of named cases")
    if not isinstance(ev.get("date"), str) or not DATE_RE.fullmatch(ev["date"]):
        v.add(rid, f"{where}: date must be YYYY-MM-DD")
    if not isinstance(ev.get("commit"), str) or not COMMIT_RE.fullmatch(ev["commit"]):
        v.add(rid, f"{where}: commit must be an exact 40-hex source revision")
    if ev.get("result") not in RESULTS:
        v.add(rid, f"{where}: result must be one of {sorted(RESULTS)}")
    kind = ev.get("kind")
    if kind not in ("runner", "review", "mutant-calibration"):
        v.add(rid, f"{where}: kind must be runner, review or mutant-calibration")
    if kind == "mutant-calibration" and (
        not ev.get("intended_property") or not ev.get("observed_rejection") or ev.get("tool_present") is not True
    ):
        v.add(rid, f"{where}: mutant must fail for its intended property, not a missing tool")
    report = ev.get("report")
    digest = ev.get("report_sha256")
    if not isinstance(digest, str) or not DIGEST_RE.fullmatch(digest):
        v.add(rid, f"{where}: report_sha256 must be a 64-hex digest of runner/review bytes")
    if not isinstance(report, str) or not report:
        return
    rel = Path(report)
    if rel.is_absolute() or ".." in rel.parts or "://" in report:
        v.add(rid, f"{where}: report must be a repository-relative file")
        return
    report_file = (root / rel).resolve()
    if not report_file.is_relative_to(root) or not report_file.is_file():
        v.add(rid, f"{where}: local report {report!r} does not exist inside the repository")
        return
    if report_file.stat().st_size > MAX_REPORT_BYTES:
        v.add(rid, f"{where}: runner report exceeds the {MAX_REPORT_BYTES}-byte admission limit")
        return
    data = report_file.read_bytes()
    if not data or (isinstance(digest, str) and hashlib.sha256(data).hexdigest() != digest):
        v.add(rid, f"{where}: report is empty or its SHA-256 does not match actual bytes")
    if kind in ("runner", "mutant-calibration"):
        if rel.parts[:2] != ("verification", "reports") or rel.suffix not in (".log", ".json"):
            v.add(rid, f"{where}: runner report must be under verification/reports/ as .log or .json, not Markdown")
        if not all(isinstance(ev.get(field), str) and ev[field].strip() for field in ("tool_versions", "inputs")):
            v.add(rid, f"{where}: runner must identify tool_versions and inputs/configuration")
        marker = ev.get("report_marker")
        if not isinstance(marker, str) or not marker.strip() or marker.encode() not in data:
            v.add(rid, f"{where}: report_marker must occur in the actual runner report")
        counts = ev.get("counts")
        if not isinstance(counts, dict) or not all(
            isinstance(counts.get(key), int) and not isinstance(counts[key], bool) and counts[key] >= 0
            for key in ("executed", "passed", "failed", "skipped")
        ):
            v.add(rid, f"{where}: runner counts must specify executed/passed/failed/skipped as nonnegative integers")
        elif (counts["executed"] == 0 or counts["executed"] != sum(
            counts[key] for key in ("passed", "failed", "skipped")
        ) or (ev.get("result") == "pass" and (counts["failed"] or counts["skipped"]))):
            v.add(rid, f"{where}: zero executions, mismatched counts or skipped/failed required cases")
        if isinstance(counts, dict) and isinstance(marker, str) and ev.get("result") == "pass":
            passed = counts.get("passed")
            if isinstance(passed, int) and not re.search(rf"(?<!\d){passed}(?!\d)", marker):
                v.add(rid, f"{where}: report_marker does not contain its claimed passed count")
            obligations = ev.get("obligations")
            if proof_row(req) and isinstance(obligations, dict):
                checked = obligations.get("checked")
                if isinstance(checked, int) and not re.search(rf"(?<!\d){checked}(?!\d)", marker):
                    v.add(rid, f"{where}: report_marker does not contain its claimed proof obligation count")


def proof_row(req: dict) -> bool:
    return req["id"] in PROOF_ROWS


def coverage_violations(req: dict, records: list[dict], v: Violations, lanes: set[str]) -> None:
    rid = req["id"]
    inventory = set(req.get("case_and_proof_inventory") or [])
    for lane in lanes:
        passing = [
            ev for ev in records
            if ev.get("lane") == lane and ev.get("result") == "pass"
        ]
        if not any(ev.get("kind") in ("runner", "mutant-calibration") for ev in passing):
            v.add(rid, f"{lane}: no passing source-bound runner report")
        covered = set()
        for ev in passing:
            covered.update(ev.get("cases_covered") or [])
        if inventory - covered:
            v.add(rid, f"{lane}: conjunctive cases without passing evidence: {sorted(inventory - covered)}")
        if proof_row(req):
            obligations = [
                ev.get("obligations") for ev in passing
                if ev.get("kind") in ("runner", "mutant-calibration")
            ]
            if not any(
                isinstance(o, dict)
                and all(isinstance(o.get(k), int) and not isinstance(o[k], bool)
                        for k in ("expected", "checked", "failed", "skipped"))
                and o["expected"] > 0 and o["checked"] == o["expected"]
                and o["failed"] == o["skipped"] == 0
                for o in obligations
            ):
                v.add(rid, f"{lane}: zero executed proof obligations or expected/checked mismatch")


def lane_violations(req: dict, v: Violations) -> None:
    rid = req["id"]
    declared = set(req.get("required_platform_capability_lanes") or [])
    lanes = req.get("lane_status") or {}
    if not isinstance(lanes, dict):
        v.add(rid, "lane_status must be an object")
        return
    for lane, state in lanes.items():
        if lane not in declared or state not in STATUS_VALUES:
            v.add(rid, f"lane_status has undeclared lane or invalid state: {lane!r}={state!r}")
    if req["status"] == "verified":
        if req.get("lane_inventory_state") != "registered" or not declared:
            v.add(rid, "verified row has unresolved lane inventory")
        if set(lanes) != declared or any(state != "verified" for state in lanes.values()):
            v.add(rid, f"verified row has missing or unresolved required lanes: {sorted(declared - set(lanes))}")


def history_violations(req: dict, v: Violations) -> None:
    rid = req["id"]
    history = req.get("status_history", [])
    if req["status"] != "pending" and not history and req["owner_milestone"] != "global":
        v.add(rid, "non-pending status has no recorded transition")
    for index, step in enumerate(history):
        if step.get("from") not in STATUS_VALUES or step.get("to") not in STATUS_VALUES:
            v.add(rid, f"status_history[{index}] has an unknown state")
        if index and history[index - 1].get("to") != step.get("from"):
            v.add(rid, f"status_history[{index}] does not follow the previous transition")
        if step.get("from") == "verified" and step.get("to") != "verified":
            # Invalidating false evidence reopens a claim; it never
            # waives or removes the requirement.
            if step.get("reason") != "invalid_evidence" or not step.get("detail"):
                v.add(rid, "downgrade verified -> unverified requires invalid_evidence detail (not a waiver)")
        if step.get("waiver") is not None:
            v.add(rid, "a status-history waiver cannot change required scope; owner amendment required")
    if history and history[-1].get("to") != req["status"]:
        v.add(rid, "status does not match the final recorded transition")


def source_fingerprint(root: Path, revision: str) -> str | None:
    if not COMMIT_RE.fullmatch(revision):
        return None
    tree = subprocess.run(
        ["git", "ls-tree", "-r", "--full-tree", "-z", revision, "--", *SOURCE_ROOTS],
        cwd=root, capture_output=True, check=False,
    )
    if tree.returncode != 0 or not tree.stdout:
        return None
    return hashlib.sha256(tree.stdout).hexdigest()


def source_binding_violations(
    ev: dict, release: str | None, root: Path, fingerprints: dict[str, str | None],
    v: Violations, identity: str,
) -> None:
    tested = ev.get("commit")
    if tested == release and release and COMMIT_RE.fullmatch(release):
        return
    if not release or not COMMIT_RE.fullmatch(release) or not isinstance(tested, str):
        v.add(identity, f"evidence from mismatched implementation {tested!r}")
        return
    if tested not in fingerprints:
        fingerprints[tested] = source_fingerprint(root, tested)
    claimed = fingerprints.get(release)
    if (
        not claimed or fingerprints[tested] != claimed
        or ev.get("source_reuse_sha256") != claimed
        or not ev.get("source_reuse_reason")
    ):
        v.add(identity, f"evidence from mismatched implementation {tested!r}: source trees differ or documented reuse missing")
