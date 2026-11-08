<!-- Appendix to plan/0056 §9 "Handover reconciliation". Generated 2026-11-08 by a read-only review of origin/main @187d741; ~30 statuses verified in code, the rest from plan/release records. Spot-check by the integrator: create_temp AlreadyExists→remove_file (gripsack-fs/src/lib.rs:115) and missing SECURITY.md confirmed; generation .staging-* residue only partly confirmed (directory staging IS retired by gc/directory_roots.rs) — re-verify before acting. -->

# Original handover reconciliation (input to plan 0056 §9)

Repo: `gripsack-foundation` at `187d741` (origin/main `ff8dd8f`, 0.51.0, plus plan 0056).
Sources: plan 0048 (§1–§9, §13 R1–R8, §6 M-V1–M-V8, §14–§19 records), 0049 + `verification/delivery.json`
(178 rows, 5 scope amendments, 1 owner addition), 0050–0055, `verification/{continuation,release}-handoff.md`,
`verification/reports/2026-10-23-h0-semantic-review.json`, STATUS.md, plus targeted greps of the code.

## 1. Executive summary

1. The original handover had two parts. Plan 0048 was the 0.42 review response: 18 findings (§1–§8), packets R1–R8, and proofs M-V1–M-V8, with every NEXT item blocking release. Plan 0049 imported the edition-5 bundle (H0, A0–A7, B0–B6, E0–E5, C0–C5, D0–D6 and G-01–G-08), which is 178 protected rows.
2. **All 0048 P0 boundary defects are fixed** and are in the code: comma-safe Deno grants E133, repo-env/credential audience E400, `HostName` E132, bounded frontend output, archive link containment, strict priors, and private journals. R1 source-bundle approval with Landlock confinement and R3 durable hook intents also landed (0.43–0.44).
3. **The product scope of the edition-5 handover has mostly shipped**, across 0.47 → 0.51. That covers the workspace contract (A1/A2/A2-P), native Conda/Pixi/wheels (A3/A6), API retirement (A5), tasks and systemd schedules (E1–E4), semantic diff and saved plans (C), signed portable sharing over OCI registries (D), and the owner-added Docker services.
4. **The ledger was never updated to match.** `delivery.json` still reads 157 pending, 14 in_progress, 5 implemented_unverified, 1 verified and 1 blocked, and STATUS still shows 0048/0049 as "(pending)". The real qualification evidence for this work is in the 0055 release records, not the ledger.
5. **Moot, retired by owner decisions:** all Mac/launchd/ARM work (PLATFORM-LINUX-WSL/X64), and all of Homebrew and BuildKit (PRODUCT-LINUX-NATIVE-2026-10-28 / 0055). That retires B0–B6, A0, A4, A7, B0-02/D2-03/E4-02, and the Homebrew/crates.io/npm publication fixes in §7/§8.
6. **Moot through the code changes in 0.50:** the module scheduler panic fix plus M-V2 Loom, Verify-step E109/E115/E110, `resources` on steps, and the old module-DSL schema parity work. The module system was deleted, and workspace realization runs no worker threads.
7. **Superseded by 0056:** every delivery/evidence/closure mechanism (H0-02, G-01–G-08, the A1-07 checker, the 39 formal-row catalogs, release receipts and signing, the 0053 Buildbot work, and the 17/12-gate policies). Also the proof programme itself (M-V1, M-V3, M-V5–M-V7 in TLAPS/Verus), which becomes the Lean theorems K1–K6 and P1–P4. M-V8 has become the F1 pilot.
8. **Still pending, all small and runtime-level:** generation staging residue is never reaped (§4.3). `create_temp` unlinks a colliding temp file (§4.5). There is no doctor inspection or manual-repair route for an unrestorable journal entry (§4.6). Trace logs are written with default permissions and grow without bound (R5/F-12). `grip update --json` does not exist (R8). There is no `SECURITY.md` (R7). Typed ownership roles are missing (R2 `plan_copy`/`plan_link` still take strings and bools).
9. **Pending product gaps:** an unreceipted file→directory replacement is refused (0055), substitution-before-build (D4) has no code, and A5-02 explanations and A5-03 dogfooding are incomplete. Automated mutable-overlay snapshots (R45-18) were deliberately not shipped. The ecosystem repos (§8 E3–E9) were not verified.
10. **Drop candidates under 0056 §11:** signing, VM/physical power-loss campaigns, disruptive sleep/reboot/DST timing campaigns, liveness claims, and complete-tree containment tiers.

