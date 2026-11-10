# 0058 — Handover, final reconciliation

Supersedes [0056a](0056a-handover-reconciliation.md). Checked against
`origin/foundation/release-0.52` at `5af45a6` (0.52.0 head), read-only: no
gate was run, so "landed" below means the code path and, where it exists, a
named test are in the tree, not that a gate was re-run today. Method: every
row cites a file in this tree; items 0056a only took from plan text were
re-read in code, and the ones that could not be (external repos) are marked.

## Verdict

The old handover (0048 R1-R8 and M-V1-M-V8, the 178-row edition-5 import in
0049, 0053 release research, 0054 completion) is finished as a *product*
and dead as a *process*. Every release-blocking boundary defect (E132, E133,
E400, archive links, source-bundle approval with Landlock, durable hook
intents, private journals) is in the code, and the workspace, tasks,
schedules, semantic diff, saved plans, signed sharing and services it asked
for all ship. The ledger machinery around it (delivery.json, 39 formal-row
catalogs, receipts, signing, the Buildbot flow) is superseded by 0056, and
the Mac/BuildKit/Homebrew halves were retired by owner decision. What is
genuinely left is small: one real recovery usability defect (the unrestorable
entry has no doctor inspection or repair route), a staging-directory leak, a
handful of cheap product and governance gaps (`SECURITY.md`, `update --json`,
trace permissions/retention, three CI hygiene nits), typed ownership roles
(which belong inside F2 step 6), and one deliberate product limitation
(file to directory replacement). One 0048 defect (§4.5 `create_temp`) is
mostly refuted, and several assumptions in the handover were wrong (see
Pushback).

## Table

Status: landed (in code), pending (not done or partial), obsolete
(superseded or moot, with the reason), rejected (premise wrong or declined).

