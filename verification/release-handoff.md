# Release qualification tracking and post-release fuzz handoff

Snapshot: 2026-09-28. Goal: a qualified release the owner can install and share
with close friends for feedback. **Not merge-ready or release-ready yet.**
Implementation, landing and qualified releases continue with the current
assistant. The additional agent is assigned post-release fuzzing and repairs,
not the remaining implementation or non-fuzz gates. This tracking document is
not a stopping point or evidence that missing implementations are complete.

## 1. Authority and safe starting point

Read the repository instructions and the `gripsack-pr`, `gripsack-release`
and, for flow tests, `gripsack-e2e` skills. Then read:

- [Plan 0048](../plan/0048-review-response-0.42.0.md), especially §§6, 9, 10,
  the §14 leaf evidence and §15 owner exception. Its NEXT scope controls the
  release, including prereleases advertising these guarantees.
- [Current status](../plan/STATUS.md), [guarantees](guarantees.md),
  [runner reports](reports/README.md) and [delivery inventory](delivery.json).
- From the repository root, the immutable edition-5 handover is in
  `../gripsack-handover/`, beginning with `START_HERE.md`, for the
  capability/milestone boundaries. That bundle is outside this repository;
  do not edit it or reset to its research baseline.

This is a navigation/runbook snapshot, **not a second authoritative backlog**.
Update plan 0048's leaf record and the owning delivery rows as work progresses.
All acceptance subcases in the cited sections remain conjunctive; the concise
rows below do not replace or narrow them.

Observed durable-authority checkpoint:

- Branch: `handover/h0-bundle-import`; pushed source
  `e3738d765025aa5ea352f47d603f83b43760feb5`. M-V5, transaction-bound
  selection, activation outcomes and four durability repairs are committed.