## 2. Reconciliation table

Status key: LANDED (version/commit), PARTIAL, PENDING, MOOT-RETIRED, SUPERSEDED-BY-0056.
Home (for PENDING/PARTIAL): ring 0/1 = kernel/protocol theorem, ring 2/3 = host/edge code, product, drop.
"✓code" means I confirmed it in the source tree, not only in plan text.

### 2.1 Evaluator boundary (0048 §1, R1, R5)

| ID(s) | Description | Status | Evidence | Home |
|---|---|---|---|---|
| 0048 §1.1 / M0-1.1a–c | Comma-in-path Deno read-grant injection | LANDED 0.43 (`953724a`) | ✓code E133 in `gripsack-ir/src/diagnostic_codes.rs`, `schema/diagnostics.json` | — |
| §1.2 / M0-1.2a–e | `[eval.env]` repo-controlled env/credential audience/TLS | LANDED 0.43 (`1c693ef`) | ✓code E400; `e2e/test_http_policy.py`; TLC `CredentialRouting.tla` | — |
| §1.3 / M0-1.3a–d | Unvalidated `--host`/lock path escape | LANDED 0.43 (`681e87b`) | ✓code `gripsack-ir/src/host_name.rs`, E132 | — |
| §1.4 / M0-1.4a–b | Unbounded frontend output; adopt touches files before trust | LANDED 0.43 (`0b92905`) | guarantees FRONTEND-BOUNDS-001, ADOPT-TRUST-001 | — |
| R1.1–R1.12 | Approve and evaluate the same captured source bundle; trust v2; evaluation receipts | LANDED 0.43 (`cc91d36`), Linux | ✓code `gripsack-store/src/source_bundle/`, `schema/trust/v2.json`, `schema/evaluation/v1.json` | — |
| R1 npm blocker / EVAL-CONFINEMENT-001 | Hoisted `node_modules` loaded unapproved code; fixed with Landlock OS confinement | LANDED (Linux) | ✓code `gripsack-process/src/confinement.rs` | — |
| R1 Mac seatbelt leaf | macOS confinement | MOOT-RETIRED (PLATFORM-LINUX-WSL-2026-10-08) | 0048 §9 amendment table | — |
| R5 bounds | Distinct input/frame/stdout/stderr types, supervised callers, reserved hook IDs | LANDED (M-V7.5/7.6) | ✓code typed `*ByteLimit` in `self_update/verified.rs`; PROCESS-BOUNDS-001 | — |
| R5 / F-12 trace privacy and retention | `$GRIPSACK_HOME/runs/*.jsonl` written with default permissions; no size or retention budget | PENDING (M3 in 0048 §9) | ✓code `gripsack-trace/src/lib.rs` uses `create_dir_all` + `File::create` with no limits (113 lines) | ring 2/3 |
| R5 complete-tree containment tiers | Stronger descendant containment | CLAIM-GATED, never started | 0048 §9 R5 row | drop |

### 2.2 Archive / fetch / acquisition (0048 §2, M-V7 budgets, A3, A6)