| Item | Handover source | Status | Evidence | Recommendation |
|---|---|---|---|---|
| Deno read-grant comma injection (E133) | 0048 §1.1 | landed | `E133` in `crates/gripsack-ir/src/diagnostic_codes.rs`; `schema/diagnostics.json` | none |
| Repo-controlled eval env / credential audience (E400) | 0048 §1.2 | landed | `E400` in `diagnostic_codes.rs`; `e2e/test_http_policy.py` | none |
| Unvalidated `--host`/lock path (E132) | 0048 §1.3 | landed | `crates/gripsack-ir/src/host_name.rs` | none |
| Bounded frontend output, adopt trust gate | 0048 §1.4 | landed | 0056a evidence; adopt gating in `commands/adopt/` (not re-read line by line) | none |
| Approve = evaluate same source bundle, trust v2, Landlock | 0048 R1 | landed | `crates/gripsack-store/src/source_bundle/`, `schema/trust/v2.json`, `schema/evaluation/v1.json`, `crates/gripsack-process/src/confinement.rs` | none |
| macOS seatbelt confinement | 0048 R1 | obsolete | PLATFORM-LINUX-WSL decision, 0055 | none |
| TAR/ZIP composed-symlink escape | 0048 §2.1 | landed | `UnsafeArchive` in `crates/gripsack-fetch/src/fetch/archive/paths.rs` | none |
| Plugin receipt identity, throttle NaN/inf | 0048 §2.2 | landed | `gripsack-fetch/src/throttle.rs`, `plugins.rs` | none |
| Worker panic deadlock in scheduler; Loom (M-V2) | 0048 §3.1 | obsolete | no `thread::spawn`/`catch_unwind`/loom in `crates/gripsack-exec/src` (only one test thread in `sharing/registry_tests.rs:41`); module scheduler deleted 0.50 | none |
| Update/`--check` parity, complete survey | 0048 §3.2, R8 core | landed | `gripsack-policy/src/update_survey.rs`; `e2e/test_update_surveys.py` | none |
| `resources` on Verify/Intent steps | 0048 §3.3 | obsolete | steps removed 0.50 | none |
| `grip update --json` | 0048 R8 | pending (confirmed) | `Command::Update` in `crates/gripsack/src/main.rs:168-178` has `host/repo/check/outputs` only, no `--json` | product, small; keep |
| Manifest `prior` bypassed admission | 0048 §4.1 | landed | 0048 §14 row; strict prior admission in store | none |
| Duplicate-destination keyed on spelling | 0048 §4.2 | landed | `generations/admission.rs` uses `entry.key()`; E111 | none |
| Generation staging residue never reaped | 0048 §4.3 | landed (F2 step 4, 0056 §18: the GC machine reaps canonical staging) | `generations/publication.rs:129-148` creates `.staging-N-<pid>` and removes only its own pid's dir; `inventory.rs:76` ignores non-numeric names; no reaper anywhere in `crates/` (grep); `generations.rs:343` asserts only `.staging-1`, a name that is never created, so the test is vacuous | fix in F2 step 4 (GC machine), 0.5 day; no correctness impact, disk leak only |
| `create_temp` unlinks a colliding temp file | 0048 §4.5 | rejected (mostly refuted) | `gripsack-fs/src/lib.rs:61-71` names temps `.{prefix}-{pid}-{counter}-{base}.tmp`, so a collision is the *same pid* leaving a file from a crashed run. It cannot hit another live writer in the same pid namespace. Residual: two containers sharing one `$HOME` both start at pid/counter 0 | drop; if wanted, add a random nonce when `MemFs`/`RealFs` land in F2 step 1 |
| Journal/quarantine world-readable | 0048 §4.4 | landed | `journal/storage.rs` uses `private_state` helpers | none |
| Unrestorable-entry wedge: doctor inspection and manual repair | 0048 §4.6, M4 | pending (confirmed) | `journal/recover.rs:362-372`: `Prior::Absent` restore calls `remove_file`, a directory at the destination errors, entry retained, recovery fails closed. `commands/doctor.rs` (no journal/recovery inspection); `docs/operations/storage.md` has no repair procedure | keep; the only real recovery usability gap. F2 step 2 plus a doctor view, see ranked list |
| Destination-read applicability limit note | 0048 §4.7 | pending (docs only) | not in `verification/guarantees.md` | fold into the generated claims at F3 |
| Hook intent IDs, durable outcomes, `grip hooks` | 0048 R3 | landed | `Command::Hooks` `main.rs:203`; `gripsack-store/src/activation/`; `commands/hooks.rs` | none |
| Typed ownership roles instead of string/bool args | 0048 R2 | pending (confirmed) | `gripsack-policy/src/ownership.rs:37-42` `plan_copy(&str, Option<&str>, Option<(&str,bool)>, bool)`, `:103` `plan_link(bool,bool,bool,bool)` | absorb into F2 step 6 (planner into kernel); do not do it separately |
| Trace logs: private mode and retention | 0048 R5, M3 | pending (confirmed) | `gripsack-trace/src/lib.rs:54-55` `create_dir_all` + `File::create`, umask permissions, nothing prunes `runs/` | small; do before F4 dissolves the crate, or fold into it |
| Complete-tree containment tiers | 0048 R5 | obsolete | claim-gated, never started; 0056 §11 | drop |
| Identity/applicability matrix, VM power-cut | 0048 R6 | obsolete | 0056 §11 (no power-loss claim); 96-partition matrix replaced by the S1-S7 storage axioms | drop VM campaigns; S1-S7 supplies the applicability statement at F3 |
| Release flow: crates.io order, Homebrew, npm provenance | 0048 §7.1/7.2/7.6 | obsolete | `release-core.yml` publishes to GitHub only | none |
| CI permissions/concurrency, `gate` aggregator | 0048 §7.4/7.5 | landed | `ci.yml` | none |
| `audit.yml` carries unused `issues: write` | 0048 §7.7 | pending (confirmed) | `.github/workflows/audit.yml:9` | one-line fix, F0 leftover |
| Demo workflow VHS image unpinned | 0048 §7.9 | pending (confirmed) | `.github/workflows/demo.yml:48` `ghcr.io/charmbracelet/vhs` with no tag or digest | pin or delete the demo workflow |
| npm dependabot ecosystem missing | 0048 §7.11 | pending (confirmed) | `.github/dependabot.yml` lists `cargo` and `github-actions` only; `typescript/` is an npm package | one-line fix |
| Verified installer / self-update | 0048 §7.12, R4 | landed (checksum only) | `commands/self_update/verified.rs`, `installer.sh`; releases unsigned by owner decision | keep checksum, drop signing |
| Release evidence receipts, manifest | 0048 R4 | obsolete | 0056 §7, §8: release = green CI on tag | `scripts/release*.py` goes with F0 |
| `SECURITY.md`, private reporting policy | 0048 R7 | pending (confirmed) | no `SECURITY*` at repo root; `.github/` has workflows and dependabot only | write it; the cheapest item here |
| Canonical site | 0048 R7, §8 E7 | landed | `gripsack-dev.github.io` active (pushed 2026-11-08) | none |
| Homebrew tap and cask | 0048 §8 E1/E2, A0, A4 | obsolete | PRODUCT-LINUX-NATIVE; `gripsack-dev/homebrew-tap` repo still exists (last push 2026-10-09) | archive the repo |
| Ecosystem repos: dotfiles SDK pin, python tombstone, gripfetch-apt, conformance suites, reverse gate | 0048 §8 E3-E6, E8, E9 | pending (not checkable here) | repos exist: `example-env-python` last pushed 2026-08-27, `gripfetch-apt`, `griplint-conformance`, `gripfetch-conformance`, `.github` all untouched since August; not inspected | owner: one pass, tombstone/archive the stale ones; no core work |
| Merge `expect`, mutant attribution, floors, stale TLA arms | 0048 §6 fixes 1-5 | landed | `specs/Transaction.tla:25`; no `from_utf8(...).expect` in merge | none |
| TLAPS Transaction pilot (M-V1) | 0048 M-V1 | obsolete | `specs/TransactionProofs.tla` retires to Lean P1-P3 (0056 §8) | delete after P1-P3 |
| Merge-scan, GC-root, recovery theorem, journal Verus kernels (M-V3..M-V7) | 0048 M-V3-V7 | obsolete | Verus kernels in `crates/gripsack-policy`; retire per kernel as Lean lands; `crates/gripsack-kernel` + `proofs/Kernel/{Journal,Retention,Selection}.lean` is the pilot | per 0056 §8 |
| Aeneas/Lean experiment (M-V8) | 0048 M-V8 | landed | became F1; 785-line kernel, 34 Lean theorems (0056 §13) | none |
| Rate/operation/retry budgets as proofs | 0048 M-V7.2-7.4 | obsolete | demoted to plain Rust + property tests (0056 §8) | F2 step 10 |
| Conda/Pixi native, wheels, bounded helper | 0049 A3, A6 | landed | `crates/gripsack-conda`, `tools/conda-helper`, `e2e/test_package_native_wheel.py` | none |
| Workspace contract, outputs, diagnostics, locks | 0049 A1, A2, A5 | landed | `typescript/src/workspace/`, `crates/gripsack-exec/src/workspace/{realize,roots,retained,profile_checks}`; `Run/Shell/Task` commands `main.rs:118-122` | none |
| Alpha API cutover, module DSL retirement | 0049 A5-01/05/06 | landed | module system deleted 0.50; `main.rs` has no module commands | none |
| Selection/rebuild explanations (terminal and JSON) | 0049 A5-02 | pending (partial) | `plan`/`check`/`diff` have `--json`, `why-owns` exists; no rebuild/selection "why" | product, optional; keep only if users ask |
| Dogfooding: gripsack's own `gripsack.ts` | 0049 A5-03 | rejected | no `gripsack.ts` at repo root; `examples/workspaces/` serve as dogfood and run in CI | drop |
| Tasks, locks, finite graph | 0049 E1 | landed | `Task` command, `workspace/` task code; 0055 journeys | none |
| systemd schedules, `grip jobs`, unattended runs | 0049 E2-E4 | landed | `crates/gripsack-exec/src/workspace/schedules/`, `commands/jobs.rs`, `scripts/smoke_scheduled_jobs.py` | none |
| launchd / Mac scheduling | 0049 E0-02, E4-02 | obsolete | PLATFORM-LINUX-WSL | none |
| Disruptive sleep/reboot/DST campaigns | 0049 E5-01 | obsolete | owner-optional, never run; 0056 §11 | drop |
| Semantic diff, snapshots, saved plans, stale refusal | 0049 C0-C5 | landed | `crates/gripsack-exec/src/semantic/`, `saved_plan/`; `Diff`, `Snapshot`, `DiffConfig` commands | none |
| Portable manifests, signed sharing, OCI transport | 0049 D0-D6 | landed | `crates/gripsack-exec/src/workspace/sharing/`, `grip package` | signing is kept as shipped; do not extend (0056 §11) |
| Eligible substitution before build | 0049 D4-01..03,06 | obsolete | no substitution code (grep); builds retired in 0055 | drop |
| BuildKit, OCI image production, source-to-Conda | 0049 B0-B6, A7 | obsolete | retired by 0055, harness deleted; `workspace/image/mod.rs` says no authoring backend | none |
| Docker Engine services | owner addition | landed | `crates/gripsack-exec/src/services/`, `Up/Down/Services` | none |
| Native file to directory replacement without a receipt | 0055 limitation | pending (deliberate) | refused by design: `e2e/test_workspace_directory_evolution.py:170` `test_leaf_with_prior_refuses_directory_growth_before_mutation`; leaf-to-directory with no prior is handled (`:154`) | not a defect; revisit only when the directory domain enters the kernel (F5) |
| R45-18 automated mutable-overlay snapshot | 0054 | rejected | owner chose a documented manual workflow | drop |
| H0 reconciliation, A1-07 checker, G-01..G-08, `delivery.json`, 39 formal catalogs | 0049 H0, G | obsolete | 0056 §8 deletes them; `verification/` now holds only `guarantees.md` | none |
| Buildbot, local CI, off-device restore, signer host | 0053 P1-P9, 0054 | obsolete | 0056 §7; but `scripts/local_ci.py`, `release*.py` and `check_tlaps.py` still exist (6112 lines in `scripts/`) | finish the F0 deletions, not a handover item |
| Rule: nothing downgrades, missing evidence blocks release | AGENTS "Plan 0048 contract" | obsolete | replaced by 0056 §7 and the F0 AGENTS rewrite | check AGENTS.md reflects this |

