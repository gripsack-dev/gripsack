#!/usr/bin/env python3
"""Apply one reviewed owner amendment without rewriting imported requirements.

This is not a waiver registry. The original lane/case/kind/prerequisite inventory
is fingerprinted; only the explicit projection below changes active acceptance.
Historical evidence is never promoted or rewritten by this projection.
"""
from __future__ import annotations

import hashlib
import json

AMENDMENT_ID = "PLATFORM-LINUX-WSL-2026-10-08"
RETIRED_LANES = frozenset({"macos", "macos-arm64", "macos-vm", "launchd"})
OUT_OF_SCOPE = frozenset({"B0-02", "E4-02", "D2-03"})
AMENDMENT = {
    "id": AMENDMENT_ID,
    "authority": "owner decision 2026-10-08; plan/0048 owner-scope register",
    "active_support": ["Linux", "WSL2 Linux execution environment"],
    "retired_lanes": sorted(RETIRED_LANES),
    "out_of_scope_requirements": {rid: "out_of_scope" for rid in sorted(OUT_OF_SCOPE)},
    "application": "scripts/delivery_scope.py; original inventories and evidence remain historical authority",
    "revisit": "only a new explicit owner decision; no automatic Mac resumption",
}
BASELINE_SHA256 = "2993f4d1d521f968f9083c52609ef763c139d5695063f8b92912305598888a2c"