| ID(s) | Description | Status | Evidence | Home |
|---|---|---|---|---|
| §2.1 / M0-2.1a–b | TAR/ZIP composed-symlink escape | LANDED 0.43 (`db1e91b`) | ✓code `UnsafeArchive` in `gripsack-fetch/src/fetch/archive/paths.rs` + tests | — |
| §2.2 | Plugin receipt source identity; throttle admits NaN/inf/<1 | LANDED | ✓code `throttle.rs::rate_admission_rejects_nonfinite_and_unfillable_buckets`; `plugins.rs` | — |
| M-V7.2–7.4 | `rate_limit`, `operation_budget`, `retry_budget` Verus kernels | LANDED as Verus → SUPERSEDED-BY-0056 | ✓code `gripsack-policy/src/{rate_limit,operation_budget,retry_budget}.rs`; 0056 §8 demotes them to plain Rust plus property tests | ring 2/3 (per 0056) |
| A3-01–A3-04 | Rattler solve/import, final-prefix materialization, bounded helper protocol, project/profile parity | LANDED 0.46 → 0.51 (protocol 4) | `gripsack-conda`, `tools/conda-helper`; real frozen Conda consumer in each release | — |
| A3-01 example failure (0049) | External example's unpinned Pixi ripgrep digest mismatch | LANDED (frozen `pixi.fromLock`) | example PR6/PR7 typecheck and `grip check` green (0055) | — |
| A6-01, A6-02 | Hash-pinned wheels; portable bundles with original archives | LANDED 0.51 | `e2e/test_package_native_wheel.py`, `workspace/sharing/conda.rs` | — |
| R45-03/05d/06/07 | Conda libc baseline, EXDEV staging, lifecycle-script preflight, `archiveRoot` | LANDED 0.46–0.47 | ✓code `workspace/conda/inventory.rs` (post-link); UP46.3/5d | — |

### 2.3 Scheduler / update (0048 §3, R8)

| ID(s) | Description | Status | Evidence | Home |
|---|---|---|---|---|
| §3.1 / M0-3.1, M-V2 Loom, 0047 PureScheduler | Worker panic deadlocks apply; Loom coverage | MOOT-RETIRED (fixed in 0.43; module scheduler and its Loom gate deleted in 0.50) | ✓code: no `thread::spawn`/`catch_unwind`/loom in `gripsack-exec`; 0055 "Loom gate disappear[s] with dead implementation" | — |
| §3.2, M-V7.7, R8 core | `update` vs `update --check` verdict parity; complete-only survey | LANDED | ✓code `gripsack-policy/src/update_survey.rs`, `exec/report.rs`; UPDATE-SURVEY-001 | — |
| §3.3 | `resources` on Verify/Intent steps never acquired | MOOT-RETIRED (steps removed in 0.50) | 0055 `f0b7c9b` | — |
| R8 JSON coverage | `grip update --json` structured coverage output | PENDING (M3) | ✓code: `Update` in `main.rs` has no `--json` | product |
| R8 arbitrary-effect `--preflight` | — | Rejected in 0048 | — | drop |

### 2.4 Recovery / store (0048 §4, R3, R6, M-V4/5/6)