## Pending, ranked

Each item is mapped to a 0056 phase. Sizes are rough engineering days.

1. **Unrestorable-entry wedge (0048 §4.6).** Two parts. (a) The decision in the Recover machine: what to do when the destination became a directory (F2 step 2, falls out of "decide, order, cleanup, seal"). (b) `grip doctor` journal view plus a one-page repair procedure in `docs/operations/storage.md`, plus a crash e2e that wedges and then repairs. Ring 1/2 and product. Size 3-5 days. Only item with user-visible harm.
2. **Typed ownership roles + FND-1 (0048 R2, 0056 §5a).** Fold into F2 step 6 (planner). Replaces the `bool`/`&str` signatures of `plan_copy` and `plan_link` and fixes the drift hole for recorded symlinks. Ring 0 (K1). Size is already inside the step; no extra ask.
3. **`SECURITY.md` + three CI hygiene lines.** `SECURITY.md`, drop `issues: write` in `audit.yml`, add npm to dependabot, pin or delete the demo workflow. F0 leftover. Size under half a day.
4. **Staging reaper (§4.3).** Sweep `generations/.staging-*` from other pids under the GC/recovery lock; make `generations.rs:343` assert the real name. F2 step 4. Size 0.5-1 day, ring 2.
5. **Trace log mode and retention (R5).** Create `runs/` and files at 0600/0700 and prune by count. F4 dissolves the crate; do it before or as part of that move. Size 1 day, ring 3.
6. **`grip update --json` (R8).** Product, outside rings. Size 1-2 days. Reasonable to wait for a user request.
7. **Ecosystem repo triage.** Archive `homebrew-tap`, `example-env-python`, tombstone the rest. Owner action, 1 hour; no code.
8. **File to directory replacement.** Hold until F5 (directory domain into the kernel). Document the refusal text instead. Size L if done, 0 to document.