- [PR #164](https://github.com/gripsack-dev/gripsack/pull/164) remains draft;
  the complete handover and remaining NEXT requirements are not merge-ready.
- Explicit-waiver run
  [36385223651](https://github.com/gripsack-dev/gripsack/actions/runs/36385223651)
  passed Linux, native macOS, docs, audit and the aggregate. Linux CLI passed
  **405/405**. Mac process tests passed **25/25**; CLI passed **403**, with **2** filesystem-unconstructible
  filename fixtures explicitly skipped. Only fuzz was an owner-waived job.
  Earlier run `36337128792`'s failures remain evidence, not erased by the repair.
- Last observed `main` protection requires `test`, with strict up-to-date
  checks, `enforce_admins=false` and zero required approvals. The fail-closed
  `gate` is now implemented, but its live protection requirement is not yet
  observed. Admin bypass cannot qualify a missing gate.
- The delivery ledger contains **178** rows: 157 pending, 14 in progress,
  5 implemented-unverified, 1 verified, 1 blocked. `B0-01` is the verified
  historical Linux/container qualification, not a qualified current builder.

No candidate version/tag or complete release capability matrix has been fixed.
Choose and record the actual release scope before closure: future C/D/E and
foundation-extension rows must not block an unrelated native release, but
neither can a partial file-profile implementation be called full A2/foundation.
Plan 0048's remaining NEXT gates still apply unless the owner explicitly amends
those exact requirements. No such additional amendment exists.

## 2. Owner-approved fuzz exception

**REL-FUZZ-2026-09-27**, recorded in [plan 0048 §15](../plan/0048-review-response-0.42.0.md):
the owner chose **“Release without fuzz/replay”** and clarified that this
assistant must complete and release the handover before the other agent fuzzes
it. The exception covers this continuation's feedback releases; every other
gate remains mandatory.

- Status: **not run / owner-waived**, not passed, verified or permanently removed.
- Do not execute fuzz/replay as part of this handoff's ordinary no-fuzz path.
- Keep the runners/corpora intact for a separately delegated fuzz assignment.
- The waiver does not cover ordinary deterministic regression/property tests,
  Loom, TLC, TLAPS, Verus, semantic-mutant calibration or persistence cases.
- CI supports the explicit manual-dispatch input
  `fuzz_waiver=REL-FUZZ-2026-09-27` and fail-closed result aggregation.
  Local calibration passed 4 positive/64 negative cases and four executable
  entrypoint smokes. Run `36337128792` correctly failed on non-fuzz failures;
  runs `36357284121` (`472a111`) and `36385223651` (`e3738d7`) passed at their
  exact sources. Live aggregate protection remains pending.
  Release assurance must still bind the exception into the candidate tuple.
  `delivery.json` has no `waived` status: do not invent one there or label a
  skipped fuzz job `verified`.
- PR/main-push CI retains its normal fuzz lane. Preserve `[skip ci]` while
  landing candidate source without triggering it; then explicitly dispatch
  the updated CI with the owner waiver. A waived gate requires fuzz to be
  skipped and every other required job successful. Unknown waiver, missing
  job, failure, cancellation or any other skip fails the aggregate.
- `.github/workflows/fuzz.yml` also has a separate weekly schedule. The release
  exception does not silently reconfigure or disable that unrelated schedule.

Deferred fuzz commands, recorded for the model specifically assigned that lane:

```sh
# Saved-corpus replay, contained by the existing runner:
docker compose run --build --rm fuzz

# Bounded coverage-guided run; requires the disposable contained environment:
docker compose run --build --rm -e FUZZ_SECONDS=300 fuzz sh scripts/check_fuzz.sh scheduled
```

`fuzz/run.py` owns containment and target inventory; `scripts/check_fuzz.sh`
selects replay/scheduled mode. No uncontained fallback. Retain failing inputs,
fix the actual production defect, then rerun the affected target and consumer
regressions. Do not treat a missing tool, timeout or sandbox-construction failure
as a successful semantic negative. §7.8 still requires archived failure
artifacts; the waiver is not permission to discard future findings.

**Less obvious corpus consumer:** `tools/buildkit-bridge/build.sh` runs
`protocol.TestSharedConformanceCorpus`, which reads
`fuzz/corpus/buildkit_protocol`. Keep that out of an advertised no-corpus path;
record it separately or delegate it with the corpus work. Do not silently skip
it and claim complete Rust/Go conformance.

## 3. Evidence already available: reuse with source matching

The R3/transaction-identity packet is qualified at source `472a111`, not as a
whole release. All six local Compose gates passed, including **401/401** CLI,
Verus **111/0** plus **17** calibrations, TypeScript **67**, the **301**-obligation
TLAPS pilot and the complete TLC gate. Full native/current-commit CI then
passed; `reports/2026-09-28-ci-472a111.log` has SHA-256
`8b573eee464d1e739f942bba572296b5a8e969f30855b5e3f3e7a041be5b658d`.
The later prior-cache, directory-birth, retained-prior and generation-prune
repairs are qualified at `e3738d7`: all local gates and full native/current-commit
CI passed. `reports/2026-09-28-ci-e3738d7.log` has SHA-256
`87aa22c96fe32b682e1f2fa45758c8546496f1f520bea40125a624aab6ea7637`.
Neither checkpoint closes generalized M-V6/M-V7, live protection or the
remaining release requirements.

The subsequent observed-generation and legacy allocation-floor repairs have a
complete local packet: **416/416** real CLI, **67/67** focused flows and all six
non-fuzz Compose gates. `reports/2026-09-28-generation-authority-local.log`
has SHA-256
`7c2e3fd0e1650bcdd926211364e43f8c9c90160ecdf68ae02886e9028d87830a`.
Its source fingerprint is
`5abd7717ff346b74cb25042d96bd0e27f76f26e112c599b619b617cc67a265e7`;
exact source `ae77f524bace1e46b738832fe99f133feb51ed51` then passed CI run
`36468870180`, including Linux **416/416**, native Mac **414 passed / 2 skips**
and native process **25/25**. `reports/2026-09-28-ci-ae77f52.log` has SHA-256
`644af54a4bebccbc5b86e0be5bde4cbc781668af29c83e92a35f83147dc3f144`.
Private proof development now includes checked journal, activation/lifecycle
and granular preparation components. Complete GC composition and integration
into the shipped proof runner remain required; this is not M-V6 closure.

The [report index](reports/README.md) also preserves historical M-V5 evidence:

| Evidence | Observed result | What it does not establish |
|---|---|---|
| `reports/2026-09-27-m-v4-c92e84e.log` | Exact-source store 69/69, seven parser/recovery properties plus missing-field calibration, CLI 10/10 | Serde theorem, new candidate CI or native platform closure |
| `reports/2026-09-27-m-v5-local-rust.log` | fmt/clippy/tests, Loom/journal/GC calibration; 898 test-image source files matched | The three ignored real-Go/Docker integration tests |
| `reports/2026-09-27-m-v5-local-cli.log` | Final working-tree real CLI/Deno **362/362**, including GC 30, build closures 7, history 10, recovery 10 | Every future handover capability or native macOS |
| `reports/2026-09-27-m-v5-local-proof.log` | Whole policy **92 verified/0 errors**, 13 attributed calibrations, 20 image-bound policy/Cargo/runner inputs | Generalized recovery M-V6, protocol M-V7, filesystem inventory completeness |
| `reports/2026-09-27-m-v5-local-smoke.log` | Five actual CLI cases; root substitution refusal, old current, orphan link, expired history, ID exhaustion | Release artifact/provenance or public installation qualification |
| `reports/2026-09-27-m-v5-local-gates.log` | All six local non-fuzz Compose services passed | TS/TLC/TLAPS RUN layers were cached; no fresh counts claimed |

Base for the M-V5 packet: `c92e84e`; tracked patch SHA-256
`87b991bc888af8342823771505a2d30bdadd3526c8cc81b5524967d751d6b5e2`;
source-file fingerprint including new source files:
`581f470d6761f6e4e6c839f9dbb91a61604c3ef2dd883e10db196ad9e92a63f6`.
Subsequent handoff documentation does not change those behavior-bearing inputs.
Reconcile live changes before reuse; commit cleanly and bind candidate evidence
rather than retroactively relabelling dirty-tree reports as protected CI.

M-V1's existing TLAPS result is a **301-obligation, one-destination/one-recovery
pilot**. M-V2's shared scheduler Loom coverage, M-V3's actual scanner proof,
M-V4's byte admission and M-V5's typed retention are implemented with local
reports. Preserve their named obligations/calibrations when extending them.

## 4. Missing implementation/proof gates

| Gate / authoritative scope | Current gap | Next action and acceptance |
|---|---|---|
| M-V6 — §6 generalized recovery | Generalized source/proof modules and the real six-destination retry campaign are integrated; complete source-bound gates and native/current-commit qualification remain open | Finish the frozen transitive proof catalog, all finite discovery/counterexample cases, full Compose gates and candidate CI. `GeneralRecoveryProofs.tla` joins journal, publication, selection, granular preparation, activation and retention; `RepeatedRecovery.tla` replaces the obsolete two-destination engine while retaining explicitly bounded conditional-completion checks over the shared actions. Private component proofs and a passing CLI campaign are not release qualification. |
| M-V7 — §6 items 1–5 | No completed production-connected journal/process/update/acquisition transition-proof packet is recorded | Constrain actual effect authority by typed/verified durable predecessor states; prove distinct frame/byte/work counters and budget arithmetic, one operation deadline, terminal-failure precedence, complete update accounting and shared check/publish decision. Extend actual observers/executors/error/receipt oracles and dropped-sync persistence calibration. Register every external-effect assumption; no ghost-only clone or opaque trusted write wrapper. |
| R1 — all §13 R1 changes/acceptance | `trust.rs` still keys approval by path and accepts `GRIPSACK_TRUST_ALL=1` | Implement captured immutable `PreparedEvaluation`, complete bounded admitted read set, source/grant/runtime-bound approval, all-round bundle reuse, versioned trust migration and private receipts. Migrate every eval caller, fixture/demo bypass and diagnostic path. Run every named worktree/import/symlink/submodule/pin/grant/pause/IO case through real Deno. |
| R2 — remaining §13 R2 plus §10 | M-V5 supplies generation types/inventory/exact pruning, **not** ownership role types, complete independent seam oracles or policy receipts | Finish zero-copy desired/live/prior roles, takeover and lineage authority types; migrate every caller/model/proof without raw overloads. Extend real filesystem oracles and attributable role/authority/observation/effect/receipt mutants; implement the versioned durable policy receipt. Required compile-fail and Verus contracts remain part of acceptance. |
| R3 — all §13 R3 | Stable selection/intent IDs, private outcomes/archives, settled failure/supersession, legacy migration, inspection and fixture-only simulations are implemented and qualified by local/native/current-commit runtime gates | Preserve the 43-case focused crash/activation bridge, latest 405-case Linux / 403-case native CLI results, real simulation/example smokes and six bounded model mutants. Compose the persisted identity with generalized M-V6; no exactly-once claim. |
| R5 — NEXT portion from §9 | Scoped process supervision/plugin fixes exist; full required identity/env/FD/receipt/escaping campaign is not closed | Inventory and migrate every caller touched by NEXT work; bind launched executable/interpreter/script identity with honest OS assumptions, role env/grants and FD handling. Add private bounded receipts/required trace controls and actual lifecycle/pressure/canary/terminal-output cases. Current process tests live at `crates/gripsack-process/src/tests/`, not the older plan's `tests/` spelling. Additional full-tree containment is claim-gated, never silently inferred from process groups. |
| R6 — NEXT applicability/identity and software-fault cases | Existing persistence tests do not close the complete named applicability/campaign contract | Record actual metadata/alias/filesystem limits, resolve dangling guarantee IDs, add the machine-readable applicability matrix and applicable ENOSPC/EDQUOT/EIO/ESTALE/EXDEV/lock/parent-replacement/readonly cases across real publication/recovery/receipts. Qualified VM power-cut evidence is separately claim-gated/M5: do not invent a hardware claim or run destructive experiments on user disks. |
| R4 — all §13 R4 | No `schema/release/` or `schema/verification/` contracts found; `install.sh` currently installs after same-origin checksum verification | Implement generated assurance/release manifests, exact-source/evidence/artifact/compatible-SDK binding, identity/ref/workflow-constrained attestation verification, fail-closed installer/self-update, revocation and interrupted-publication recovery. Run the complete non-publishing valid/tampered/wrong-identity/missing-evidence/mismatched-SDK/revoked cases; old binary stays usable on every failure. |
| R7 and §8 E1–E3/E7–E8 — NEXT public surface | No root `SECURITY.md` found; current-candidate tap/site/profile/example and owner security qualification is not recorded | Finish current API/frontend/compatibility docs, real private reporting/support/rotation/revocation policy and owner actions. Canonical site is `gripsack-dev/gripsack-dev.github.io`; bind banner/installer/examples/assurance to one verified release tuple. Exercise actual examples/browser/deployed headers and advertised Formula/Cask channels. Do not archive/yank/delete other projects implicitly. |
| Existing §§1–6 NEXT fixes and §10 quality | Local packets exist, but the final complete release candidate is not qualified | Preserve and requalify evaluator/host/adopt, archive/plugin/throttle, scheduler, prior/privacy, update/resource/sema and evidence-integrity fixes. Include source-matched original regressions, required module/type cleanup and explicit review. A required failing case promotes its necessary fix to NEXT. |

R8's current survey/error-precedence contract remains required; its new JSON
survey interface is authorized later work. §4.2/4.3/4.5/4.7, broad trace/caller
breadth, recovery convenience, ecosystem conformance additions and M-V8 retain
exactly the NEXT/LATER/CLAIM-GATED split in §9. Do not make every later item a
new release blocker, or defer a prerequisite exposed by a required failure.

## 5. Delivery/CI changes: all 13 §7 items accounted for

| Plan item | Observed gap / required action |
|---|---|
| §7.1 | Core publish loop still places `gripsack-ir` before `gripsack-policy`. Derive/validate topological order from Cargo metadata, including every publishable current workspace crate; calibrate a dependency-order violation without publishing. |
| §7.2 | Homebrew bump precedes GitHub release creation. Finalize/verify the complete release tuple before tap/site/channel advancement; registry publication remains before announcement. |
| §7.3 | Post-download handoff checks count four tarballs but do not verify the exact expected subject set/digests before attestation. Add recomputation at every handoff and test missing/extra/mismatched subjects and checksum root resolution. |
| §7.4 | CI read-only permissions and cancel-in-progress concurrency are implemented. Preserve non-canceling release publication. |
| §7.5 | Fail-closed `gate` and explicit fuzz-waiver admission are implemented and their real failed-run behavior observed. Live protection still needs the aggregate; qualify corrected current-source CI before relying on it. |
| §7.6 | SDK workflow has no `id-token: write`, npm `--provenance` or non-canceling publication concurrency. Implement and verify the selected registry identity/auth policy; trusted-publishing/token retirement remains an explicit external action if chosen. |
| §7.7 | Scheduled audit grants unused `issues: write`. Remove it or implement real idempotent failure reporting; record the yanked/unsound advisory policy deliberately. |
| §7.8 | Scheduled fuzz workflow lacks crash-out mount/upload. Preserve this repair requirement for the delegated lane; archive failures with 90-day retention when that lane runs. Current feedback waiver does not fabricate a passing fuzz result. |
| §7.9 | Demo uses floating VHS and `curl ... deno.land/install.sh | sh`. Pin VHS by digest, verify the Deno ZIP, register the pins, and migrate source-bound fixture approval with R1. |
| §7.10 | Verus image still downloads an unchecksummed rustup installer. Pin `rustup-init` bytes and validate the installer pin. |
| §7.11 | Dependabot lists Cargo/GitHub Actions only. Add npm for `/typescript`; Docker pins remain owned by `check_pins.py`. |
| §7.12 | Installer selects matching core tags without a stable-only semantic filter and verifies checksums only. Complete R4's fail-closed verified path and prerelease/withdrawal policy; test old-binary preservation before executing candidate bytes. |
| §7.13 | CI audit's unused `checks: write` permission and stale explanation are removed in the committed CI policy. |

Workflow source and explicit-waiver CI dispatch changed during continuation.
No branch protection, registry, tag, release, domain or private-reporting
setting has been changed by this packet.

## 6. Missing executions and real-platform gates

| Gate | What must run / be recorded | Limits and prerequisites |
|---|---|---|
| Clean committed candidate | Commit reviewed implementation/new files, then bind every applicable report and PR check to the full candidate SHA | Do not lose the dirty M-V5 changes or substitute the older remote PR head. Split the omnibus draft into reviewable complete packets where necessary. |
| Required CI | Existing Linux `test`, native `e2e-macos`, docs and audit plus the implemented aggregator/waiver policy | Do not dispatch the current unconditional fuzz workflow. Failed/cancelled/unexpected skips block. Manual dispatch alone does not establish branch protection. |
| Native macOS | Full real CLI/Deno suite on the declared native runner; filesystem/name/permission and changed process/effect cases | Prior `ce3c7e0` native 319/319 is historical, not new-candidate evidence. Cross-compilation is not native runtime evidence. |
| BuildKit Linux qualification | Current-source `verification/buildkit-qualification/bridge/build.sh` and `probes.sh`, with real pinned daemon and independent image verification/run | Required by the current Linux CI job. Its historical B0-01 pass does not close B1/B2 or prove a Mac VM. Inspect the harness's disposable-resource requirements first. |
| Three ignored Rust integration tests | `transport::tests::the_real_go_bridge_negotiates_fails_closed_and_cancels_idempotently`; `worker::linux::tests::the_real_docker_worker_provisions_leases_and_stops`; `worker::linux::tests::the_real_docker_worker_refuses_a_foreign_cache_volume` | Not exercised by ordinary `cargo test`. Real Go bridge and dedicated Docker daemon/image required. Production bridge build also runs the shared corpus noted in §2; delegate that part explicitly. Record individual results, not “Rust gate passed.” |
| Mac VM / launchd | B0-02/B1 Mac worker lanes and E0-02 native launchd, when included in the selected capability/closure claim | Current ledger marks B0-02 blocked. systemd evidence is not launchd; Linux BuildKit is not native Darwin production. Missing facilities stay blocked, not unsupported-success fixtures. |
| Reproducibility | Two clean pinned-image release builds using `scripts/check_reproducible.sh` | Archive report/config/toolchain/binary hashes. One build or a cached compile is not this experiment. |
| Artifact qualification | All advertised Linux musl/Darwin targets, package content, in-binary SBOM/advisory checks, checksums, manifest/provenance verification and compatible packed SDK | Current core workflow lists four targets; its Darwin x86_64 cross-build is not runtime qualification. Use the actual release tuple, not an assumed equal core/npm version. |
| Public docs/install/site | Actual packed SDK + candidate core in executable website examples, disposable install/update cases, browser and deployed-header checks | Source site from the canonical repository, not an invented local mirror. Live domains/private reporting/registry grants need observed external evidence. |
| Delivery closure | Inventory/calibration plus selected milestone/scope closure on the committed candidate | Inventory validity is not closure. Keep future unrelated scopes pending; all required cases/lanes/proof catalogs of a claimed milestone must pass. |

## 7. Existing commands for the assigned model

Run from the repository root unless a command states otherwise. Resolve missing
implementation above first; **today's `model`/`tlaps`/`verify` commands cannot
prove an unimplemented M-V6/M-V7 contract**. Do not run release tag workflows as
tests; they publish to real registries.

Inventory and calibrated dependency boundaries:

```sh
python3 scripts/check_delivery.py --validate
python3 scripts/delivery_checker_calibration.py
python3 scripts/check_architecture.py --self-check
```

Ordinary non-fuzz container gates (use `--build` after source changes):

```sh
docker compose run --build --rm test
docker compose run --build --rm ts-test
docker compose run --build --rm e2e
docker compose run --build --rm model
docker compose run --build --rm tlaps
docker compose run --build --rm verify
```

For fresh execution rather than reused build RUN layers, use the existing
in-image runner; retain its actual output. Examples:

```sh
docker compose run --build --rm test python3 scripts/check_gc_roots.py
docker compose run --build --rm test python3 scripts/check_journal_admission.py
docker compose run --build --rm model sh scripts/check_models.sh /tla/tla2tools.jar
docker compose run --build --rm tlaps python3 scripts/check_tlaps.py
docker compose run --build --rm verify sh scripts/check_verus.sh
docker compose run --build --rm e2e uv run --locked --no-sync pytest test_gc_admission.py test_build_closures.py test_generation_history.py test_transaction_recovery.py
```

B0 qualification uses its own pinned Go client, not the production bridge build:

```sh
sh verification/buildkit-qualification/bridge/build.sh
sh verification/buildkit-qualification/probes.sh
```

On the qualified pinned Rust Linux runner, after the actual production bridge
and dedicated Docker prerequisites have been supplied, these execute the
otherwise ignored tests. Coordinate corpus work as described in §2:

```sh
cargo test --locked -p gripsack-buildkit --lib transport::tests::the_real_go_bridge_negotiates_fails_closed_and_cancels_idempotently -- --ignored --exact --nocapture
cargo test --locked -p gripsack-buildkit --lib worker::linux::tests::the_real_docker_worker_provisions_leases_and_stops -- --ignored --exact --nocapture
cargo test --locked -p gripsack-buildkit --lib worker::linux::tests::the_real_docker_worker_refuses_a_foreign_cache_volume -- --ignored --exact --nocapture
```

`GRIPSACK_BRIDGE_BIN` can name the actual binary; otherwise the test uses
`tools/buildkit-bridge/bridge-bin`. Use disposable owned resources, never remove
foreign containers/volumes to get a pass. Finish with the normal container gates.

Non-publishing artifact work, after the candidate version/scope is settled:

```sh
sh scripts/check_reproducible.sh
# VERSION must be the reviewed candidate version, not an invented release claim.
: "${VERSION:?set the reviewed candidate version}"
docker compose run --build --rm -e "VERSION=$VERSION" release
(cd typescript && npm ci && npm test && npm run build && npm pack)
```

Use Cargo metadata-derived order for **all** publishable-crate package/dry-run
checks after §7.1 is fixed; do not copy the current broken publish loop. No
`cargo publish`/`npm publish` without the dry-run flag during verification.

For published examples, set `SITE` to the real canonical website checkout,
`SDK` to the just-built tarball, and `GRIPSACK_BIN`/`GRIPSACK_DENO` to the actual
candidate/pinned runtime paths:

```sh
python3 scripts/check_examples.py --site "$SITE" --core-repo . --sdk "$SDK" --grip "$GRIPSACK_BIN" --deno "$GRIPSACK_DENO" --selfcheck
```

Current checker closure commands (expected to reject incomplete claims):

```sh
: "${CANDIDATE:?set the full committed candidate SHA}"
: "${MILESTONE:?set an actually claimed milestone}"
python3 scripts/check_delivery.py --close-milestone "$MILESTONE" --release "$CANDIDATE"
# Only when claiming the entire registered foundation scope:
python3 scripts/check_delivery.py --close-scope foundation --release "$CANDIDATE"
```

Other registered scopes are `foundation_extensions`, `tasks_schedules`,
`semantic_change` and `artifact_sharing`; there is no registered `feedback`
scope. Do not invent a successful closure by relabelling the claim.

## 8. Failure handling, evidence and release exit

For each assigned gate:

1. Fix its named support/case/proof inventory **before** judging results. Record
   the production entrypoint and implementation prerequisite, not just a command.
2. Run the real surface in sandboxed HOME/offline fixtures or the explicitly
   qualified platform. Reuse existing runners; no mirror algorithm or no-op test.
3. Preserve failure output and its source/tool/input identities. Diagnose and fix
   the production cause; add/retain a deterministic consumer-visible regression
   where appropriate. Do not bless new output, weaken the oracle, lower a proof
   floor, replace a positive with unsupported, or hide failure as a skip.
4. Re-execute the affected gate and integration surface after correction. Proof
   mutants must fail the named property/function; compiler errors, missing tools,
   timeout and unrelated failures are not calibration.
5. Record full SHA plus dirty-patch digest if applicable, exact command, tool/image
   versions, fixtures/configuration, expected and actual executed/pass/fail/skip
   counts, named obligations/calibrations, raw report and SHA-256, assumptions,
   remaining blockers and next action. Verify downloaded artifact bytes at handoffs.
6. Update the existing leaf ledger, guarantees and report index together. A local
   pass does not change a live GitHub setting or prove native/hardware behavior.

External owner/facility gates remain explicit: stronger branch protection and
release environment policy; registry/signing authorization; tested private
security-report route and support policy; revocation/trust-anchor recovery;
canonical domain/tap/site authority; native Mac/VM resources for claimed lanes.
Check reachable APIs/configuration first. If unavailable, record exactly what
was attempted and who must supply it; never invent contacts, settings or evidence.

Exit only when the selected release's implementation and non-waived evidence
close, the exact committed candidate has required protected/native results,
the non-publishing artifact/installer/resume negatives pass, and the finalized
compatible release tuple is verified before tap/site/channel consumers advance.
The owner authorized proceeding toward a release, **not bypassing the remaining
gates**. Keep unrun/waived and qualified results distinct in the final handoff.