| ID(s) | Description | Status | Evidence | Home |
|---|---|---|---|---|
| §4.1 / M0-4.1 | Manifest `prior` bypassed strict admission | LANDED 0.43 | 0048 §14 M0-4.1 row | — |
| §4.4 / M0-4.4 | Journal/quarantine world-readable | LANDED | ✓code `journal/storage.rs` → `private_state::{ensure,restrict}_directory/file` | — |
| §4.2 | Duplicate-destination validation keyed on spelling | LANDED | ✓code `generations/admission.rs:172` uses `entry.key()`; E111 in workspace sema | — |
| §4.3 | Generation staging residue (`.staging-N-pid`) never reaped after a crash; vacuous test assertion | **PENDING** | ✓code `generations/publication.rs:129–144` removes only its own pid dir; `generations.rs:343` still asserts `.staging-1` | ring 1 (P3 cleanup) + ring 2 |
| §4.5 | `create_temp` unlinks a colliding temp file (could belong to another writer) | **PENDING** | ✓code `gripsack-fs/src/lib.rs` `create_temp`: AlreadyExists → `remove_file` → retry | ring 2 |
| §4.6 | Unrestorable-entry wedge: doctor inspection and documented manual repair | **PENDING** (M4) | ✓code `commands/doctor.rs` (263 lines) has no journal/recovery inspection | ring 1 (recovery decision) + product (doctor) |
| §4.7 | Destination observation unbounded (ledger-only note) | PENDING (docs-only) | not found in `guarantees.md` | drop, or list as a named residual in the 0056 claims |
| R3.1–R3.8, M-V6.3 | Stable hook intent IDs, durable outcomes, duplicate simulation, `grip hooks` | LANDED 0.43–0.44 | ✓code `gripsack-store/src/activation/{ledger,inspect}.rs`, `exec/activate/effect.rs`, `Hooks` command | F5 candidate (activation pending-state) |
| M-V6.4–6.10 concrete durability repairs | Seal priors/manifests before authority, allocation floor, batch prune | LANDED 0.43 | 0048 §16 rows; JOURNAL-CHAIN-001, PRIOR-DURABILITY-001 | — |
| R6 changes 1–5 | Identity/applicability contract and per-filesystem matrix | PARTIAL (identity contract and 96 Linux persistence partitions exist; no published applicability matrix) | `gripsack-fs/src/identity.rs`; 0054/0055 partitions | ring 2 (0056 S1–S6 "Linux ext4/btrfs/xfs/tmpfs; WSL2 named") |
| R6 VM/power-cut campaign | Real power-loss evidence | MOOT under 0056 §11 (no physical power-loss claims) | 0056 §11 | drop |
| 0055 directory gap | Unreceipted native file→directory replacement is refused (pre-existing since 0.48) | **PENDING** (product limitation) | 0055 integration observations; `e2e/test_workspace_directory_evolution.py` | ring 0 (K1/K2 ownership) + product |

### 2.5 Contracts / frontend / workspace (0048 §5, A1, A2, A2-P, A5, R45)

| ID(s) | Description | Status | Evidence | Home |
|---|---|---|---|---|
| §5 Verify-action E109/E115, stepped E110 | — | MOOT-RETIRED (step pipeline removed in 0.50) | 0055 contract | — |
| §5 remaining schema/DSL parity (`vars`/`marker` E108, `append`, lint protocol schema, zero-match diagnostics, resource order) | Module-DSL parity | MOOT-RETIRED (module DSL deleted; IR9 → IR10 workspace-only) | 0055 | drop |
| A1-01–A1-12 | Workspace/output contract, typed outputs, `runBash`, diagnostics, locks, commands, file contract, lock refs | LANDED in product (IR8 0.47 → IR10 0.51); ledger rows stale | ✓code SDK exports `pkg/environment/profile/task/check/hook/schedule/service/lock/ensureArtifact/runBash/templateFile/repoTree/managedBlock/trackedCopyTo` | — |
| A2-01–A2-07, A2-06 | Native acquisition/publication/retention/realization; profile file checks | LANDED 0.47–0.50 | ✓code `exec/workspace/{realize,roots,retained,profile_checks}` | — |
| A2-P-01–05 | Project run/shell/named task, env composition, reuse | LANDED 0.47 | ✓code `Run`, `Shell`, `Task` commands | — |
| A5-01, A5-05, A5-06 | Alpha API/IR cutover, Module/Step migration, primitive retirement | LANDED 0.50 (`f0b7c9b`, −37.5k lines) | 0055 | — |
| A5-02 | Selection/rebuild/dependency explanations in terminal and JSON | PARTIAL (`plan --json`, `check --json`, `diff --json`, `why-owns`; no rebuild/selection "why") | ✓code `main.rs` | product |
| A5-03 | Gripsack dogfooding its own project + a common integration entry point | PENDING | ✓code: no `gripsack.ts` at repo root | product, or drop |
| A5-04, A5-07 | Release closure/support docs; small executed examples | LANDED 0.50/0.51 | `examples/workspaces/{01,02,04}`; site PR26/27 | — |
| R45-01–R45-17 | 0.45 reviewer packet (host-runtime roots, migration, launchers, `templateFile`, `repoTree`, capture globs, `--focus` review, ELF/static-PIE, etc.) | LANDED 0.46–0.48 | 0054 table; ✓code `consumer/admit/elf.rs`, `profile_launchers.rs` | — |
| R45-18 | Automated snapshot for mutable tool overlays | PENDING by design (only a documented manual workflow shipped) | 0054 R45-18 row | product, or drop |