Not listed because dropped: signing, VM power-loss, disruptive timing
campaigns, liveness, containment tiers, D4 substitution, A5-03, R45-18.

## Pushback

- **"Every NEXT item blocks release" was the wrong gate.** It forced 178
  protected rows, hand-built receipts and a 17-gate local CI to make a ledger
  true, while the code was shipping real features faster than the ledger could
  follow. `delivery.json` read "157 pending" while workspaces, schedules,
  diff and sharing were all in the tree. The ledger measured the paperwork,
  not the product. 0056 is right to delete it. Do not recreate it.
- **The 0053 flow answered the wrong question.** Nine pain points (P1-P9)
  led to a Buildbot, an off-device restore drill and a signer host, for a
  project whose release is now a GitHub tarball plus a checksum. Most of the
  pain came from the evidence process, not the build. Release = green CI on
  tag is enough. Note the machinery is not yet gone from the tree
  (`scripts/local_ci.py`, `release*.py`, `check_tlaps.py`: 6112 lines).
- **The handover treated four platform and product lanes as required that
  the owner retired.** Mac, launchd, BuildKit, Homebrew and image production
  account for roughly 50 of the 178 rows (B0-B6 alone is 40). They were never going to be "pending".
  A handover should not enumerate rows for scope the owner is deciding to cut.