# Zero-based positions in the fingerprinted original flat inventories. None
# retires only a wholly Mac case. Mixed cases retain every Linux/common clause.
# Existing by-lane assignments are used, not rebuilt from matching prose.
# Pure unsupported-target counterexamples and historical data compatibility
# remain required even where they mention Mac; they are not support promises.
CASE_AMENDMENTS = {
    "H0-02": {2: "future runtime/proof rows may remain pending but their inventories cannot defer any applicable registry, Linux native-manager or formal cases at closure; Mac-only scope is retired, not verified"},
    "G-05": {4: "each advertised Linux, WSL2 and other active capability claim has separate actual evidence; one platform result never qualifies another; Mac is not advertised as supported"},
    "A0-01": {2: None},
    "A2-P-04": {2: "native Linux x86_64 is exercised with exact platform/ABI compatibility and without a false hermetic claim for host-access development tasks"},
    "A3-02": {4: "native Linux loader/libc behavior is executed; incompatible ABI is an actionable refusal, not an inference from another platform"},
    "A4-03": {
        0: "freeze an exact native OS/architecture/version/ABI/layout support matrix before results, including the Linux bottle tags actually advertised",
        1: "execute each advertised Linux architecture/version/layout combination with required command/runtime closure cases and retained post-cache behavior",
        3: "success on one Linux architecture or version never qualifies another; each newly advertised architecture requires its own native evidence",
    },
    "A5-04": {
        1: "four graduated examples are admitted and the dotfiles/downloaded-tool/manual-Task native subset actually executes on supported Linux paths",
        3: "release-support matrix records Linux/WSL2 support and Mac retirement, and any blocked applicable lane as open; published capability/CLI/API docs and guaranteed proof claims match exact artifacts",
    },
    "A5-07": {0: "dotfiles-only, downloaded native tool with owned config and basic named manual Task journeys execute on native Linux x86_64"},
    "A6-01": {2: "real Python/compiled-extension command fixtures run on native Linux after staging/cache loss and preserve the final-prefix contract"},
    "A7-01": {3: None},
    "B1-01": {
        0: "actual grip build on a clean qualified Linux local worker provisions its own pinned daemon/Go bridge only when the selected graph requires Linux building",
        1: "complete verified bootstrap manifest pins every applicable runtime, helper and bridge byte without manual socket/start or floating install script dependencies",
    },
    "B1-03": {5: "actual Rust and Go decoders/state adapters run persisted fuzz seeds, bounded fuzz and Go race fixtures; real Linux worker paths remain mandatory"},
    "B3-01": {3: None},
    "B3-03": {2: None},
    "B3-05": {3: "the full E4 Linux manual/timer journey remains E-owned and open"},
    "B5-02": {
        0: "small Rust service uses native tools on supported Linux, the Linux package and mandatory checks through BuildKit, and an independently runnable image with declared runtime dependencies",
        6: "native release-only workflow provisions no builder; A5 old-state migration remains intact",
        7: "E's separate source-tool/manual-and-timer journey is still E-owned and cannot be claimed by this source-image fixture",
    },
    "B5-04": {
        0: "all applicable B0–B5 rows and global gates have exact-source production, native Linux, runtime, formal, decoder fuzz/Go race and protected CI evidence",
        1: "Rust/musl core and pinned Go bridge release bytes match their reviewed image/helper manifests and preserve native independent bootstrap",
        2: "final cold/warm/offline bytes, disk/RAM/startup/cleanup compared to B0 Linux budgets with regressions explained; static source/OCI checks are not runtime evidence",
        4: "a blocked applicable worker/prover lane stays open rather than being represented by a skipped test or a weaker source-only substitute",
    },
    "E2-01": {0: "daily and weekly local-time schedules normalize weekday/time with an explicit user-scope capability before selecting the native Linux manager"},
    "E2-02": {
        1: None,
        3: None,
        4: "checked unit bytes remain identical through protected staging and installation, including replacement-between-check-and-register rejection",
    },
    "E2-03": {
        2: "task prerequisites stay inside the Gripsack graph, not per-task OS dependency units, and native Linux config injects no shell or hidden restart",
        3: "container-gate parser/dispatch fixtures bind admitted unit bytes to the one runner argv graph without claiming the native OS manager executed inside a Linux container",
    },
    "E2-04": {2: None},
    "E4-04": {3: None},
    "E5-01": {0: "actual Linux user-manager fixture installs, fires, reports, disables and removes the owned job"},
    "E5-02": {3: "schema, Rust/TS, five Compose gates and native Linux CI bind exact source with no skipped required cases"},
    "E5-03": {
        0: "every applicable E0–E5 row, named case, native Linux manager/timing capability lane and global gate has current-source complete results",
        1: "actual proof obligations are nonempty, named semantic mutants reject, and relevant decoded IR/unit/journal fuzz seeds/campaigns run",
        3: "delivery checker rejects missing Linux sleep/clock timing, exact-byte translation, proof or failed fixture rather than accepting mock/skip evidence; retired Mac work is never verified",
    },
    "E5-04": {0: "Linux source-built daily maintenance tool runs through actual systemd with per-invocation checks"},
    "E5-05": {
        0: "same admitted command and Task runs both manually and from actual native systemd manager with one source-mapped result and postconditions on every invocation",
        3: "Linux source-versus-download producer swap runs through B3 without Task rewrite",
    },
    "C0-02": {1: "register Linux x86_64 and the B-qualified Linux image lane with their prerequisites; C0 inventory does not execute those later runtime cases"},
    "C3-01": {0: "actual native Linux CLI compares two retained generations, compatible snapshots and selected current versus proposed locked declarations through one admitted semantic model"},
    "C5-01": {3: "compare C's before/after terminal and JSON predictions against executed source/package/image operations, protected receipts and rollback result on native Linux lanes"},
    "C5-02": {
        1: "native Linux compare and B-qualified Linux image cases execute separately with actual source/result identities",
        4: "a missing Linux, proof, B image or oversized-data case remains open and cannot be hidden as skipped",
    },
    "C5-04": {0: "real E4 Linux source-produced Task and installed job are compared in old and proposed captured revisions with actual manager/package receipts"},
    "D0-02": {3: None},
    "D0-03": {4: "register production-entry proof kernels, independent oracles, mutants and Linux qualification evidence inventory before declaring any result"},
    "D2-02": {1: None},
    "D5-02": {0: "clean Linux recipients start with core, explicit local trust policy and A6 bundle only, including any separately approved pinned helper bootstrap"},
    "D6-01": {
        0: "producer creates qualified Linux static/dynamic/plugin/interpreter and Conda fixtures with exact signed manifests/receipts and runtime closures",
        2: "separate Linux x86_64 recipients have distinct user/home/store and no access to producer store/worker/cache; imported native tools run at different roots",
    },
    "D6-03": {
        0: "required native Linux, independent Linux image and private-registry client jobs run on exact committed/released source with documented dependency versions",
        4: "missing Linux, independent recipient, registry, prover or supported successful dynamic fixture stays blocked, never represented by a skipped/mock pass",
    },
    "D6-04": {3: "every applicable D0–D6 row, Linux dynamic fixture, format proof, registry and global gate is verified before full sharing release is claimed"},
    "D6-05": {
        0: "E4 Linux source-built tool is imported by independent local recipients at different roots with producer store/worker/registry/cache inaccessible",
        1: "recipient-authored Task and profile install systemd schedule; two separate invocations execute checks and retain job/process/tool roots through GC",
    },
}