### 2.6 Tasks and schedules (E0–E5)

| ID(s) | Description | Status | Evidence | Home |
|---|---|---|---|---|
| E0-01, E0-02 (systemd lane) | Scope/vocabulary; real systemd probe | LANDED (record) | plan 0050; `2026-09-26-e0-systemd-ce3c7e0.log` | — |
| E0-02 launchd, E4-02 | Mac scheduling | MOOT-RETIRED (PLATFORM-LINUX-WSL) | `delivery.json` scope_amendments | — |
| E1-01–E1-07 | Finite task graph, artifact requirements, locks, timeouts, manual task UI | LANDED 0.47–0.51 | ✓code `task`, `lock`, `ensureArtifact`; 53 real-CLI task/schedule cases (0055) | — |
| E2-01–E2-04, E3-01–E3-04, E3-06 | systemd translation, captured revisions, registration lifecycle, roots, unattended runs | LANDED 0.51 | ✓code `exec/workspace/schedules/{systemd,lifecycle,runner}`, `grip jobs`; real systemd 255 journey | — |
| E4-01 | Source build, then repeated task use | PARTIAL: source-build half MOOT (BuildKit retired); repeated native task use LANDED | 0055 | — |
| E4-03, E4-04 | Stable prepared revision; build-only/cancellation boundaries | LANDED 0.51 | 0055 scheduler journey (in-flight old revision completes) | — |
| E5-01 idx1, platform-time lane | Disruptive sleep/logout/reboot/DST campaigns | Owner-optional, never run | REL-PRACTICAL-051 | drop (0056 §11) |
| E3-05 | Lifecycle formal evidence with real bridges | SUPERSEDED-BY-0056 | — | ring 0/1 (F5 "schedule lifecycle") |

### 2.7 Semantic change review (C0–C5)

| ID(s) | Description | Status | Evidence | Home |
|---|---|---|---|---|
| C0–C1, C3, C4-01–C4-04, C4-06, C5-03/04 | Snapshots, typed change model, offline diff/config views, private reports, saved-plan envelope, stale refusal | LANDED 0.51 (`75269f4`) | ✓code `exec/semantic/*`, `exec/saved_plan/*`; `Diff`, `Snapshot`, `DiffConfig`, `plan --save`, `apply --saved-plan`; 26-command journey | — |
| C2-01–C2-06 | Impact propagation and causal explanations | LANDED 0.51 (exact depth not audited) | `semantic/compare.rs`, `operations.rs` | — |
| C1-04, C2-04, C4-05 | Production proof kernels for comparison/impact/saved binding | PARTIAL Verus (`gripsack-policy/src/semantic.rs`) → SUPERSEDED-BY-0056 | — | ring 0/1 (F5) |
| C5-01 image portion, C5-02 CI/fuzz gates | Image change journey; required CI evidence | MOOT (image production) / SUPERSEDED-BY-0056 | — | — |

### 2.8 Artifact sharing (D0–D6) and services

