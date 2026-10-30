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
# Later owner decisions supersede acceptance, never the imported inventory or
# historical evidence. Indices below address the same fingerprinted originals;
# no keyword classifier can silently waive a newly added or mixed requirement.
PRODUCT_AMENDMENT_ID = "PRODUCT-LINUX-NATIVE-2026-10-28"
PRACTICAL_AMENDMENT_ID = "REL-PRACTICAL-051-2026-10-28"
RETIRED_MILESTONES = frozenset({"A0", "A4", "A7", *(f"B{i}" for i in range(7))})
PRODUCT_RETIRED_REQUIREMENTS = frozenset({
    "A0-01", "A4-01", "A4-02", "A4-03", "A7-01", "A7-02",
    *(f"B{milestone}-{index:02}" for milestone, count in
      ((0, 4), (1, 5), (2, 7), (3, 5), (4, 4), (5, 6), (6, 5))
      for index in range(1, count + 1)),
})
# linux-image denotes B-qualified image production in this frozen inventory,
# not OCI Distribution package transport or executing an existing image.
PRODUCT_RETIRED_LANES = frozenset({"linux-image", "remote-worker", "cross-target", "protocol-fuzz-race"})
PRODUCT_CASE_AMENDMENTS = {
    "G-04": {0: "review applicable native A, C, D and E responsibility and dependency direction before calling a slice landed"},
    "G-05": {3: "native acquisition, check and preview never bootstrap retired builder components or execute effects merely to present a plan"},
    "A1-01": {0: "current schema/Rust/TypeScript writer and reader conform across supported native output kinds; historical strict stored readers preserve original meaning",
              4: "unsupported retired production declarations reject before activation or builder bootstrap"},
    "A1-02": {0: "decoded package/check/task/output references reject missing and wrong kinds with declaration spans",
              2: "runtime and task-prerequisite cycles reject with declaration spans; validation edges remain mandatory publication gates",
              3: "package/environment target and ABI mismatches reject with both declaration spans; fixed-prefix consumers declare compatible destinations",
              4: "ordered local commands remain distinct from runtime, task prerequisite, validation and retention roles",
              5: "versioned native workspace graph adapters preserve required checks through the production policy kernel"},
    "A1-03": {4: "native command host access stays explicit and hooks retain post_link/post_activate/on_remove lifecycle semantics"},
    "A1-04": {0: "raw archive, canonical tree, native package, workspace lock and OCI artifact envelope use distinct digest domains; provenance locations and labels are not semantic inputs",
              1: "native package pins, platform/ABI/layout and enforcement changes invalidate only affected identities",
              2: "equivalent declarations and diagnostic-only edits preserve native identities while ordered argv/PATH/action edits do not",
              3: "moving a compatible package between project and personal profile or changing deployment path cannot change its immutable identity"},
    "A1-05": {0: "versioned workspace lock pins target/platform resolution and imported frontend libraries with complete transitive acquisition and ecosystem records",
              2: "transitive package/import pin and captured declaration bytes invalidate affected plan identity; local cache paths and lease state do not",
              3: "bounded dirty-worktree/import snapshot binds admitted bytes and facts/probes before effects; development Tasks retain explicit live writable checkout"},
    "A1-06": {4: "read-only check/preview starts neither a builder nor a scheduler; unavailable capabilities fail at their declaration with actionable diagnostics"},
    "A1-07": {0: "AGENTS, plan STATUS and module ownership agree on current authoring and retained strict historical readers",
              3: "current and historical schema/parser acceptance covers supported native output kinds and negative drift; ordinary TypeScript type/emitter regressions remain required",
              4: "protected CI runs source inventory validation and architecture checks; selected practical reports bind required cases and lanes to exact source"},
    "A1-08": {0: "schema/Rust/TS executable and host-path distinctions share one exec/run_bash grammar for native owners; cwd and typed argv retain distinct roles",
              2: "unsupported task or schedule capabilities reject before effects at their declaration; implemented capabilities require actual native evidence",
              3: "typed command references imply their artifacts; artifact/package_command references resolve against the catalog with source-aware errors",
              4: "package identity, consumer selection and Task invocation remain distinct; consumer/invocation changes cannot grant new production authority",
              5: "decoded IR rejects forbidden phase grants and edge roles for native Task, hook, check and schedule owners with declaration spans and deterministic hostile-context regressions"},
    "A1-09": {1: "moving an unchanged declaration between modules or changing its diagnostic label preserves native semantics while a real script/action-order edit does not",
              2: "ordered command lists, Task prerequisites, package runtime refs, validators and hook intents retain distinct lifecycles",
              3: "forged phase/context flags cannot grant Task or host effects; unsupported production variants reject with declaration spans"},
    "A1-10": {1: "graduated native TypeScript workspace examples type-check and are admitted by the shipped CLI without hostname shims or executor effects",
              2: "ordered local commands remain distinct from named Task prerequisites and checked schedule bindings",
              5: "shared command construction preserves native validation, task-postcondition and hook lifecycles plus literal Bash source maps"},
    "A1-12": {3: "host mutation exclusion grants no backend, network or credential authority"},
    "A2-01": {3: "acquiring a package neither switches a personal generation nor invokes activation hooks or provisions a builder"},
    "A2-03": {0: "actual parser/constructor traces cover retained generations, project selections, process leases, typed job roots and unfinished recovery roots"},
    "A2-07": {2: "one compatible typed package command serves project and profile consumers without accidental personal activation or builder bootstrap",
              3: "generic downloads do not masquerade as coherent Conda environments; A3 retains ecosystem-specific success and rejection obligations"},
    "A5-02": {0: "terminal and JSON explain native pins and changed semantic package, command or configuration input from actual plan, lock and durable receipts"},
    "A5-04": {1: "graduated dotfiles, downloaded-tool and manual-Task examples are admitted and execute on native Linux x86_64"},
    "A3-04": {2: "legacy execution paths retire after native Linux replacement parity; retained-state readers preserve original receipts"},
    "A5-05": {0: "legacy Module, fetch/build/install/config, env, activation and preflip verification map to supported native owners and stages or explicit source-located migration refusal, without duplicate runtime"},
    "A5-06": {2: "typed source/rendered-content/destination, stage-aware checks, pure locks and Conda/Pixi providers work through real native project and profile consumers"},
    "A5-07": {2: "native journeys work without BuildKit, Lima or a mandatory Gripsack resident daemon; manager ownership limits stay truthful",
              3: "installed schedule journeys remain separately required at E3/E5 and are not claimed by manual native runs; source production is retired"},
    "A6-01": {3: "source archives and editable records reject; no source-to-Conda builder or silent source execution is offered"},
    "E0-01": {1: "inventory applicable native manual-task and calendar scenario families separately from retired source/VM cases"},
    "E1-02": {2: "Task artifact needs use the shared native realization API; completed host tasks are not immutable packages"},
    "E1-03": {3: "required package validators remain publication gates and cannot be discharged by cached Task success or postconditions"},
    "E1-06": {2: "read-only preview cannot fetch, register a job or execute commands; missing inputs and unsupported capabilities diagnose before effects"},
    "E1-07": {1: "failure/cancellation prevents successor commands and ordered postconditions; label-only moves preserve native identity while command reordering changes it",
              2: "pure scope/key mutation locks deduplicate and conflict with both source sites; namespaces do not alias or grant resource authority",
              4: "Task, check and hook owners admit only their own command contexts; forged phase/cache-control IR rejects before effects"},
    "E3-02": {1: "foreign systemd unit collision or drift refuses overwrite/delete; installation never adopts an unrelated manager job",
              2: "failed preparation leaves old revision intact; interrupted file publication, systemd registration and reload reconcile to honest active/disabled/unknown state"},
    "E3-04": {3: "cache removal never breaks a retained job; a systemd unit file is not the only durable root record"},
    "E4-01": {0: None,
              2: "the retained native tool runs after acquisition cache removal; no cached host-task result suppresses a new invocation"},
    "E4-03": {1: "failed replacement acquisition or validation retains the old active scheduled job and roots; in-flight runs stay bound to their old revision"},
    "E4-04": {0: "typed Task artifact requirements use the common native realization service, without recursive grip or source compilation",
              1: "acquisition and validation failures retain declaration and Task provenance; required validators cannot be pruned or replaced by invocation postconditions",
              2: "consumers sharing native realization retain independent leases; canceling one Task cannot cancel a still-leased attempt or publish a late canceled result"},
    "E5-04": {0: "a retained native Linux maintenance tool runs through actual systemd with per-invocation checks",
              3: "no general resident Gripsack daemon, all-formats schedule parser or builder host-task fallback is part of the calendar journey; services are a separate owner addition"},
    "E5-05": {2: "only the selected local profile grants registration; package acquisition and Task consumer definitions stay separate and compatible",
              3: None,
              4: "retained generation/job migration works; examples require only used task/profile/manager concepts with explicit OS limits"},
    "C0-01": {1: "map native acquisition, task, ownership and retained-state responsibilities without a second planner, identity or delivery ledger"},
    "C0-02": {1: "register Linux x86_64 native comparison lanes; no retired B image-production prerequisite applies"},
    "C1-02": {2: "argv, PATH, overlay, script and array order remain meaningful; unordered maps normalize, and native pin/import/policy changes affect the correct consumers",
              3: "native package, portable artifact, selected consumer and exact stored bytes retain distinct typed identities; historical producer identities retain their stored meaning"},
    "C2-01": {1: "runtime, validation, ordering-only and retention edges produce their documented effects without manufacturing content dependencies"},
    "C2-02": {1: "package pin and script changes affect the correct identities; trust/platform changes alter eligibility and validation",
              2: "changed package inputs report needs-realization rather than guaranteed execution or cache hit; selected bytes and runtime layout stay separate",
              3: "required native validators and unknown evidence remain visible even when no public output file changes"},
    "C2-05": {1: "package pin changes may require native realization and a new prepared revision; module-location-only changes stay provenance-only"},
    "C2-06": {0: "template-variable and declared native-input edits invalidate rendered consumers; unrelated dotfiles do not rebuild a tool"},
    "C3-01": {2: "native diff/plan works with all BuildKit/VM binaries absent; retired image production is not a prerequisite"},
    "C3-03": {0: "pure native snapshot comparison performs no repo eval, dependency acquisition, manager operation, host write or activation",
              2: "historical snapshots compare without Deno, builder or registry and enforce byte/depth/graph/time/output bounds"},
    "C4-03": {0: "artifact-bound plan requires exact admitted manifest/tree/target receipt; unresolved native realization remains explicitly unknown and retains required validators",
              2: "unavailable native workload capability fails without secretly granting host mode, force, takeover or bearer authority",
              3: "execution uses existing protected publication, transaction, activation and hook replay; arbitrary command or hook effects are not reversible"},
    "C5-01": {0: "two real project/worktree selections and an independent profile compare retained and proposed native receipts without a duplicate planner",
              1: "captured declaration, transitive pin, environment/PATH-only and runtime library edits yield distinct selected impact and realization explanations",
              3: "terminal/JSON predictions agree with executed native package/profile operations, protected receipts and rollback on Linux"},
    "C5-02": {1: "native Linux compare cases execute with actual source/result identities"},
    "C5-03": {0: "executed CLI/examples explain native/project pin changes, incomplete historical comparisons and actionable migrated old-state behavior",
              3: "all applicable native C0-C5 cases and essential global gates need independent evidence before product completion; retired B image cases do not"},
    "C5-04": {0: "real native Linux Task and installed job compare old and proposed captured revisions with actual manager/package receipts"},
    "D0-01": {0: "record applicable native A/C/E and sharing contracts/revisions without losing generation, store, planner or trust semantics"},
    "D0-02": {5: "fixed-prefix exact-match success and different-prefix refusal stay distinct from unrestricted relocation"},
    "D1-02": {2: "acquisition-only provenance is not runtime authority; all declared libraries, interpreters, plugins and data remain runtime requirements"},
    "D2-01": {2: "spaces and moved-root paths work with producer source, home and cache unavailable"},
    "D2-04": {1: None},
    "D4-01": {0: "a real signed package on an independent recipient with empty store and inaccessible producer/cache binds exact selected package/check/target policy and receipt",
              1: "eligible local trust and verified payload permit native process/project/profile consumption with no local source compilation",
              2: "absent or changed publisher authority, missing runtime roots or incompatible results cannot turn a candidate digest into authorization"},
    "D4-02": {0: "wrong package pin/lock/result, target/ABI/layout, enforcement, required checks, publisher namespace or current trust refuse substitution",
              2: "require-published miss fails explicitly; any permitted native acquisition fallback remains fully declared and validated",
              3: "no cache or unsupported platform request silently grants source compilation or weaker host authority"},
    "D4-03": {1: "required executable validators run under admitted native consumer policy and cannot be replaced by publisher text or prior Task success"},
    "D5-03": {1: None},
    "D5-04": {0: "different candidate native results retain distinct tree/result identities and cannot overwrite retained immutable data",
              1: "a frozen selected result stays pinned through recipient import/update despite a later candidate",
              3: "signing an archive or one clean fixture does not imply arbitrary byte-identical materialized prefixes"},
    "D6-01": {5: None},
    "D6-02": {0: "one real CLI/JSON semantic report explains selected native acquisition or verified publisher substitution and rejected publisher/layout/target"},
    "D6-03": {0: "native Linux and private-registry independent-client jobs run on exact released source with documented dependencies"},
    "D6-05": {0: "a retained native Linux tool is imported by independent recipients at different roots with producer store, registry and cache inaccessible"},
}
OPTIONAL_REQUIREMENTS = frozenset({"G-03"})
OPTIONAL_LANES = frozenset({"proofs", "tlc", "verus-amd64", "fuzz", "platform-time"})
# Pure assurance clauses are omitted from practical admission; mixed clauses
# retain their ordinary behavioral regressions. These are not proof successes.
PRACTICAL_CASE_AMENDMENTS = {
    "H0-02": {2: "pending runtime and optional assurance remain explicit; selected practical closure still requires applicable registry and native-manager cases"},
    "G-02": {0: "register each selected practical lane's positive and negative cases and expected execution counts before judging results",
             1: "actual selected runner commands, nonempty source-bound reports, pass/fail/skip counts and case coverage agree"},
    "G-08": {1: "no pending, blocked or implemented-unverified selected product requirement is reported complete; optional assurance is reported separately"},
    "A1-01": {5: None},
    "A1-02": {6: "ordinary graph/closure regressions reject lost mandatory validation edges"},
    "A1-03": {5: "ordinary normalization regressions check fluent/object equivalence and reject changed command semantics"},
    "A1-04": {4: "ordinary schema/serialization/admission regressions preserve native equivalence, invalidation and typed digest boundaries"},
    "A1-05": {5: "production lock/plan admission, historical stored locks and source-bound negative regressions remain required"},
    "A1-06": {6: "diagnostic registry parity, positive/negative runner reports and selected source-bound CI lanes remain required"},
    "A1-10": {6: "ordinary normalization/provenance regressions and protected CI cover the supported authoring surface"},
    "A1-11": {9: "ordinary source/content normalization and file-ownership policy regressions exercise actual production operations and trusted boundaries"},
    "A1-12": {5: "ordinary lock normalization/admission and typed emitter/parser regressions reject conflicts and incorrect mutation grants"},
    "A2-02": {3: None, 4: "real parsed receipts, locks, platforms, prefixes and validators exercise publication admission; ordinary crash/fault regressions reject publication before validation"},
    "A2-03": {3: "production root adapters agree with actual parsed receipts; ordinary omitted-runtime-root regressions reject missing retained dependencies"},
    "A2-06": {5: "ordinary ownership/merge/graph regressions agree with real emitted operations and reject wrong source origins, cached foreign merges and lost checks"},
    "A2-P-02": {3: "ordinary composition/admission regressions reject conflicting operations at their production boundary"},
    "A2-P-03": {2: "real process/GC regressions exercise lifetime and publication decisions", 3: "ordinary regressions reject dropped runtime roots, premature release and accidental personal-generation mutation"},
    "A3-03": {4: "ordinary helper admission regressions bind success to the live attempt instead of trusting a success flag"},
    "A5-04": {0: "all selected native A/A2-P cases and essential global gates require source-bound implementation/runtime evidence; optional assurance is separate"},
    "E0-01": {3: "optional production proof targets remain inventoried separately, without asserting they ran"},
    "E1-05": {0: "ordinary production task admission regressions cover edges, prerequisite readiness, postcondition gating and lock conflicts",
              1: None, 2: "ordinary diamond/parameter/late-event graph and real decoded-IR, scheduler and lock regressions agree",
              3: "ordinary negative regressions reject omitted artifact edges, false prerequisites, accepted conflicts and lost failures"},
    "E1-07": {5: "ordinary readiness/conflict regressions cover action-list and lock boundaries without a parallel universal Step runtime"},
    "E3-05": {0: None, 1: None,
              2: "ordinary decoded-job/journal admission, lease and root regressions exercise actual registration and launch boundaries",
              3: "ordinary stale-launch, early-root-release, omitted-runtime-root and false-success regressions reject with native manager evidence"},
    "E5-01": {1: None,
              2: "OS/version/user-scope differences and ordinary long-running/coalesced job observations remain visible; unrun disruptive sleep/wake, logout/login, reboot and clock/DST campaigns remain explicit optional assurance, with no portable or exactly-once timing guarantee"},
    "E5-02": {3: "schema, Rust/TS, essential practical gates and native Linux evidence bind exact source without skipped selected cases"},
    "E5-03": {0: "all selected native E cases, ordinary systemd manager lifecycle lanes and essential global gates require current-source results",
              1: None,
              3: "missing selected systemd registration, calendar invocation, update, disable, rollback, GC, unsupported timing-policy admission or exact-byte translation evidence rejects; optional disruptive timing campaigns and proof/fuzz remain separately unrun, not passing"},
    "C0-02": {2: "freeze report/JSON/exit completeness, ordinary positive/negative cases and resource budgets; optional proof inventories are separate"},
    "C1-04": {0: "ordinary production equivalence/admission regressions check semantic equality, byte-envelope distinction and invalid fields",
              2: "ordinary independent semantic regressions cover equivalent forms, order and unknown-as-equal rejection",
              3: "parser/field completeness, cryptographic assumptions and arbitrary TypeScript determinism remain explicit limits"},
    "C1-06": {3: "ordinary decoder regressions reject wrong source-origin/lock/check semantics without a parallel comparator"},
    "C2-04": {0: "ordinary production impact regressions cover typed old/new graph relations",
              2: "ordinary dropped-edge/runtime/validator regressions reject at the actual consumer boundary",
              3: "graph correctness is not a claim that the filesystem supplied a complete snapshot"},
    "C4-05": {0: "ordinary saved-plan admission regressions require reviewed scope and exact identity before effects",
              1: None, 2: None,
              3: "ordinary source/lock/receipt/session regressions reject stale tokens and changed trust",
              4: "filesystem races, hashes and OS durability remain explicit tested/trusted boundaries"},
    "C5-02": {0: "fresh essential practical gates and exact-source CI exercise ordinary semantic comparison and saved-plan regressions",
              2: "ordinary graph/decoder regressions reject unknown-equal, PATH reorder, removed edges, stale bindings and silent truncation",
              4: "missing selected Linux or oversized-data cases remain open; optional proof/fuzz and retired B image work are not substituted passes"},
    "D0-03": {4: "register ordinary production-admission, independent interoperability and Linux qualification cases; optional proof catalogs remain separate"},
    "D1-04": {0: "ordinary production closure/admission regressions check runtime reachability, DAG and target/layout compatibility with explicit external ABI limits",
              2: "ordinary regressions reject dropped runtime roots, build-only-as-runtime and mismatched prefixes",
              3: "declared closure validation does not claim discovery of every arbitrary binary dependency"},
    "D4-05": {0: "ordinary production binding regressions check current publisher scope, package/result/target/layout/check enforcement and terminal attempt authorization",
              2: "ordinary lifetime regressions exercise imports, conflicting results, revocation, cancellation, stale export, crash and GC",
              3: None,
              4: "ordinary negative regressions reject key-ID authority, wrong payload, missing runtime members, warm trust bypass, omitted validators, canceled publication and lost roots"},
    "D6-03": {2: None,
              4: "missing selected Linux, independent recipient, registry or supported dynamic fixture stays blocked; optional assurance is separate, never a skipped/mock pass"},
    "D6-04": {3: "all selected native D cases, dynamic fixtures, registry and essential global gates require actual evidence before sharing product completion"},
}
PRODUCT_AMENDMENT = {
    "id": PRODUCT_AMENDMENT_ID,
    "authority": "explicit owner amendment 2026-10-28; plan/0048 owner-scope register",
    "active_support": ["Linux x86_64", "WSL2 Linux x86_64 execution environment"],
    "retired_products": ["Mac", "Homebrew", "BuildKit", "source-to-Conda production", "Gripsack image production"],
    "retired_requirements": sorted(PRODUCT_RETIRED_REQUIREMENTS),
    "retired_milestones": sorted(RETIRED_MILESTONES),
    "retired_lanes": sorted(PRODUCT_RETIRED_LANES),
    "prerequisite_projection": {"A1": {"common_prerequisites": ["H0"]}},
    "case_projection": {rid: {str(index): text for index, text in changes.items()}
                        for rid, changes in PRODUCT_CASE_AMENDMENTS.items()},
    "retained": "native task, ownership, retention, trust and consumer semantics; OCI Distribution artifact sharing and running existing images remain required and distinct from retired image production",
    "history": "all original IDs, fields, statuses, reports and source fingerprints remain unchanged; no retirement is verification",
}
PRACTICAL_AMENDMENT = {
    "id": PRACTICAL_AMENDMENT_ID,
    "authority": "explicit owner amendment 2026-10-28; LOCAL-PRACTICAL-QUALIFICATION-2026-10-28; plan/0048",
    "optional_requirements": sorted(OPTIONAL_REQUIREMENTS),
    "optional_lanes": sorted(OPTIONAL_LANES),
    "optional_checks": ["formal proofs", "model checking", "fuzz campaigns", "exhaustive persistence campaigns", "clean-build reproducibility campaigns", "delivery checker calibration", "disruptive native sleep/wake, logout/login, reboot and clock/DST campaigns"],
    "optional_disposition": "not_required_by_default; run only on explicit owner request; unrun is not passing",
    "essential_checks": ["Rust", "TypeScript", "ordinary flows", "dependency audits", "packaging", "installed consumers", "real smokes for changed behavior", "source inventory validation"],
    "case_projection": {rid: {str(index): text for index, text in changes.items()}
                        for rid, changes in PRACTICAL_CASE_AMENDMENTS.items()},
    "completion": "selected product completion is separate from optional assurance; original 178-row closure proof is not a release prerequisite",
}
SERVICES_ADDITION = {
    "id": "SERVICES-FIRST-2026-10-28",
    "authority": "explicit owner addition 2026-10-28; plan/0055",
    "sequence": "after Bash/task QoL, before calendar scheduling",
    "scope": "native Linux/WSL2 x86_64 services; distinct from finite tasks and calendar jobs",
    "status": "implemented_unverified",
    "evidence": "parent must record actual source-bound practical and native evidence before claiming completion",
    "edition_5": "separate owner addition, not a retroactive requirement or an added original delivery row",
}
AMENDMENTS = [AMENDMENT, X64_AMENDMENT, FUZZ_AMENDMENT, PRODUCT_AMENDMENT, PRACTICAL_AMENDMENT]