def baseline_digest(ledger: dict) -> str:
    fields = ("id", "required_platform_capability_lanes", "case_and_proof_inventory",
              "case_and_proof_inventory_by_lane", "evidence_kinds")
    milestone_fields = ("id", "common_prerequisites", "lane_specific_prerequisites",
                        "required_delivery_ids", "global_gates_apply")
    inventory = {
        "requirements": [{key: row.get(key) for key in fields}
                         for row in sorted(ledger["requirements"], key=lambda row: row["id"])],
        "milestones": [{key: row.get(key) for key in milestone_fields}
                       for row in sorted(ledger["milestones"], key=lambda row: row["id"])],
        "closure_scopes": ledger.get("closure_scopes"),
        "global_gate_ids": ledger.get("global_gate_ids"),
    }
    return hashlib.sha256(json.dumps(inventory, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def scope_violations(ledger: dict, violations) -> bool:
    authorized = ledger.get("scope_amendments") == [AMENDMENT]
    if not authorized:
        violations.add("ledger", f"missing or unapproved scope amendment: expected only {AMENDMENT_ID}")
    intact = baseline_digest(ledger) == BASELINE_SHA256
    if not intact:
        violations.add("ledger", "original lane/case/evidence-kind/prerequisite inventory changed; reviewed scope baseline required")
    for req in ledger["requirements"]:
        if req["id"] in OUT_OF_SCOPE and req.get("status") == "verified":
            violations.add(req["id"], f"out_of_scope under {AMENDMENT_ID} cannot count as verified")
    return authorized and intact


def effective_requirement(req: dict) -> dict | None:
    """Project only after scope_violations admits the exact frozen inventory."""
    if req["id"] in OUT_OF_SCOPE:
        return None
    active = dict(req)
    lanes = [lane for lane in req["required_platform_capability_lanes"] if lane not in RETIRED_LANES]
    original = req["case_and_proof_inventory"]
    changes = {original[index]: replacement for index, replacement
               in CASE_AMENDMENTS.get(req["id"], {}).items()}

    def cases(items: list[str]) -> list[str]:
        return [changes.get(case, case) for case in items if changes.get(case, case) is not None]

    active["required_platform_capability_lanes"] = lanes
    by_lane = req.get("case_and_proof_inventory_by_lane")
    if by_lane is not None:
        selected = {lane: cases(by_lane[lane]) for lane in lanes}
        active["case_and_proof_inventory_by_lane"] = selected
        retained = {case for inventory in selected.values() for case in inventory}
        active["case_and_proof_inventory"] = [case for case in cases(original) if case in retained]
    else:
        active["case_and_proof_inventory"] = cases(original)
    active["lane_status"] = {lane: state for lane, state in req.get("lane_status", {}).items() if lane in lanes}
    active["evidence_records"] = [ev for ev in req.get("evidence_records", []) if ev.get("lane") in lanes]
    return active