| ID(s) | Description | Status | Evidence | Home |
|---|---|---|---|---|
| D0, D1-01–03, D2-01/02/05, D3-01–04, D5-01–06, D6-01/02/04/05 | Portable manifests, runtime closure, static/dynamic Linux packages, DSSE trust/revocation, offline + OCI registry transport, env export/import, independent recipient + scheduled tool | LANDED 0.51 | ✓code `exec/workspace/sharing/*`, `grip package pack/sign/import/inspect/export/push/pull`; authenticated-TLS registry and independent-filesystem journeys | — |
| D2-03 (Mac), D2-04 (bottle adapter) | — | MOOT-RETIRED (Mac; Homebrew) | — | — |
| D4-01–D4-03, D4-06 | Eligible substitution of a shared artifact *before build* | **PENDING / likely moot**: no substitution code; no build production remains after 0055 | ✓code: grep finds no substitution logic | owner decides: drop, or product |
| D4-04 | Producer-independent lifetime | LANDED (recipient works after producer deleted) | 0055 | — |
| D1-04, D4-05, D6-03 | Closure/trust/substitution proof kernels; fuzz calibration | SUPERSEDED-BY-0056 | — | ring 0/1 (F5 "sharing/closure admission") |
| SERVICES-FIRST-2026-10-28 | Docker Engine services, ownership, readiness, data-volume retention | LANDED 0.51 (ledger still says implemented_unverified) | ✓code `exec/services/{engine,plan,state}.rs`, `up/down/services`; PostgreSQL journey of 27 commands | F5 candidate (service identity) |

### 2.9 Release / CI / assurance / ecosystem (0048 §7, §8, R4, R7; 0049 H0/G; 0053)

| ID(s) | Description | Status | Evidence | Home |
|---|---|---|---|---|
| §7.1, §7.2, §7.6 | crates.io order, Homebrew-before-release, npm provenance | MOOT (publication is now manual, GitHub-only and unsigned; `release-core.yml` publishes nothing) | ✓code `.github/workflows/release-core.yml` | — |
| §7.3 | Recompute artifact digests across handoffs | LANDED via local release scripts → SUPERSEDED-BY-0056 (scripts deleted in F0) | `scripts/release_*.py` | — |
| §7.4, §7.5 | CI permissions/concurrency; `gate` aggregator | LANDED | ✓code `ci.yml` lines 30–35 and the `gate:` job | — |
| §7.7, §7.8, §7.9, §7.10, §7.11, §7.13 | audit `issues: write` unused; fuzz crash artifact; demo VHS unpinned; rustup pin; npm dependabot; dead `checks: write` | PENDING (minor; §7.7/7.9/7.11 confirmed unfixed) | ✓code `audit.yml`, `demo.yml`, `dependabot.yml` | fold into 0056 F0 CI collapse, or drop |
| §7.12, R4 verified installer/self-update | Fail-closed verified install path | PARTIAL: embedded checksum/manifest admission in `self_update/verified.rs`; explicitly "never a signature verifier"; releases unsigned | ✓code `self_update/installer.sh`, `verified.rs` | ring 2/3 (keep checksum); drop signing (0056 §11) |
| R4 release evidence manifest/receipts | Release-scoped assurance record | LANDED (0.45–0.51 receipts) → SUPERSEDED-BY-0056 | `verification/reports/`, release JSON assets | — |
| R7 SECURITY.md / private reporting / revocation policy | Governance surface | **PENDING** | ✓code: no `SECURITY*` anywhere; `.github/` holds only dependabot and workflows | product (cheap) |
| R7 canonical site / current docs | Manifest-driven canonical site | LANDED (gripsack.dev from gripsack-dev.github.io, PR26/PR27) | 0055 | — |
| §8 E1, E2 | Homebrew tap/cask | MOOT-RETIRED (Homebrew) | — | — |
| §8 E3, E4, E5, E6, E8, E9 | Dotfiles SDK pin; python tombstones; gripfetch-apt README; griplint-conformance choker; org profile; reverse conformance gate in core CI | UNVERIFIED / likely PENDING (other repos; not inspected) | 0048 §8 | product/ecosystem; owner decides |
| §8 E7 | Canonical website | LANDED | — | — |
| H0-01, H0-02, A1-07, G-01–G-08, 39 formal catalogs, `delivery.json`, `delivery_*` scripts | Inventory/closure machinery | SUPERSEDED-BY-0056 (deleted in F0) | 0056 §8 | — |
| 0048 §10 quality contract | Domain types, cohesive crates | SUPERSEDED-BY-0056 F2/F4 | — | — |
| 0053 Buildbot, local CI, Seagate restore, HP-mini signer, off-device restore | CI and release infrastructure | SUPERSEDED-BY-0056 §7 (release = green CI on tag) | 0053, 0054 | drop |

