# STATUS — the ledger: what landed, what's deferred, what's rejected

One row per plan. "Deferred" items live on the
[roadmap](https://gripsack.dev/docs/roadmap.html); "rejected" is
settled (don't relitigate without new evidence). Update this file in
the same PR as the plan it tracks (convention since 0036).

## Foundations (0001–0019)

| Plan | Title | Shipped |
|---|---|---|
| 0001 | Architecture: modules, store, generations, ownership, invariants | 0.x foundation |
| 0002 | Sourcing: resolver/transport split, fetcher protocol | foundation |
| 0003 | Repo, gates, releases | foundation |
| 0004 | Rich IR + compiler passes + provenance | foundation |
| 0005 | Frontends + configuration | foundation |
| 0006 | Gradual migration | foundation |
| 0007 | Steps, resources, scheduling | foundation |
| 0008 | Canonicalization + satisfaction | foundation |
| 0009 | Diagnostics across boundaries | foundation |
| 0010/0011/0012 | Plugin provisioning → validation plugins → linters in core | 0.13.0–0.14.0 |
| 0013 | Constrained evaluation (sandboxed Deno, trust gate) | 0.15.x |
| 0014 | Content-addressed fetches | 0.16.x |
| 0015 | `grip adopt` | 0.16.x |
| 0016 | Platform facts, floating git, read-only store | 0.16.x |
| 0017 | Hardening/fuzzing playbook | process |
| 0018 | Critique response 0.17.14 | 0.17.14 |
| 0019 | Deploy journal (crash recovery) | 0.18.x |

Deferred from this era (on the roadmap): resolver executables (0013
D8), rollback adapters, module env inheritance for dependents,
secrets model, more probe kinds, more reference fetchers; the LSP and
the fetcher registry stay the north star.

## The audit/review era (0020–0036)

| Plan | Title | Release | Landed | Deferred → roadmap | Rejected |
|---|---|---|---|---|---|
| 0020 | Review response 0.18.1 | 0.19.x–0.20.0 | journal run marker, quarantine, durable cleanup, plan reversibility labels, macOS CI + attestations | SBOM (0022), cap-std (0021), signed channel manifest | cap-std piecemeal, most product proposals |
| 0021 | cap-std fs hardening | 0.21.0 | all five phases (cap-std Dir capabilities, fd-relative writes) | non-UTF-8 symlink targets end-to-end | — |
| 0022 | SBOM cargo-auditable | 0.21.x | in-binary SBOM, release verification | CycloneDX sidecar (when a user asks) | sidecar-only SBOM |
| 0023 | Case-fold everywhere | 0.26.0 | E111 case-folds on every host (stashed → landed with 0030) | — | — |
| 0024 | Review response 0.21.0 | 0.21.x | migration-report fixes | one-commit release modules (carried) | — |
| 0025 | Transaction coverage | 0.22.0 | rollback/prune/env-profile transactions, run-level compensation | the full kill-point matrix | — |
| 0026 | Path-centric transactions | 0.23.0 | generation identity, exact txn identity, intent recorded pre-mutation, mode-preserving writes | `--force` drift overwrite (owner decision pending), mode-in-identity (→ 0031) | — |
| 0027 | Provable transactions | 0.24.0 | postcondition verification, preflight, validated generations | persistent fuzz harnesses in-repo | — |
| 0028 | Machine-checked model | (no release) | Transaction.tla + TLC gate, Rust explorer driving shipped classify/decide, mutation calibration | — | — |
| 0029 | Ownership lineage | 0.25.0 | origin rides the epoch, observed ≠ authorized, lineage explorer | — | — |
| 0030 | Canonical destinations | 0.26.0 | E119 aliases, single-observation deploys, lineage retention on re-take-over, exact removal guard, legacy-marker refusal (later purged, 0035) | activation hooks (→ 0032), grip resolve, CAS displacement, merge aggregation | TOML frontend (5th time) |
| 0031 | Mode-aware identity | 0.27.0 | full mode in identity, chmod drift detection, exact mode restore, deterministic landing modes | — | — |
| 0032 | Durable activation | 0.28.0 | pre-flip pending record, resume/discard, Activation.tla + TLC, idempotency contract | rollback activation (fires intents on rollback) | — |
| 0033 | Review 0.27.0 | 0.29.0 | take-over keeps private modes, prior store 0600/0700, pin-grant validation, step needs ordering (E120), plan runs full gates, preserved-drift blocks mode switch, adopt codegen sanitizes | shared op list (→ 0034), fetch memory budget | machine-local pin setting; "plan predicts external state" |
| 0034 | Shared operation list | 0.30.0 | one planner for plan/apply/rollback; ops/{ISA, codegen, VM, preview}; preview is honest about drift | — | a new TLA+ spec for the op list (the protocol models already cover it) |
| 0034+ | VM-level op harness | 0.31.0 | 560 materialized cases: planner ≡ algebra, execution lands intent | — | — |
| 0035 | Review fb11aaf | 0.32.0 | canonical DestinationKey in manifests, verify receipts, strict TS fields, dep-pin build keys, closed txn boundary, vendored frontend (cargo install works), read-only accurate preview, normalized step graph, on_remove fires, reserved GRIPSACK_*, recursive fsync publish | build closures, rollback activation, streaming/memory budget, persistence-fault evidence | SQLite, rewrites, machine-local pin setting |
| 0036 | Representation audit | (docs only) | the typed/bare ledger + the convention (typed producers, plain wire, explicit seams) | GenerationId newtype (with the revisit trigger: a second u64 domain sharing a signature) | — |
| 0037 | Rollback activation | 0.33.0 | rollback runs the target generation's intents (durable, crash-resumable via 0032's record); rollback-undeclared modules fire on_remove | per-entry intent diffing (if anyone asks) | — |
| 0039 | Build closures | core/TS 0.35.0 | IR v2 `Dependency.for`; store-only build deps, graph-ordered PATH + GRIP_DEP_*, payload receipts, same-run dependency pins, generation-pinned GC, zero-destination op case + seeded journeys | library/header exports and controlled PATH (low priority, real-consumer/opt-in triggers); general env inheritance retains its existing roadmap priority | speculative verify/provisioning edge types; extra closure Op kind (marker suffices) |

| 0038 | Journey harness | 0.34.0 | seeded random journeys + expectation model + system oracles; caught the preserved-verify bug on day one | wider world (more module kinds, parallel-applies stay TLA+'s) | — |
| 0040 | Post-0.35 project sweep | review | boundary review + 13 isolated CLI probes; P1s implemented by 0041, P2s by 0042 | independent audit and separate product/trust decisions remain prioritized | broad rewrites without a concrete consumer |
| 0041 | P1 contract fidelity and persistence | core/TS 0.36.0 | fail-closed GC; one prepared view; ordering-only needs; output contracts; IR v3 removes inert retries; concrete preview; permission receipts and exact rollback; exhaustive recorded-cut matrix + ordering calibration; cohesive module/test splits | remaining 0040 P2s → 0042 | global step VM and partial retry engine |
| 0042 | Bounded acquisition, complete pins and stronger evidence | core/TS 0.37.0 | streaming verified acquisition; bounded process supervision; command-local client/recipe reuse; causal logs; update-time source pin completion including pixi; strict executable contracts; persistent sandboxed fuzzing; repeated recovery and calibrated protocol models; pinned inputs and durable concurrent self-update | automatic install-time provenance and external audit remain separate trust/evidence decisions | payload-sized buffering, shadow schema validators, silent cleanup success |
| 0043 | Migration feedback and permission identity | core/TS 0.38.0 | executable, mode-identified templates; merge mode receipts/markers and guarded prune; installed-frontend doctor; non-publishing update --check; researched tuicr 0.20–0.25 pack; calibrated FileMode model and real planner/executor explorer | — | template in-file banners (would alter rendered formats); universal correctness claims from abstract protocol models |
| 0044 | Complete surveys, explicit version spellings and bounded HTTP failures | core/SDK 0.39.0 | lossless all-block merge boundaries; complete survey/error precedence; shared bare-version expansion and staged preflight; host-bound auth diagnostics; bounded GET retries, cooldowns and redacted attempt evidence; four calibrated TLA+ models and real-code bridges | opt-in host-scoped gh credential provider; trustworthy read-only recipe-layout evidence | implicit base-URL credential grants; changing existing version token meaning; heuristic archive-layout warnings; automatic shell profile sourcing |

| 0045 | Recovery admission hardening, mutation sessions, editor-reachable frontends | core/SDK 0.40.0 | tagged + versioned journal identities with fail-closed legacy quarantine; required previous_generation marker parse; GC refuses unfinished recovery (dry-run included); LifecycleSession binds lock+home for gc/rollback/store-repair; types resolve from frontend src + frontend/current link + doctor editor hint (npm keeps dist/ as the runtime entry — Deno never type-strips under a real node_modules); guarantee ledger opened | Verus classifier pilot → GC planner proof → ownership proofs → structural op contracts (all landed in 0046); Lean/Aeneas and TLAPS stay later; classify-aware precise GC admission; frontend/ts-* cleanup | shipping compiled dist in the embedded frontend (second unexecuted artifact form); guessing legacy journal variants; replacing explorers/TLA+ with proofs of abstractions |

| 0046 | Verified decision kernels (Verus pilot + ownership + GC retention + op contracts) | core/SDK 0.41.0 | gripsack-policy crate; classifier/ownership/retention kernels proved (biconditional contracts, mutation-calibrated runner, obligation floor); verify compose gate in the required CI job; structural Op contracts (RemoveTarget in-variant, ExecutableOp boundary); proof erasure on plain cargo + musl | merge-splice, scheduler and HTTP-budget kernels (roadmap ROI order); Lean/Aeneas and TLAPS studies stay later; precise classify-aware GC admission; root-completeness proof at manifest production | a parallel verification reimplementation; feature-flagged algorithm swaps; verified wrappers over unverified bodies |

| 0047 | Verified merge splice, graph closures and scheduler transitions | core/SDK 0.42.0 | merge splice kernel over real bytes (spec-mirror contract, identity + gaps lemmas); build-closure/build-only graph kernels with soundness+completeness over cycles; PureScheduler transition system proved (exact readiness, at-most-once start, failure latch) and WIRED into production run_all — scheduling decisions are the proved kernel; four-mutant calibration (classifier, splice-tail, closure-membership, scheduler-latch); obligation floor 50; runtime smoke tests pin the erased-ghost runtime contract; solver lessons recorded (implies-vs-==>, ghost snapshots, cast-collapse lemma, loop-carried facts) | Follow-on structural scanner proof is now 0048 M-V3 / MERGE-SCAN-001; the Transaction.tla induction pilot is 0048 M-V1, not generalized M-V6. Lexical/OS correspondence and other 0048 release obligations remain distinct. Lean/Aeneas remains exploratory | verified wrappers over the threaded bridge (the bridge stays tested, never claimed verified) |

| 0048 | Unified review handoff with binding release and quality gates | (pending) | §14 records partial NEXT/M0: typed `HostName`/E132 (`681e87b`), bounded frontend/pre-effect adopt trust (`0b92905`, 12/12), comma-safe Deno grants/E133 (`953724a`, source-bound 18/18), repo-env/credential/TLS cutover (`1c693ef`, source-bound Rust/real CLI **12/12**, TLC credential positive/three mutants **4/4**, fingerprint `9a82d56b48bef69d8a57c831a3efa16638123279150689d277ecdd8a81447825`), and archive link-graph containment (§2.1 leaves M0-2.1a/b at `db1e91b`: shared order-independent bounded resolver for TAR/ZIP/`validate_tree`/`copy_tree_filtered`, cap-std root-pinned materialization, both original composed-link fixtures reject `UnsafeArchive` after failing-before, fuzz corpus seeds added, six gates + fuzz replay green; receipt `2026-09-26-m0-archive-links-db1e91b.log`, implemented-unverified). Old repo PATH selected `ldd`; repo `GH_HOST` sent an operator dummy token over HTTP. Current code detects facts before build env, rejects operator credential audience/tokens E400 terminal/JSON, scopes child proxy/CA and refuses cleartext bearer. Exact-source manual CI [`36227812302`](https://github.com/gripsack-dev/gripsack/actions/runs/36227812302) at `1c693ef` passed Linux test, native macOS arm64 full e2e **319/319**, audit/fuzz/docs. The later `52943b2` checker-source CI [`36230541824`](https://github.com/gripsack-dev/gripsack/actions/runs/36230541824) failed required Rust `test` with `ETXTBSY` from an in-place rewritten self-update test script; production process errors were not suppressed. `70c5502` stages/closes/renames that fixture before execution, and exact-source manual CI [`36231520581`](https://github.com/gripsack-dev/gripsack/actions/runs/36231520581) then passed required Linux test (delivery checker **24/24**, Rust/TS/e2e/TLC/Verus), native macOS 14.8.9 arm64 **319/319**, audit, fuzz and docs; native job log `verification/reports/2026-09-26-h0-macos-ci-70c5502.log`. Manual dispatch does not enforce branch protection or qualify Mac-VM/launchd/R5/M-V7. Isolated RUSTSEC-2026-0285 rustls 0.23.45 patch merged on protected main via PR #165 (`db32f20`), but optional external TypeScript example still fails on unpinned Pixi ripgrep (A3 pending). Draft PR #164 remains nonmergeable: R1–R8/M-V1–M-V7, H0/global, native Mac-VM, A3 and actual protected completion remain open | NEXT / M0–M2: boundary fixes, typed policies/refinement, hooks, release/assurance/install/public contracts and M-V1–M-V7; M3/M4 and claim-gated VM/platform evidence remain required; M-V8 cannot waive NEXT; release blocked without owner decision | no unilateral deferral, fabricated containment/proof, evidence substitution, partial completion claim, unreviewable module/type debt or publication without applicable gates |
| 0049 | Handover bundle import (edition 5) — H0 reconciliation + A0 bottle selection | (pending) | Eight immutable bundle checksums verified; **178/178** protected IDs/fields retain platform/capability lanes, conjunctive case/proof inventories and runner/formal/review kinds. Structural `2026-09-26-h0-inventory-953724a.log` proves IDs/fields/checksums, **not** complete 178-case semantics. A1-07's checker requires each evidence kind per claimed lane, five review-only rows need no fake runner, formal proof catalogs bind row/minimum/milestone/report bytes, and count digits in SHA hashes cannot impersonate executed counts. An optional per-lane case map must partition the full case union across **all** lanes: systemd success cannot cover positive native launchd cases. Active direct H0 receipt `2026-09-26-h0-catalog-ce3c7e0-e2-b0-ledger.log` at evidence-only `143f6d5`/behavior source `ce3c7e0` has **30** negative calibrations and synthetic H0/A0/E0-lane/review-only positives; actual H0 closure fails. All **39** real formal rows lack reviewed proof catalogs and H0-02 remains `in_progress`; eight targeted A1/E0/E2 inventory repairs are not review of all 178 paraphrases. Statuses after source `80b50a3`: **158 pending, 13 in_progress, five implemented_unverified, one blocked, one historically verified (B0-01 qualification at `ce3c7e0`)**; **H0/A0 claims retracted**. External TS example still fails unpinned Pixi ripgrep (A3-01/A3-02, never repin observed bytes blindly) | B0-01's historic real Linux and required-CI qualification lanes were verified at `ce3c7e0`, but the rotated worker manifest and current source need renewed protected CI/negative evidence; full B0/E0, A1, proof inventories and plan/0048 NEXT remain required before public release; native Mac-VM/launchd, registry/recipient and formal lanes need independent evidence |
| 0050 | E0 qualification — task/schedule scope, vocabulary, proof targets | (pending) | E0-01 records source→A1/A2-P mappings, eight scenario families, frozen failure/skip/unknown vocabulary, budgets and E1/E3 proof targets. E0-02's `systemd-linux` lane has exact-source `ce3c7e0` real systemd 255 user-manager evidence: UID/user scope, daily normalization, OnCalendar trigger within 0.995 s and zero-residue cleanup bound to `verification/reports/2026-09-26-e0-systemd-ce3c7e0.log`. Native launchd cases remain named separately and blocked without a Mac, so E0-02 row and E0 milestone remain open; no E3 Gripsack scheduler or sleep/reboot/DST inference | E1 gated on A1/A2/A2-P; actual launchd user-agent and future TLAPS proof infrastructure required |
| 0051 | B0 qualification — BuildKit harness + integration inventory | (pending) | B0-04 records LLB field matrix, bridge fences, proof mappings, emitted-LLB witness and scheduler migration. Source `ce3c7e0` ran **6/6** real Linux/amd64 pinned Go/BuildKit cases: graph reuse/cancel/failure, declared-input policy, surviving executable, two independently verified clean OCI exports and a separate Docker engine running their verified content. Source-bound `verification/reports/2026-09-26-b0-linux-ce3c7e0.log` plus three real-worker negatives and four loaded-image substitutions check native builder admission, blob hashes, Docker archive load, loaded tag/platform/Env/exact DiffIDs. Docker 28 CI rejected a pure OCI tar as `/blobs/json` (failed required runs `0be1eaa`/`fc67212`); the independent verifier now repackages **the same checked bytes** in a Docker-save archive and compares effective loaded semantics, not raw Docker config ID. The **full required PR `test` job** at `ce3c7e0` passed B0 **6/6** on Docker28, real CLI e2e **319/319**, TLC and Verus **72/0** plus seven mutants; exact job bytes: `verification/reports/2026-09-26-b0-required-ci-ce3c7e0.log` (SHA-256 `50b48797ce77092a311188b687d18943515a37945b6fe08fdbf6fb00b64815e3`). **Only B0-01** is verified in `linux-amd64` and `container-gates`; full B0 remains pending. B1-02/04 kernels landed at `80b7738`: the pure worker lease transition table + owned-only CleanupSet (a live lease blocks stop; crashes keep leases and block recovery until explicit drain) with `specs/WorkerLease.tla` in the model gate — positive two-client crash model clean, early-stop and crash-wipes mutants violate exactly their named invariants (the mutant caught a vacuous TLC `EXCEPT` draft before it could count); bridge gate now race-enabled (receipt `2026-09-26-b1-worker-leases-80b7738.log`, six gates green). B1-03 transport landed at `50b829c`: the bridge speaks stdio frames with a deliberately FAIL-CLOSED Submit (no client linked — bounded log + terminal Failed, never fake success) and the Rust core drives real sessions (`transport::BridgeProcess`) to exactly one fenced terminal; a cross-process Rust↔Go e2e (ignored test, explicit run) proves negotiate/submit-fail/cancel against the compiled bridge — receipt `2026-09-26-b1-transport-50b829c.log`, six gates green. Earlier at `1d43f0d`: the production Go bridge began (`tools/buildkit-bridge`, package `protocol`) mirroring the exact Rust contract — strict framed decoding, 256 KiB header cap in both DecodeFrame and streaming ReadFrame, cap-checked base64 log chunks, exact-pair CheckNegotiation and the EventGate port — with the fuzz corpus doubling as a two-sided conformance corpus (valid seeds decode and hostile seeds reject on both sides; receipt `2026-09-26-b1-bridge-go-d0d0a4a.log`). Rust crate `gripsack-buildkit` landed at `d656075` — the bounded framed Rust↔Go wire contract (header-checked 256 KiB cap, strict tagged shapes, digest-bound Submit, 64 KiB log chunks, exact-pair negotiation) plus the pure `EventGate` fence kernel (epoch fencing, one terminal, duplicate-terminal and log-budget rejection), a registered fuzz target with 7 seeds and six gates green (receipt `2026-09-26-b1-protocol-d656075.log`); no Go bridge/transport/worker speaks it yet. B0-03 footprint partial; no production BuildKit backend wired into `grip` | B0-02 Apple Silicon Mac VM blocked (no hardware); B1 worker/bridge and B2 lowering not landed; never infer Mac VM or full B0 from Linux/hosted native Mac e2e |
| 0052 | A1 workspace contract: v4 history, v5 typed admission and partial graph/command/diagnostic/file gates | (pending) | Strict v3 modules and read-only v4 workspaces retained. Current v5 schema/Rust/TS admits nine named output kinds, typed targets/layouts, graph roles, fluent/object commands and source-mapped Bash; unsupported execution fails E124 before activation. The E131 adapter rejects missing/substituted source edges, wrong reference sites and catalog indices; `WORKSPACE-INDEX-001` pointwise binds candidate names, and `WORKSPACE-TARGET-001` proves typed target comparison, **not** full serde/source-edge/IR-enum refinement. A1-06's manifest generates registry-backed Rust codes, five frontend constants and a static membership Record; unallocated E999 remains a traceback, malformed spans get E129 labels, E124 owner diagnostics prefer the first declared capability and logs cannot pollute `check --json` stdout. `runBash` statically requires a package-command ref but resolved interpreter bytes/options await A1-05. A1-08 registers common commands, E128 host-shell refusal and inert schedule/prerequisite owners; identity/context proof remains open. A1-11 admits composed source/content/destination values, rejects destination escapes E102 and duplicate physical destinations E111. Source `d946d30` rejects `repo_file` parent/absolute/ambiguous paths in v5 schema, TypeScript and decoded Rust E130 at the file span before E124 (receipt `2026-09-26-a1-repo-file-d946d30.log`, five focused groups + five gates). Source `d9ce200` lands eval-time `treeFiles(src, to, {include, exclude, mode, maxEntries})`: stable sorted expansion of a captured repo directory into ordinary v5 per-file entries with per-entry spans, segment-boundary include/exclude, eager destination validation, a 10 000-entry cap and symlink/special rejection without following — no wire or version change (decision in §30.2; `pin.ts` re-exports the helper after a failing-before authored import). Receipt `2026-09-26-a1-treefiles-d9ce200.log`; five gates on identical bytes: fresh Rust, Deno **67/67**, e2e **321/321** with the new authored case, TLC, Verus **72/0**+7 mutants — not A1 proofs. Source `b0861eb` makes managed blocks per-marker owners: destination admission folds by case-folded path (whole-file policies still reject co-declarations) then by case-folded marker, so distinct markers over one host file coexist (failing-before: always E111) while a duplicate case-variant marker rejects labeling both declarations (receipt `2026-09-26-a1-block-markers-b0861eb.log`; five gates, e2e **322/322**). The later native-file continuation now prepares repo/literal/template content and executes symlink, tracked-copy and managed-block file-only profiles through existing generations, drift handling, rollback and prune (see 0048 §14 and the continuation below). Artifact/package/environment/task realization, artifact-side trees, staged lint/check mapping and owner/refinement proofs remain open; earlier A1 source-bound receipts remain historical. Portable locks/pins, identity/context proofs, protected exact-head CI and native Mac still block full workspace/release closure | A1-01 implemented_unverified; A1-02/A1-03/A1-06/A1-07/A1-08/A1-10/A1-11 in_progress; A1-04/05/09/12 and plan/0048 NEXT open |

**0048 R1 continuation (2026-09-29):** Source/policy-bound approval, copied
read-only roots, retained runtime selection, immutable per-round inputs and
private evaluation receipts are implemented but not fully qualified. The
development real-Deno group passed 63 cases, including six controlled
repo/pin/link mutation pauses; one invalid operator-config fixture key was
corrected after its setup failure. Native source aliases initially staged as
dangling links; resolved captured-object materialization now passes the actual
apply/live-edit witness. Adopt has explicit generated-source renewal/resume
and consumes the same outcome for preview/apply. Schema/read-fault/alias-form,
whole-suite, demos/examples and native/exact-source qualification remain open.
Website migration changes live separately on `handover/source-approval-docs`.

**0048 R1 Mac ARM qualification (2026-10-01):** A literal root-directory
Seatbelt grant fixes platform-launcher SIGABRT without recursive read access.
Private source captures are named before read-only sealing; canonical identity
and declared diagnostic/native-path spelling are distinct. Diagnostic source
`e7841e3` passed **5/5 source-capture + 60/60 real-Deno** cases on native ARM64
([CI](https://github.com/gripsack-dev/gripsack/actions/runs/36923313515)); the
preceding native process/journal authority job also passed. Complete candidate
CI and persistence qualification remain open. The positive TLAPS timeout from
run `36906254878` is repaired by proof decomposition: fresh two-CPU `bfa9468`
qualification passed **4,645 generalized + 301 pilot obligations** and all
calibrations with unchanged theorem, catalog floors and timeout policy.
Generated prover output is now ignored and archived logs marked generated;
checksum-bound historical evidence remains intact rather than being silently
deleted or counted as current release qualification.

**0051 source update (`80b50a3`, 2026-09-26):** The retired
BuildKit v0.33.0 manifest-list digest was replaced by a resolvable
digest with the same qualified amd64 leaf. The pinned Go bridge was
rebuilt; real Linux B0 probes passed **6/6**. The isolated
`gripsack-buildkit::worker::linux` provider now provisions a labelled
worker on a local Unix Docker daemon, guards deletion by the
immutable container ID and owner labels, refuses foreign cache
volumes, retains disk cache on idle stop and explicitly tears it
down. Its **2/2 real Docker** and **7/7 unit** cases passed. Five
non-fuzz container gates passed (Rust fresh, TS **cached**, real CLI
**322/322**, TLC worker-lease mutants, Verus **72/0** + seven
pre-existing policy mutants). Exact source and observed scope:
`verification/reports/2026-09-26-b1-linux-80b50a3.log`,
SHA-256 `0769f8a44950e4c6dc22b8bd74aba5f6135ca1cfe5d45e402b96b14631968e0a`.
`B1-01` and `B1-02` are now `in_progress`, **not verified**;
no `grip` caller, Mac VM, private socket, full capability handshake,
BuildKit client/LLB lowering, production output/store export or
new protected Docker28 CI exists. Fuzz was not run per owner request;
the existing pull-request CI would replay fuzz, so no public release
or website/config adoption claim is authorized by this packet.

**2026-09-27 working-tree continuation:** native v5 file profiles now use
the existing preparation/store/ownership/journal/generation lifecycle.
Real CLI flows exercised all three destination policies, retained-byte
rollback, scoped selection, two-block pruning and crash recovery.
Journal v2 preserves the run-original prior plus the immediate-before
state; the fresh TLC gate passed both loss mutants and the two-write
non-vacuity witness. Scheduler panic completion, typed/capability-backed
priors, private capability-backed journals, complete update-pin
comparison, E109/E110/E115/E134 admission and named Verus attribution
are implemented. All five local non-fuzz gates passed: Rust plus Loom
calibration, TypeScript **67/67**, real CLI **342/342**, fresh TLC and
Verus **72/0** plus nine semantic mutants/unrelated-failure refusal.
`verification/reports/2026-09-27-native-files-local-gates.log` archives
the actual outputs. `plan/0048` and `plan/0052` retain the leaf/scope
record. These are working-tree observations; exact-source release
evidence and all remaining implementation/proofs/platform lanes stay
open. Fuzz remains unrun this round by owner instruction.

**M-V1 local induction continuation:** `TransactionProofs.tla` now proves
the corrected one-destination/one-recovery transaction pilot:
**301/301** fresh TLAPS obligations, plus a **13/13** checked bad-barrier
transition and a reachable TLC `Oracle` failure. The runner rejects
unrelated-failure and empty-target evidence and never counts timeout
as semantic calibration. A byte-pinned `tlaps` Compose service joins
the required CI `test` job; no CI workflow was dispatched this round.
Six local non-fuzz gates passed, including real CLI **342/342**.
Fresh committed-source receipt:
`verification/reports/2026-09-27-m-v1-e92f5ad.log` at `e92f5ad`,
with clean source roots before/after (SHA-256 in the report index).
M-V6 generalization, other NEXT work and release qualification remain
open; the pilot makes no fresh-None, repeated-recovery, multi-destination,
Rust/OS refinement, liveness or hardware claim.

**M0 §2.2 continuation:** plugin cache admission now compares source and
tag; validated rate budgets and one absolute capability/admission/exchange
deadline replace panic/unbounded-wait paths. Corrupt saved token balances
and timestamps are admitted without arithmetic panic. Local Rust
process/fetch **90/90**, real CLI **55/55** and six standalone before/after
witnesses passed after correction. The same 90 Rust + 55 CLI cases now
bind exact source `0a960e5` with clean source roots in the report index.
Final local gates passed, including full CLI **348/348** and Verus
**72/0** plus ten calibrations. This does not discharge M-V7 or qualify
a release; protected CI and native-platform evidence remain open.

**M-V3 scanner continuation:** the actual production line walk and range
state machine now live in `gripsack-policy::merge::scanner`; five named
queries prove structural/UTF-8 admission and its splice-predicate bridge.
The local gate passed **80/0** with **11** calibrations, including the
attributed one-byte range-start mutant. Policy/exec **56/56**, real CLI
**32/32** and a standalone Unicode/CRLF update/prune smoke passed. Fresh
proof and runtime evidence now bind `221c6cf` with clean source roots;
all six final local gates passed, including full CLI **349/349**.
See `verification/reports/2026-09-27-m-v3-221c6cf.log` and the report index.
Lexical recognition, OS behavior, protected CI/native-platform qualification
and other NEXT obligations remain distinct; no release is authorized.

**M-V4 byte-admission continuation:** seven deterministic production-parser
and recovery properties cover 24 marker/20 entry round-trips, 29 marker/110
entry rejections, 173 truncated prefixes and 139 real recovery-effect
rejections. The required test gate now calibrates missing-previous→null
against the actual recovery oracle. Ten real transaction flows and a
standalone cold-home missing/null lifecycle passed. This is bounded
property/refinement evidence, not a serde proof. The preserved receipt now
binds source `c92e84e` with clean roots: store **69/69**, the seven-property
campaign/calibration and CLI **10/10**
(`verification/reports/2026-09-27-m-v4-c92e84e.log`). Release/platform
qualification remains open. No fuzz or corpus replay ran.

**M-V5 production-root continuation:** the recorded store-root symlink
failure is corrected: GC pins generation/store/prior directories and
admits all manifests/candidate/size reads before pruning. Typed generation
IDs preserve numeric persisted formats; borrowed/owned inventory admission
and exact oldest-prefix pruning pass the production Verus gate
(**92/0**, **13** attributed calibrations). Real-collector properties cover
**7** retained histories, **8** recovery refusals and **1** root-replacement
interleaving; a dropped production build root fails the independent byte
oracle. Five standalone CLI smoke cases passed, including retained old
current, expired history and generation-ID exhaustion. See `plan/0048`
M-V5.1–4 and the report index for source binding and final gate observations.
M-V6/M-V7, R2 ownership/receipt work, other NEXT leaves and native/protected-CI
release qualification remain open; no fuzz or corpus replay is claimed.

Final M-V5 local gates passed: Rust/Loom/journal/GC, real CLI **362/362**
and Verus **92/0** plus **13** calibrations. Unchanged TS/TLC/TLAPS gate
layers were cached. The report index binds the dirty candidate and records
the three pre-existing ignored BuildKit integration cases separately.
Next ready proof packet is M-V6; this is not a whole-plan or release closure.

**Active release continuation (2026-09-27):** the owner clarified that the
current assistant must implement and land the handover, then cut qualified
release(s). The additional agent handles post-release fuzzing and repairs.
[`verification/release-handoff.md`](../verification/release-handoff.md)
remains a tracking/runbook artifact, not a transfer of unfinished non-fuzz work.
`REL-FUZZ-2026-09-27` covers this continuation's feedback releases only; fuzz
is **not run**, not passed. All other gates remain required. A local explicit
manual-waiver CI path and fail-closed aggregate now pass 4 positive/64 negative
cases plus four CLI smokes. Live PR/protection qualification remains open;
normal PR/main CI still runs fuzz unless skipped deliberately. No release yet.

**Transaction-bound activation continuation (2026-09-27):** new current
selections and markers distinguish repeated rollback transactions from their
generation numbers. Private plans, per-intent Started/terminal outcomes and
archived receipts drive the real apply/rollback/removal runner; `hooks list`
and fixture-only `hooks test` are implemented. A false new-generation delivery
after replay was exposed and fixed. The 43-case focused CLI group, three actual
simulation modes, runnable idempotency examples and the extended calibrated TLC
gate passed locally. Process death and physical power loss remain distinct;
the model is not the generalized M-V6 theorem. Native byte/FD admission caught
and removed `execvp`'s implicit shell fallback; syscall-capability faults and
closed-stdio cases use the actual executor. See the R3 leaf record and
`verification/reports/` for precise evidence/limits. Full candidate/native CI,
M-V6/M-V7 and the remaining handover implementation are still required.

**Durable metadata authority (2026-09-28):** generalized recovery work exposed
four concrete gaps: cached-prior admission, visible directory-birth retry,
retained-prior reads and generation pruning before payload collection. Each
has a retained failing-before witness and a repaired real consumer path.
All six local non-fuzz Compose gates passed, including **405/405** real CLI,
Rust/clippy/tests and current contract calibration. Standalone prior/GC
refusal/retry journeys passed; no physical power-loss claim follows. The
source/image-bound report is `2026-09-28-durable-authority-local.log`.
The committed repairs at `e3738d765025aa5ea352f47d603f83b43760feb5` passed full
CI run `36385223651`: Linux CLI **405/405**, native Mac CLI **403 passed / 2
unconstructible-filename skips**, native process **25/25**, docs, audit and
aggregate. `2026-09-28-ci-e3738d7.log` retains the exact-source evidence;
fuzz/replay alone was owner-waived/not run. Live `gate` protection is separate.
Generalized lifecycle/activation composition, M-V7 and the remaining handover
implementation/release requirements remain open.

**Observed-generation authority (2026-09-28):** the M-V6 publication bridge
exposed a further concrete gap: a retained manifest could authorize committed
cleanup without sealing its artifact/name durability. The shared validator now
separates read-only inspection from effect admission. Apply, rollback, current
flip, recovery, activation and GC use the latter. The focused recovery campaign
passes **19/19**, including five file/directory error/kill boundaries; a separate
real rollback smoke preserves destination/current on failure and succeeds on
clean retry. The complete generalized theorem remains open; the later
`ae77f52` CI receipt below qualifies these repairs rather than reusing the
earlier `e3738d7` result.

The same bridge found a legacy allocation-floor loss: collecting generation 20
without a high-water counter allowed the next apply to allocate 11. Publication
and pruning now preserve/seal the admitted floor before discarding names. The
standalone replay allocates 21; the combined GC/history/recovery group passes
**67/67**, including stale/higher/exhausted counters and counter-barrier failures.
The earlier whole-CLI attempt timed out during the exhaustive persistence
matrix and also exposed two stale fault-location selectors; those selectors now
target the final post-prune barrier, with the original payload/current oracle
unchanged. Its incomplete run is not passing evidence.
The final no-deadline CLI run passed **416/416** with every persistence case.
All six local non-fuzz gates passed; the source/image-bound receipt is
`verification/reports/2026-09-28-generation-authority-local.log` (SHA-256
`7c2e3fd0e1650bcdd926211364e43f8c9c90160ecdf68ae02886e9028d87830a`).
It binds 1123 registered inputs and the actual exercised CLI binary.
Exact source `ae77f524bace1e46b738832fe99f133feb51ed51` then passed CI run
`36468870180`: Linux CLI **416/416**, native Mac CLI **414 passed / 2 skips**,
native process **25/25**, docs, audit and aggregate. The complete archived
receipt is `2026-09-28-ci-ae77f52.log` (SHA-256
`644af54a4bebccbc5b86e0be5bde4cbc781668af29c83e92a35f83147dc3f144`).
Only fuzz/replay was owner-waived/not run. Generalized M-V6/M-V7, live aggregate
protection and the remaining handover/release work remain open.

**Generalized recovery integration (2026-09-29):** M-V6 now has shared
journal/publication/selection/preparation/activation/retention definitions.
Source-matched development runs checked **40 proof modules, 443 named theorems
and 4,542 obligations**. A timed-out coupling obligation was decomposed and
then passed; no timeout is counted as a proof or mutant.
The frozen catalog admits the entire local dependency closure and rejects
missing imported units, wrong theorem names, zero floors, unreachable units
and commented-out proofs. Its actual admission/calibration smoke passed.
`RepeatedRecovery` replaces the obsolete hard-coded `MultiDestination`
engine; seven finite-work completion cases and the constructive missing-restore
barrier calibration passed privately. The real six-destination campaign passed
six consecutive interrupted recoveries, including four SIGKILLs, while
preserving an independent edit and refusing both forms of collection.
All six local Compose gates passed at `8bad070`. Candidate CI `36497472294`
passed Linux CLI **417/417**, native macOS process **25/25** and CLI
**415 passed / 2 unconstructible-filename skips**, but failed a coupling
proof obligation on an internal prover timeout; Verus was not reached.
The same implication was split into epoch identity, preparation binding and
active-plan typing premises; its **101** obligations passed fresh two-CPU,
complete rebuilt local and candidate-CI execution at `b53e334`.
CI `36514136863` then failed a separate invocation theorem (1/71 module
obligations, internal timeout), while Linux CLI **417/417**, native process
**25/25** and native CLI **415 passed / 2 unconstructible-filename skips**
passed. Verus was not reached and the aggregate refused.
The unchanged durable full-selection conclusion now follows separate
projection/typing/binding facts. Fresh two-CPU execution passed **105/105**
module obligations; the catalog floor is **4,589**, still 40 modules / 443 names.
The complete rebuilt local model/TLAPS gate passed at `fd20e09`, including
all **4,589** obligations. Its receipt is
`2026-09-29-m-v6-invocation-local.log`. Candidate CI `36531160370` passed
the coupling/invocation modules, Linux CLI **417/417**, native process **25/25**
and native CLI **415 passed / 2 unconstructible-filename skips**, but failed
`OrdinaryLifecycleKeepsActiveConstructor` on an internal prover timeout.
The unchanged implication now uses explicit ordinary-transition cases;
fresh two-CPU execution passed **97/97** module obligations. The catalog
floor is **4,623**, still 40 modules / 443 names. The complete rebuilt
model/TLAPS gate passed every module and required calibration; see
`2026-09-29-m-v6-constructor-local.log`. Exact-source CI `36557008541`
at `8143443` remains required.
No timeout is release approval. The failure/repair receipts in
`verification/reports/` retain complete logs and the exercised source hashes.
M-V6 release closure, M-V7 and the remaining handover remain open.

**M-V7 acquisition continuation (2026-09-29):** actual token credits now use
full-range finite-rate arithmetic, explicit legacy migration and exact saved
nanoseconds. Monotone operation deadlines constrain process/throttle callers;
timed parking_lot locks and HTTP attempt/completion transitions use the original
budget. Reproduced bugs admitted throttled work after a contended deadline and
returned HTTP body success after expiry. Both have permanent concrete oracles
and failing semantic/effect mutants. Rebuilt local Rust and Verus gates passed:
**244/0**, **32** proof/evidence calibrations, **10** evidence negatives;
**13 rate properties / 6 mutants**, **12 HTTP properties / 4 mutants**.
Three rate and five HTTP real CLI journeys passed at explicitly identified
development snapshots. See `2026-09-29-m-v7-acquisition-local.log`.
This is not exact-candidate/native evidence or whole-M-V7 closure:
journal authority, process frames/counters/lifecycle and update accounting
remain required before release.

**M-V7 process continuation (2026-09-29):** named byte units, one outstanding
input write, framing, cumulative counters, exact retained suffixes and
signal/reap/cleanup states now constrain the actual supervisor. The guard
consumes an owned `Child`, not an arbitrary PID. Whole-policy proof passed
**305/0**; the concrete Linux gate passed **35 positives / 10 mutants**.
Actual faults exposed expired cleanup reported successful, swallowed EPERM
and lost native errno. The corrected deadline/classification/receipt paths
have permanent calibrated oracles. Darwin's zombie-only exception now checks
complete identity-stable group observations; cross-compilation passed before
the final ownership cutover, but native execution remains required.
See `2026-09-29-m-v7-process-development.log`. Full aggregate/new-candidate
qualification, journal authority and complete update accounting remain open.

**M-V7 complete survey continuation (2026-09-29):** legacy modules and native
file profiles now stream into one complete-only report collector. Selected
counts and actual report positions constrain summary construction; failure
dominates changes and CLI/executor share the checked publication decision.
Whole-policy proof passed **320/0**, with **12** named survey queries.
Three production model/filesystem properties and seven seam mutants passed.
Four real grip/Deno journeys preserved Check's lock/cache, failed Publish's
lock, exact current lock bytes and the distinction between layout and native
effects. The no-publication oracle also pins file identity: byte equality
alone missed a same-byte atomic write. See
`2026-09-29-m-v7-update-development.log`. Journal effect authority and
full/new-candidate/native qualification remain required.

**M-V7 journal authority continuation (2026-09-29):** record publication now
uses separate file/name/namespace states; mutation borrows its admitted run,
captured prior and pinned destination parent. Current publication consumes the
run, and cleanup requires committed or reconciled progress. All executor and
fixture consumers migrated without a directory-only compatibility adapter.
The development policy proof passed **333/0** with eight named journal queries;
real grip/Deno apply, update, interrupted rollback, recovery and fresh-selection
rollback passed. Four permission/error oracles plus the actual filesystem-order
oracle passed all six source mutants, including dropped file/parent syncs.
The complete Verus gate passed **333/0**, **60** semantic/evidence calibrations
and **16** evidence negatives. The complete rebuilt Rust gate and **417/417**
CLI suite passed; native/exact-candidate CI remains required. The integrated
local receipts are indexed in `verification/reports/README.md`; no formal
model-to-Rust refinement or physical-storage guarantee is inferred.


## Security maintenance (non-release changes)

- 2026-09-26: The protected `audit` check exposed
  [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html)
  in locked `rustls 0.23.43`. `gripsack-fetch` now requires patched
  `rustls >=0.23.45`, and `Cargo.lock` selects exactly `0.23.45`.
  Existing rustls-only TLS features and HTTP callers are unchanged.
  This dependency repair is not evidence for a frontend sandbox fix,
  formal verification, native platform qualification or a public
  release; the required CI and runtime gates still apply.

- 2026-09-26: plan/0048 §2.1's demonstrated composed-symlink escape is
  fixed at `db1e91b`: one order-independent bounded link-graph validator
  (`gripsack-fetch::fetch::archive::links`) is shared by TAR, ZIP,
  `validate_tree` and `copy_tree_filtered`, and materialization is
  root-pinned (cap-std `O_NOFOLLOW` open via the trusted parent, relative
  names only, symlinks created last). Both original fixtures reject
  `UnsafeArchive` in either member order after failing-before `Ok(())`;
  cycles/dangling links reject; valid composed links and hard-link
  ordering still work; regression seeds land in the fuzz corpus
  (`composed-link-escape.tar/.zip`) with a resolved-destination oracle.
  Six gates green on the exact tree (Rust 68 in fetch, TS 65/65,
  e2e **320/320**, TLC, Verus **72/0**+7 mutants, fuzz replay). Receipt:
  `verification/reports/2026-09-26-m0-archive-links-db1e91b.log`.
  Acquisition-side containment only — no deploy ownership change, no
  containment theorem; required PR CI at the evidence head `23d4940`
  later passed (run 36257568636).

## Settled rejections (all eras)

- **TOML/data-format frontend** — five times. TypeScript is the
  authoring surface.
- **cap-std piecemeal** — the migration was all-or-nothing (0020);
  0021 did it whole.
- **Sidecar-only SBOM** — the SBOM travels inside the binary (0022).
- **SQLite for the store** — can't transact scattered HOME files;
  the fs IS the store (0035).
- **Machine-local pin grant setting** — the trust gate is the
  boundary; validation happens at the grant (0033).
