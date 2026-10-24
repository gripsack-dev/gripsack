#!/usr/bin/env python3
"""Apply fixed reviewed owner amendments without rewriting imported requirements.

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

# The later standing architecture decision removes cross-architecture execution,
# not the mixed B6-03 native/cross identity and unsupported-target admission case.
X64_AMENDMENT_ID = "PLATFORM-LINUX-X64-2026-10-09"
X64_CASE_AMENDMENTS = {
    "B6-03": {
        0: None,
        1: None,
        2: "cache/receipt and CLI correctly identify native execution; unsupported host/target/toolchain combinations fail before any accidental execution, without claiming cross/emulated/remote production",
    },
}
X64_AMENDMENT = {
    "id": X64_AMENDMENT_ID,
    "authority": "plan/0048 Standing owner scope PLATFORM-LINUX-X64-2026-10-09",
    "active_support": ["Linux x86_64", "WSL2 Linux x86_64 execution environment"],
    "retired_requirement_lanes": {"B6-03": ["cross-target"]},
    "case_projection": {"B6-03": [
        {"original_index": 0, "disposition": "out_of_scope", "clause": "real explicit source/worker/target architecture pair and pinned cross toolchain/emulator policy"},
        {"original_index": 1, "disposition": "out_of_scope", "clause": "resulting target artifact executes on a compatible distinct target runtime"},
        {"original_index": 2, "disposition": "retained_native_and_invalid_target", "required_case": X64_CASE_AMENDMENTS["B6-03"][2]},
    ]},
    "retained": "all Linux x86_64/common runtime, invalid-target, proof and 96-partition persistence obligations",
    "revisit": "only a new explicit owner decision and native qualification; no emulated or historical pass",
}

# Exact reviewed leaves, indexed in the unchanged original inventory. These are
# mixed obligations: only engine/corpus execution is waived, never whole cases.
# Ordinary schema/type and hostile-context regressions remain required. The
# owner's later explicit golden-replay exclusion is recorded in plan0048 §17.
FUZZ_AMENDMENT_ID = "REL-FUZZ-2026-09-27"
FUZZ_CASE_AMENDMENTS = {
    "A1-07": {3: "v3/v4/v5 schema/parser acceptance exercises every admitted output kind with negative drift calibration; ordinary TypeScript type and emitter regressions remain required"},
    "A3-03": {3: "real crossprocess positive/negative wire fixtures exercise length and identity admission"},
    "A4-01": {4: "synthetic collision/link/long-prefix fixtures plus named Verus compatibility/closure mutations exercise the same production boundary"},
    "B1-03": {5: "actual Rust and Go decoders/state adapters run Go race fixtures; real Linux worker paths remain mandatory"},
    "B2-02": {4: "actual decoder limits and real Rust/Go/worker fixtures demonstrate the validation boundary, not a golden protobuf snapshot"},
    "B5-04": {0: "all applicable B0–B5 rows and global gates have exact-source production, native Linux, runtime, formal, Go race and protected CI evidence"},
    "E5-03": {1: "actual proof obligations are nonempty and named semantic mutants reject"},
    "C3-02": {3: "managed-block proposed bytes and existing foreign drift stay separate through the one shared operation planner; adversarial decoders run bounded regression fixtures"},
    "C5-02": {2: "shrinking graph oracles reject unknown-equal, PATH reorder, removed edge, stale binding and silent truncation mutants"},
    "D2-05": {3: "bounded adversarial archive and registry descriptors exercise actual production path, not a mock parser or syntax-only failure"},
    "D6-03": {2: "nonempty production Verus/TLC/TLAPS proofs and semantic mutants reject intended properties with attribution"},
}
FUZZ_EXCLUDED_CLAUSES = {
    "A1-07": "saved TypeScript golden IR corpus replay",
    "A3-03": "actual decoder fuzz seeds/campaigns",
    "A4-01": "formula-metadata parser fuzz fixtures",
    "B1-03": "persisted fuzz seeds, bounded fuzz",
    "B2-02": "persisted corrupt-LLB fuzz",
    "B5-04": "decoder fuzz",
    "E5-03": "relevant decoded IR/unit/plist/journal fuzz seeds/campaigns",
    "C3-02": "fuzz fixtures (not bounded regression fixtures)",
    "C5-02": "strict snapshot/JSON/TOML/saved-plan decoder fuzz replays",
    "D2-05": "fuzz (not bounded adversarial archive and registry descriptors)",
    "D6-03": "manifest/envelope/descriptor/path/registry fuzz campaigns",
}
FUZZ_RETAINED_CASES = {"A1-07": [1], "A1-08": [5]}
# Standalone fuzz lanes occur also on rows with no embedded fuzz clause. Their
# deterministic cases remain required on every retained lane. In particular,
# protocol-fuzz-race is NOT removed: Go race and protocol boundaries still run.
FUZZ_LANE_REQUIREMENTS = frozenset({
    "A3-03", "A4-01", "A5-04", "B5-04", "E5-03", "C1-01", "C1-04",
    "C3-02", "C3-04", "C5-02", "D1-01", "D2-05", "D3-01", "D3-03", "D6-03",
})
FUZZ_AMENDMENT = {
    "id": FUZZ_AMENDMENT_ID,
    "authority": "plan/0048 sections 15 and 17 explicit golden-replay exclusion; owner continuation reaffirmed in plan/0054",
    "disposition": "not_run_owner_waived",
    "applies_to": "feedback releases in this handover continuation only; not unrelated future releases",
    "excluded_requirement_lanes": {rid: ["fuzz"] for rid in sorted(FUZZ_LANE_REQUIREMENTS)},
    "case_projection": {
        rid: [{"original_index": index, "excluded_clause": FUZZ_EXCLUDED_CLAUSES[rid],
               "required_case": replacement, "disposition": "not_run_owner_waived"}
              for index, replacement in changes.items()]
        for rid, changes in FUZZ_CASE_AMENDMENTS.items()
    },
    "reviewed_without_exclusion": FUZZ_RETAINED_CASES,
    "retained": "fuzz workspace dependency policy, ordinary schema/type/emitter and deterministic hostile-context regressions, boundary/property, Go race, fault, model, Verus/TLAPS and persistence obligations",
    "meaning": "projected closure is not original fuzz completion; original inventories, historical evidence, runners and corpora remain intact",
}
AMENDMENTS = [AMENDMENT, X64_AMENDMENT, FUZZ_AMENDMENT]


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
    authorized = ledger.get("scope_amendments") == AMENDMENTS
    if not authorized:
        violations.add("ledger", "missing or unapproved scope amendment: expected only "
                       + ", ".join(amendment["id"] for amendment in AMENDMENTS))
    intact = baseline_digest(ledger) == BASELINE_SHA256
    if not intact:
        violations.add("ledger", "original lane/case/evidence-kind/prerequisite inventory changed; reviewed scope baseline required")
    for req in ledger["requirements"]:
        if req["id"] in OUT_OF_SCOPE and req.get("status") == "verified":
            violations.add(req["id"], f"out_of_scope under {AMENDMENT_ID} cannot count as verified")
        lane_status = req.get("lane_status", {})
        if req["id"] in FUZZ_LANE_REQUIREMENTS and lane_status.get("fuzz") == "verified":
            violations.add(req["id"], f"fuzz is not_run_owner_waived under {FUZZ_AMENDMENT_ID}, not verified")
        if req["id"] == "B6-03" and lane_status.get("cross-target") == "verified":
            violations.add(req["id"], f"cross-target is out_of_scope under {X64_AMENDMENT_ID}, not verified")
    return authorized and intact


def effective_requirement(req: dict) -> dict | None:
    """Project only after scope_violations admits the exact frozen inventory."""
    if req["id"] in OUT_OF_SCOPE:
        return None
    active = dict(req)
    excluded_lanes = set(RETIRED_LANES)
    if req["id"] == "B6-03":
        excluded_lanes.add("cross-target")
    if req["id"] in FUZZ_LANE_REQUIREMENTS:
        excluded_lanes.add("fuzz")
    lanes = [lane for lane in req["required_platform_capability_lanes"] if lane not in excluded_lanes]
    original = req["case_and_proof_inventory"]
    indexed_changes = dict(CASE_AMENDMENTS.get(req["id"], {}))
    indexed_changes.update(X64_CASE_AMENDMENTS.get(req["id"], {}))
    indexed_changes.update(FUZZ_CASE_AMENDMENTS.get(req["id"], {}))
    changes = {original[index]: replacement for index, replacement in indexed_changes.items()}

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