- **§4.5 was a misread.** `create_temp` unlinks only a same-pid, same-counter
  leftover, because the name carries `{pid}-{counter}`. It cannot take another
  live writer's file within one pid namespace. At most it is a container edge
  case. Not worth a work item.
- **§4.3 is real but minor.** It is a disk leak, not a correctness hole:
  `inventory.rs:76` explicitly ignores non-numeric names, so a leftover cannot
  impersonate a generation. The test that claims "no staging residue" checks a
  name that is never created, so it proves nothing. Fix the test with the
  reaper.
- **File to directory refusal is not a bug.** The handover listed it as a gap.
  It is a conscious, tested refusal (a prior-bearing leaf is never silently
  replaced by a directory). Treat it as a documented limitation.
- **Proof work was specified as tool-specific counts.** "40 modules / 4,623
  obligations" and "no umbrella Verus total" described the checking tool, not
  the claim. The Lean theorems K1-K6 and P1-P4 state the claims directly.
  M-V1..M-V7 are therefore obsolete as work items; only their *properties*
  carry over, and those are already in 0056 §5.
- **Dogfooding (A5-03) and rebuild explanations (A5-02) were nice-to-haves
  written as release blockers.** The shipped examples already serve as
  dogfood.
- **Caveats on this review.** I could not inspect the external repos beyond
  their listing, and I did not re-run any gate. The rows marked "landed" with
  only a directory or command citation prove existence, not every conjunctive
  case of the original row. Where a defect was in question I read the code
  path (§4.3, §4.5, §4.6, R5, R8, R2); the rest of the §1-§4 fixes rest on the
  diagnostic code and schema files that exist plus 0056a's earlier reading.