def effective_milestone(milestone: dict) -> dict | None:
    """Retire only the explicit products, keeping native prerequisites."""
    if milestone["id"] in RETIRED_MILESTONES:
        return None
    active = dict(milestone)
    active["common_prerequisites"] = [
        mid for mid in milestone.get("common_prerequisites", []) if mid not in RETIRED_MILESTONES
    ]
    # A0 used to carry H0 transitively. Removing Brew cannot remove source
    # inventory/handover admission from the surviving native authoring slice.
    if milestone["id"] == "A1":
        active["common_prerequisites"] = ["H0"]
    active["lane_specific_prerequisites"] = [
        item for item in milestone.get("lane_specific_prerequisites", [])
        if (item.get("milestone") if isinstance(item, dict) else item) not in RETIRED_MILESTONES
    ]
    active["required_delivery_ids"] = [
        rid for rid in milestone.get("required_delivery_ids", [])
        if rid not in OUT_OF_SCOPE | PRODUCT_RETIRED_REQUIREMENTS | OPTIONAL_REQUIREMENTS
    ]
    return active


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
    additions_authorized = ledger.get("owner_additions") == [SERVICES_ADDITION]
    if not additions_authorized:
        violations.add("ledger", "missing or unapproved separate services owner addition")
    if not intact:
        violations.add("ledger", "original lane/case/evidence-kind/prerequisite inventory changed; reviewed scope baseline required")
    for req in ledger["requirements"]:
        # B0-01 already has historical verified qualification receipts. Keep
        # that record intact without counting it toward the selected product.
        if req["id"] in PRODUCT_RETIRED_REQUIREMENTS - {"B0-01"} and req.get("status") == "verified":
            violations.add(req["id"], f"retired product under {PRODUCT_AMENDMENT_ID} cannot count as verified")
        if req["id"] in OUT_OF_SCOPE and req.get("status") == "verified":
            violations.add(req["id"], f"out_of_scope under {AMENDMENT_ID} cannot count as verified")
        lane_status = req.get("lane_status", {})
        if req["id"] in FUZZ_LANE_REQUIREMENTS and lane_status.get("fuzz") == "verified":
            violations.add(req["id"], f"fuzz is not_run_owner_waived under {FUZZ_AMENDMENT_ID}, not verified")
        if req["id"] == "B6-03" and lane_status.get("cross-target") == "verified":
            violations.add(req["id"], f"cross-target is out_of_scope under {X64_AMENDMENT_ID}, not verified")
    return authorized and intact and additions_authorized


