#!/usr/bin/env python3
"""Negative calibration for scripts/check_delivery.py (handover A1-07).

The closure checker must reject each dishonest-completion shape the
handover names; a checker that only passes good ledgers proves
nothing. Every case below mutates the real ledger in a temp dir and
asserts a nonzero exit plus a message naming the violated rule. The
same pattern as the model gate's failing configs: required failures,
not optional coverage.

Cases (handover §"Make incomplete completion claims fail CI"):
  1. missing required row            (silent requirement removal)
  2. fake verified without report    (handwritten pass flag)
  3. skipped Mac job                 (blocked lane claimed closed)
  4. zero proof obligations          (proof row, no executed proofs)
  5. failed positive case            (cited evidence is a failure)
  6. mutant killed by missing tool   (not its intended property)
  7. mismatched implementation       (evidence from another revision)
  8. downgrade without waiver        (verified reopened silently)
plus the clean ledger must validate.
"""
from __future__ import annotations

import copy
import json
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CHECKER = REPO / "scripts" / "check_delivery.py"
LEDGER = REPO / "verification" / "delivery.json"

PASS_EVIDENCE = {
    "lane": "pure",
    "date": "2026-09-24",
    "entry_points": ["synthetic fixture"],
    "environment": "calibration fixture",
    "command": "calibration fixture command",
    "result": "pass",
    "report": "report.md",
    "commit": "0" * 40,
    "cases_covered": ["calibration-case"],
}


def run(ledger_path: Path, *extra: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(CHECKER), "--ledger", str(ledger_path), *extra],
        capture_output=True,
        text=True,
    )


def base() -> dict:
    return json.loads(LEDGER.read_text())


def requirement(ledger: dict, rid: str) -> dict:
    return next(r for r in ledger["requirements"] if r["id"] == rid)


def write(tmp: Path, ledger: dict) -> Path:
    (tmp / "report.md").write_text("calibration report (synthetic)\n")
    path = tmp / "delivery.json"
    path.write_text(json.dumps(ledger))
    return path
def case_missing_row(tmp: Path) -> str:
    ledger = base()
    ledger["requirements"] = [r for r in ledger["requirements"] if r["id"] != "B0-04"]
    out = run(write(tmp, ledger), "--validate")
    assert out.returncode != 0 and "B0-04" in out.stderr, out.stderr
    return "missing required row rejected"


def case_fake_verified(tmp: Path) -> str:
    ledger = base()
    row = requirement(ledger, "H0-02")
    row["status"] = "verified"
    row["evidence_records"] = []
    out = run(write(tmp, ledger), "--validate")
    assert out.returncode != 0 and "not evidence" in out.stderr, out.stderr
    return "verified without evidence rejected"


def case_skipped_mac(tmp: Path) -> str:
    ledger = base()
    row = requirement(ledger, "E0-02")
    row["status"] = "verified"
    row["evidence_records"] = [dict(PASS_EVIDENCE, lane="launchd", result="skipped")]
    out = run(write(tmp, ledger), "--validate")
    assert out.returncode != 0 and ("launchd" in out.stderr or "skipped" in out.stderr), out.stderr
    return "skipped Mac lane rejected"


def case_zero_obligations(tmp: Path) -> str:
    ledger = base()
    row = requirement(ledger, "A1-04")
    row["status"] = "verified"
    row["case_and_proof_inventory"] = ["production normalization/admission proof over identity inputs"]
    row["evidence_records"] = [dict(PASS_EVIDENCE, cases_covered=["production normalization/admission proof over identity inputs"])]
    out = run(write(tmp, ledger), "--validate")
    assert out.returncode != 0 and "zero executed proof obligations" in out.stderr, out.stderr
    return "zero proof obligations rejected"


def case_failed_positive(tmp: Path) -> str:
    ledger = base()
    row = requirement(ledger, "A0-01")
    row["status"] = "verified"
    row["case_and_proof_inventory"] = ["intel-mac-lexical-counterexample"]
    row["evidence_records"] = [dict(PASS_EVIDENCE, result="fail", cases_covered=["intel-mac-lexical-counterexample"])]
    out = run(write(tmp, ledger), "--validate")
    assert out.returncode != 0 and "result='fail'" in out.stderr, out.stderr
    return "failed positive case rejected"


def case_mutant_wrong_reason(tmp: Path) -> str:
    ledger = base()
    row = requirement(ledger, "A0-01")
    row["status"] = "verified"
    row["evidence_records"] = [
        dict(
            PASS_EVIDENCE,
            kind="mutant-calibration",
            intended_property="linux bottles never selected on macOS",
            observed_rejection="checker missing python3",
            tool_present=False,
            cases_covered=["intel-mac-lexical-counterexample"],
        )
    ]
    row["case_and_proof_inventory"] = ["intel-mac-lexical-counterexample"]
    out = run(write(tmp, ledger), "--validate")
    assert out.returncode != 0 and "missing tool" in out.stderr, out.stderr
    return "mutant killed by missing tool rejected"


def case_mismatched_revision(tmp: Path) -> str:
    ledger = base()
    row = requirement(ledger, "H0-01")
    row["status"] = "verified"
    row["evidence_records"] = [dict(PASS_EVIDENCE, cases_covered=row["case_and_proof_inventory"])]
    out = run(write(tmp, ledger), "--close-milestone", "H0", "--release", "f" * 40)
    assert out.returncode != 0 and "does not match claimed release" in out.stderr, out.stderr
    return "mismatched-implementation evidence rejected"


def case_silent_downgrade(tmp: Path) -> str:
    ledger = base()
    row = requirement(ledger, "E0-02")
    row["status"] = "verified"
    row["lane_status"] = {"systemd-linux": "verified"}
    row["evidence_records"] = [dict(PASS_EVIDENCE, lane="systemd-linux", cases_covered=row["case_and_proof_inventory"])]
    row["status_history"] = [{"date": "2026-09-24", "from": "verified", "to": "pending"}]
    out = run(write(tmp, ledger), "--validate")
    assert out.returncode != 0 and "waiver" in out.stderr, out.stderr
    return "verified→pending without owner waiver rejected"


def case_clean() -> str:
    out = run(LEDGER, "--validate")
    assert out.returncode == 0, out.stderr
    return "clean real ledger validates"


def main() -> int:
    results = [case_clean()]
    cases = [
        case_missing_row,
        case_fake_verified,
        case_skipped_mac,
        case_zero_obligations,
        case_failed_positive,
        case_mutant_wrong_reason,
        case_mismatched_revision,
        case_silent_downgrade,
    ]
    with tempfile.TemporaryDirectory() as tmpdir:
        for case in cases:
            results.append(case(Path(tmpdir)))
    for line in results:
        print(f"  {line}")
    if len(results) != len(cases) + 1:
        print("FAIL: a calibration case did not run", file=sys.stderr)
        return 1
    print(f"delivery checker calibration: {len(cases)} negative cases rejected, clean ledger accepted")
    return 0


if __name__ == "__main__":
    sys.exit(main())