### 2.10 Formal programme (0048 §6)

| ID | Description | Status | Evidence | Home |
|---|---|---|---|---|
| §6 fixes 1–5 | Merge `expect`, mutant attribution, per-module floors, OWNERSHIP/GC mutants, `Transaction.tla` stale arms | LANDED | ✓code: no `from_utf8(bytes).expect` in `managed_blocks`; `ASSUME PREV # TARGET` in `specs/Transaction.tla:25` | tooling retires with Verus |
| M-V1 | TLAPS Transaction pilot (301/301) | LANDED → SUPERSEDED-BY-0056 | `specs/TransactionProofs.tla` | ring 1 (P1–P3 in Lean) |
| M-V2 | Loom scheduler seam | MOOT-RETIRED (scheduler deleted) | — | — |
| M-V3 | Merge-scanner Layer-2 (MERGE-SCAN-001) | LANDED (Verus) → SUPERSEDED-BY-0056 | `gripsack-policy/src/merge/` | ring 0 (K5) |
| M-V4 | Journal/marker byte-admission properties | LANDED | `journal/admission_tests.rs`, `scripts/check_journal_admission.py` | keep as ring-2 decoder property tests |
| M-V5 | GC-root/build-closure integration property | LANDED (0.43) → SUPERSEDED-BY-0056 | M-V5.1–5.4 rows | ring 0 (K4) |
| M-V6 | Generalized recovery theorem (40 modules / 4,623 obligations) | LANDED but flaky in CI (prover timeouts); no formal gate in any release since 0.48 | `specs/GeneralRecoveryProofs.tla`, `specs/recovery-proofs.json` | ring 1 (P2) |
| M-V7 | Verus journal/process/update/acquisition kernels | LANDED as checked kernels; "full qualification" never closed → SUPERSEDED-BY-0056 | `gripsack-policy/src/{journal_protocol,process_budget,update_survey,...}` | journal → ring 1 (P1); budgets → ring 2 |
| M-V8 | Aeneas/Lean experiment | Became 0056 F1 pilot | 0056 §9 | ring 0 |
| R2 typed policy admission (F-02) | Typed roles instead of string/bool arguments for ownership planning | **PARTIAL**: retention is typed (`GenerationInventory`), ownership is not | ✓code `ownership.rs:37` `plan_copy(&str, Option<&str>, Option<(&str,bool)>, bool)`, `plan_link(bool,bool,bool,bool)` | ring 0 (K1, F2) |
| Liveness / "eventually recovers" | Claim-gated | Not started | 0056 §5 keeps it TLC-only | drop |

### 2.11 Platform, BuildKit, Homebrew (retired)

| ID(s) | Description | Status | Evidence |
|---|---|---|---|
| B0-01 | BuildKit/Go-LLB harness (the only historically verified row) | MOOT-RETIRED (historical) | PRODUCT-LINUX-NATIVE-2026-10-28 |
| B0-02–B6-05 (40 rows) | BuildKit workers, LLB lowering, Cargo helper, OCI image production, remote/cross builds | MOOT-RETIRED (0055; harness deleted in 0.51) | ✓code: only 3 incidental `buildkit` mentions left in `crates/` |
| A0-01, A4-01–03 | Homebrew bottle selection and closure | MOOT-RETIRED | ✓code: no brew/bottle code outside a schema test |
| A7-01–02 | Source-to-Conda production | MOOT-RETIRED | — |
| Mac/launchd/Lima/ARM/native Windows lanes, Mac persistence partitions, R1 Mac seatbelt, Darwin process fixtures | — | MOOT-RETIRED (PLATFORM-LINUX-WSL-2026-10-08, PLATFORM-LINUX-X64-2026-10-09) | 0048 §9 |
| OCI transport and running existing images | Kept, distinct from image production | LANDED (registry transport, read-only historical image receipts) | ✓code `exec/workspace/image/mod.rs` ("no image authoring backend exists") |