def effective_requirement(req: dict) -> dict | None:
    """Project only after scope_violations admits the exact frozen inventory."""
    if req["id"] in OUT_OF_SCOPE | PRODUCT_RETIRED_REQUIREMENTS | OPTIONAL_REQUIREMENTS:
        return None
    active = dict(req)
    excluded_lanes = set(RETIRED_LANES)
    excluded_lanes.update(PRODUCT_RETIRED_LANES | OPTIONAL_LANES)
    if req["id"] == "B6-03":
        excluded_lanes.add("cross-target")
    if req["id"] in FUZZ_LANE_REQUIREMENTS:
        excluded_lanes.add("fuzz")
    lanes = [lane for lane in req["required_platform_capability_lanes"] if lane not in excluded_lanes]
    original = req["case_and_proof_inventory"]
    indexed_changes = dict(CASE_AMENDMENTS.get(req["id"], {}))
    indexed_changes.update(X64_CASE_AMENDMENTS.get(req["id"], {}))
    indexed_changes.update(FUZZ_CASE_AMENDMENTS.get(req["id"], {}))
    indexed_changes.update(PRODUCT_CASE_AMENDMENTS.get(req["id"], {}))
    indexed_changes.update(PRACTICAL_CASE_AMENDMENTS.get(req["id"], {}))
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
    active["evidence_kinds"] = [kind for kind in req["evidence_kinds"] if kind != "formal"]
    active["evidence_records"] = [
        ev for ev in req.get("evidence_records", [])
        if ev.get("lane") in lanes and ev.get("kind") in active["evidence_kinds"]
    ]
    return active