## 3. PENDING items for an explicit owner keep/drop decision (ranked by user impact)

1. **§4.6 recovery wedge.** If a directory appears at an absent-prior destination mid-crash, every apply, rollback and GC is blocked, and the only way out is deleting journal files by hand. Suggest **keep**: put the recovery decision in ring 1 (P2 already covers "never a third state") and add read-only doctor inspection plus a documented repair procedure.
2. **File→directory replacement is refused (0055 gap).** Users hit this as a hard refusal when a managed file becomes a directory without a receipt. Suggest **keep** as a ring-0 ownership decision (K1/K2).
3. **§4.3 staging residue and §4.5 `create_temp` unlink.** These are a disk leak after crashes and a small clobber race. Both are cheap. Suggest **keep** in ring 2 (`gripsack-host`), with §4.3 cleanup covered by P3.
4. **R5 trace logs.** `$GRIPSACK_HOME/runs` grows without bound and its files use umask permissions, while the logs can contain paths and destinations. Suggest **keep** in ring 2 (private mode and a retention budget).
5. **R8 `grip update --json`.** Scripting and CI users need it. Suggest **keep** as product work.
6. **R7 `SECURITY.md` and private reporting policy.** This is a governance gap and costs very little. Suggest **keep**.
7. **R2 typed ownership roles.** Suggest **keep**, folded into F2 (the K1 kernel signature).
8. **D4 substitution before build.** Builds were retired, so this probably no longer applies. Suggest **drop** unless recipes return.
9. **A5-02 rebuild/selection explanations and A5-03 dogfooding.** Product nice-to-haves; owner's call.
10. **R45-18 automated mutable-overlay snapshot.** A documented manual workflow exists. Suggest **drop or defer**.
11. **Ecosystem repos (§8 E3–E6, E8, E9).** These are external repos I did not check. Suggest a one-pass check, then archive or tombstone.
12. **Minor CI hygiene (§7.7, §7.9, §7.11, etc.).** Fold into the F0 CI collapse.
13. **Signing (R4), VM power-loss (R6), disruptive timing campaigns (E5-01), liveness, containment tiers.** Suggest **drop**, per 0056 §11.

## 4. Confidence

- **Checked in code at `187d741`:** §1.1, §1.2, §1.3 (diagnostic codes, `host_name.rs`), §2.1 (`UnsafeArchive`), §2.2 throttle, the R1 artifacts and Landlock, §4.2, §4.3 (still pending), §4.4, §4.5 (still pending), §4.6 (doctor has no journal inspection), R3 activation ledger and the `hooks` command, R5 trace (pending), R8 (no `--json`), R2 signatures (partial), the R4 verifier (no signatures), R7 (no `SECURITY.md`), §6 fixes 1 and 5, §7.4/7.5 (fixed) and §7.7/7.9/7.11 (unfixed), the §3.1/M-V2 moot status (no worker threads, no loom), presence of the E/C/D/services modules and CLI commands, the SDK export surface, absence of BuildKit/Homebrew code, and absence of substitution code.
- **Taken from plan text and release records, not re-run:** whether shipped features meet every conjunctive case in their edition-5 rows (E/C/D/A completeness, C2 impact depth, A2-04 relocatable identity, E4-03/04). The real-journey claims come from the 0055 0.51 records. Also taken from text: proof counts and states for M-V1/M-V3–M-V7, §4.1, §1.4, M0 commit IDs, R45 item details, R6 partition coverage, and §7.3.
- **Not verified at all:** the §8 ecosystem repos (dotfiles, gripfetch-apt, griplint-conformance, org profile).
- **Caveat on the ledger.** The `delivery.json` row statuses (157 pending) are stale and should not be read as missing implementation; 0054 and 0055 say this explicitly. Plan 0056 deletes the ledger in F0, so this table, not the ledger, should be the input to 0056's handover reconciliation.
