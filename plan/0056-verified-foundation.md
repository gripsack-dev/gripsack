# 0056 — Verified foundation: a Lean-proved kernel, rings outward, and a trim

- Status: approved by the owner 2026-11-08; executing (F0, F1)
- Date: 2026-11-08
- Owner direction: 2026-11-08 conversation — "a kernel that is Lean proved …
  interaction with the world needs proper proofs … keep expanding the rings";
  "go aggressive on the current proofs/test harness"; "feel free to reorganize
  modules and crates"
- Supersedes: 0047 "Tooling decisions" (Lean deferred), 0048 §6 binding
  verification programme (M-V1–M-V8, "no new proof engine replaces Verus"),
  0048 §9–§10 as release gates, the formal/evidence parts of 0053/0055 that
  this plan replaces. Runtime safety contracts (0001 §9, ownership, journal
  authority, artifact integrity) are **not** superseded — they become theorems.
- End-state picture: [`docs/diagrams/foundation.svg`](../docs/diagrams/foundation.svg)
  (source `docs/diagrams/foundation.d2`, rendered by `docker compose run --rm diagram`)

## 1. Why

Gripsack's guarantees are real but expensive. They were added after the
code was written, one review response at a time, so every new guarantee
needed its own model, its own Rust explorer that mirrors the model, its own
calibration mutant and its own evidence receipt. Measured on `main` at
`ff8dd8f`:

| Artifact | Size |
|---|---|
| TLA+ specs (`specs/`) | 90 modules, ~9.5k lines; TLAPS ~4,600 obligations, CI failures from prover timeouts |
| Verus kernels (`gripsack-policy`) | 6.5k lines, depended on by 9 crates (ir, process, fetch …) — a utility crate, not a kernel |
| Hand-mirrored Rust explorers | `lineage_model`, `mode_model`, `ops/model`, `gc/root_model`, `journal/model`, `journal/repeated_model` |
| Evidence / release / local-CI Python | ~10k of the 15.7k lines in `scripts/` |
| Committed evidence logs | `verification/reports/`, 180 files, 51 MB |
| Plan prose | 16.4k lines; 0048 alone is ~2.9k lines of binding contract |
| `gripsack-exec` | 54k lines: transaction core, Conda, sharing, services, schedules, launchers, semantic review |
| Compose gates / CI jobs | 12 services; 11 CI jobs |

Two structural problems sit under the numbers:

1. **No proof is about the composition.** Verus proves small decision
   functions, TLA+ proves abstract protocols, and tested "bridges" connect
   them. 0048 itself labels those links unproved. Each bug found at a link
   adds another bridge.
2. **The code isn't shaped for proof.** Decisions, observation and effects are
   interleaved through `apply.rs`, `ops/*`, `journal/*`, `gc.rs` and
   `activation/*`. So every proof needs an extraction step, a mirror and a
   receipt.

## 2. Principles

1. **Rings.** A pure kernel at the center, proved. Each ring outward holds
   less trusted code and is checked in a cheaper way. Code moves *inward* to
   gain guarantees; models are never added *outward*.
2. **Proofs are about the shipped code.** The kernel is plain safe Rust,
   translated to Lean by Charon/Aeneas, and proofs are over that translation.
   There is no hand-written Lean copy of the Rust.
3. **One proof language.** Lean 4 for the kernel *and* for the crash/recovery
   protocol (ring 1), so "kernel plus storage is crash-safe" is one theorem
   rather than a cross-tool bridge. TLC stays as a cheap **counterexample
   finder**, never the proof of record. Verus and TLAPS retire.
4. **Sans-IO.** Kernel and protocol machines never touch the world. They
   consume events (observations, effect results) and emit commands. One small
   driver executes commands.
5. **The survival rule.** An artifact stays only if it is (a) a machine-checked
   proof, (b) a test guarding behavior users see, or (c) a counterexample
   finder that has caught a real bug. Anything that exists to prove we ran
   something goes.
6. **The release rule.** A release is green CI on the tag. Proofs run in CI
   like any other test. No receipts, evidence indexes, per-release proof
   catalogs or waiver IDs.
7. **Generated claims.** The guarantee list is generated from Lean theorem
   statements plus `#print axioms`, not written as prose.

## 3. The rings (end state)

| Ring | Contents | Guarantee | How it is checked |
|---|---|---|---|
| **0 — Kernel** | Pure decisions over typed state: desired-state planning, ownership/consent, drift, managed-block splice, generation/selection identity, commit classification, recovery decisions, GC retention, activation-intent bookkeeping | Kernel theorems (§5) for **all** inputs | Lean proofs over the Aeneas translation |
| **1 — Protocol** | Sans-IO transaction machines: apply, rollback, recover, GC, activation; journal ordering and barriers | Crash anywhere, then recover, preserves the kernel invariants, under the storage model (§4) | Lean: machine × crash semantics of the storage model. TLC models stay as bug-finders |
| **2 — Host** | Driver loop, cap-std filesystem adapter, store layout, wire encode/decode, process supervisor, clocks, locks | The storage model's assumptions hold on Linux ext4/btrfs/xfs/tmpfs (WSL2 named separately) | Small enough to read. Every assumption registered. Fault injection, a crash-injecting in-memory storage, power-cut/persistence smokes, property tests of the decoders |
| **3 — Edge** | Deno frontend + IR admission, fetch/HTTP + rate limits, archives, Conda/Pixi, plugins, Docker services, systemd schedules | Untrusted input cannot reach rings 0–2 except as admitted typed values or hash-verified bytes | Sandbox, schema/type admission, content addressing; weekly fuzzing of parsers; e2e |
| **4 — Product** | CLI, rendering, review/saved plans, adopt, doctor, self-update | Behavior users see | e2e flows, golden IR, examples |

Expanding a ring means moving a decision inward. Candidates, in rough order
of value: activation/hook pending-state, schedule lifecycle, service identity
and ownership, sharing/closure admission, update survey. Each move follows
§9 F5.

## 4. Kernel interface and the storage model

The reviewer's key finding (verified in the code): the real boundary is
**observe → decide → re-check at mutation**, not plain command/event.
`ops/mod.rs` plans from observed state and the journal re-validates at
mutation time; `journal/mutation.rs` says outright "this is not CAS".
The interface therefore makes the check explicit. Rollback and managed
blocks show that the machines cannot plan everything up front and then
execute: the next block in a shared file must see the previous block's
actual result (`rollback.rs:187`). So every machine is a loop: **observe
one unit → decide → issue one command → read its result**, with at most
one effect outstanding. Sequential traces keep P2 a plain induction.

```rust
// kernel vocabulary: interned, Copy, no strings/paths/maps
DestId(u32)  Digest([u8;32])  GenId(u64)  TxId([u8;32])  CmdId(u32)
enum Live     { Absent, Obj(Ident), Unsupported }
enum Command  { ReadJournal, ReadCurrent, ReadInventory(Scope), ReadLedger, Observe(DestId),
                CapturePrior{dest, expect}, PublishRecord{role, rec},
                Mutate{id, dest, expect, effect}, Seal{dest, file}, EnsureParent(DestId),
                Reserve{gen}, RaiseFloor(GenId), PublishGeneration{gen, manifest},
                Flip{expect, to}, Barrier(Scope), Unlink(Name), RenameNoReplace{from,to},
                RemoveTree(Name), RunChecks, RunHook{intent, attempt},
                PersistIntent{intent, state}, Finish(Outcome), Abort(Refusal) }
enum Event    { Start, JournalRead{..}, CurrentRead(..), Observed{dest, live}, Inventory{..},
                LedgerRead(..), PriorCaptured{..}, Reserved(..), Landed{id, live}, Done(CmdId),
                Failed{id, why: Drift|Io(i32)|Unsupported}, ChecksDone(bool), HookDone{..} }
fn recover_step(RecoverState, Event) -> (RecoverState, Command)   // likewise gc_, activation_,
fn apply_step(ApplyState, &Desired, Event) -> (ApplyState, Command) // rollback_ (embed recover/
                                                                    // activation sub-machines)
```

For `Mutate`/`CapturePrior` the driver opens the parent pinned, computes
`Live` with the same function `Observe` uses, returns `Failed{Drift}` on
mismatch, otherwise acts and returns `Landed{live}`. The *kernel* compares
`Landed` with the intended identity. `Splice` re-reads the latest bytes and
calls the kernel's proved splice. Two-barrier cleanup is `Unlink`+`Barrier`
commands, so its order is proved, not trusted. `Flip` carries `expect`.

**Storage model** (ring 1's only axioms, one Lean file, mirrored by the
`MemFs` crash-injecting storage in ring 2). Inode-based, not path-based,
so O_NOFOLLOW parent pinning and directory receipts (fs id, inode, birth
time) are expressible:

- S1 `rename` within a directory is atomic and replaces the target.
  `RENAME_NOREPLACE` and exclusive `mkdir` fail if the name exists.
  `remove_dir_all` is *not* atomic; it is a sequence of unlinks.
- S2 A file's data is durable after `fsync(file)`. A name is durable after
  `fsync(parent dir)`.
- S3 Two crash kinds. **Kill**: the process dies, the page cache survives,
  so later processes can observe non-durable state (why the code seals what
  it observes before acting). **Power loss**: an *arbitrary subset* of
  non-durable operations is lost, consistent only with completed barriers.
  No prefix-closure is assumed. Each P-theorem is proved for both.
- S4 One lifecycle writer at a time: apply, rollback, gc and recover hold
  `LifecycleSession` (flock, released when the holder dies).
- S5 External writers exist and are only *detected*. Between the driver's
  re-check and its act there is a window. "Never overwrite unowned data" is
  proved for every external write the re-check sees; the window is a named
  residual risk in the claim list.
- S6 The filesystem's name equivalence is no coarser than the kernel's
  destination-key equivalence. (A case-insensitive mount such as WSL
  `/mnt/c` violates this today unless keys fold consistently; see §5a.)
- S7 Content identity is a SHA-256 digest; equal digests mean equal bytes
  (collision resistance). `#print axioms` cannot show this, so it is listed.

`gripsack-fs/src/persistence_model.rs` is a dirty-set checker over
traces, not a storage. `MemFs` is new (inode table, volatile and durable
layers, kill and power-loss enumeration, ~1–1.5k lines) and reuses its
fault-boundary vocabulary and every-cut tests.

## 5. Theorems (the end-state claim list)

Revised after the F2 design study (2026-11-08), which found the first
draft's K1, K2, K4, K6, P2 and P4 false or unprovable as stated.

Ring 0 (all inputs):

- K1 Ownership, per unit: a copy/template destination is mutated only if
  Fresh, consented TakeOver, or its live identity equals the last managed
  write; a managed block only if intact; a remove only if the live object is
  the deployed one; rollback's restore only via the three-way rule. Preserved
  drift is never overwritten. (Owned links currently violate this; §5a.)
- K2 Fidelity, per unit: for each leaf destination the decision is a
  function of (desired, live, previous record, consent); executing it to
  `Landed` yields the intended identity or a refusal. Preserve/Refuse never
  claim the desired state. The unjournaled effects (parent mkdirs,
  selection reservations, staging dirs) are listed, not hidden.
- K3 Commit classification: Committed iff current = target, plus the
  existing biconditionals (CLASSIFY-001); numeric order never grants commit
  authority.
- K4 GC, relative to the `Inventory` event: deletions ⊆ candidates − roots;
  current and every retained generation are never deleted; nothing is
  deleted while recovery or activation is pending. K4′: roots include the
  closure of every retained manifest. (Gathering workspace/schedule roots is
  ring-3 input and is tested, not proved.)
- K5 Managed-block splice: foreign bytes outside markers are preserved
  exactly; splice is idempotent; block generation is in the kernel.
- K6 Generations: new generation numbers exceed the durable high-water
  floor; every flip carries a never-reused TxId; published generations are
  immutable. (Rollback creates no generation; it reselects with a fresh
  TxId. Vacuous applies create none.)

Ring 1 (under S1–S7, separately for Kill and Power loss):

- P1 Mutation never precedes durable intent and prior evidence.
- P2 After any execution with crashes at arbitrary points (unbounded),
  followed by recovery: the K-invariants hold, `current` names a durably
  published generation, and each destination holds its prior, its intended
  identity, or a value gripsack did not produce — never a gripsack-produced
  intermediate.
- P3 Cleanup never destroys evidence needed after a later crash; a
  malformed or quarantined record yields no effects.
- P4 A hook runs only after its `Started` record is durable and while its
  selection is current; an interrupted `Started` attempt replays under the
  same intent ID; `Failed`/`Succeeded`/`Superseded` are terminal. Hooks are
  never auto-rolled back (0001 §3.8). (Not "at-least-once": failed intents
  are not retried and superseded intents run zero times.)
- Liveness ("eventually recovers") stays claim-gated and TLC-only.

`grip doctor --guarantees` and the website list exactly these, generated from
the Lean sources with their axioms.

### 5a. Defects found while designing the theorems

| ID | Defect | Evidence | Fix lands in |
|---|---|---|---|
| FND-1 | A recorded (owned) symlink is re-pointed whatever is live: if the user replaced it with their own file, the next apply overwrites it. Breaks the drift promise for links | `gripsack-policy/src/ownership.rs:103` (`recorded ⇒ Link`), `ops/plan/link.rs:18`; `lineage_model.rs:199` encodes it as intended | F2 step 6 (planner), with a failing-before e2e |
| FND-2 | Rollback backfills `generations/<n>/env/profile.sh` into an existing generation and deletes the legacy env file outside the journal | `gripsack-exec/src/env.rs:27-49`, called from `rollback.rs:72` | F2 step 8 |
| FND-3 | Suspected: on a case-insensitive mount, re-spelling only a destination's case deploys the new spelling, then prunes the old one — the same inode | admission folds (`deploy/destinations.rs:26`) but prune compares raw `PathBuf` keys (`apply.rs:443-486`) | needs a reproducer first; F2 step 7 |

## 6. Target layout

```text
crates/
  gripsack-kernel/   rings 0+1: pure, no_std+alloc, no IO, no float, no dyn;
                     Aeneas-shaped. Types, decisions, protocol machines.
  gripsack-host/     ring 2: driver, cap-std fs (absorbs gripsack-fs), store
                     layout + wire codecs (journal/manifest/selection), process
                     supervisor (absorbs gripsack-process), locks, clock, trace
  gripsack-ir/       ring 3: schema types + admission (drops policy dep)
  gripsack-sources/  ring 3: fetch, archives, Conda/Pixi, pins, sharing,
                     rate/retry budgets (absorbs gripsack-fetch, -conda,
                     exec/workspace/{conda,sharing,pins,content})
  gripsack-runtime/  ring 3: services (Docker), schedules (systemd), tasks,
                     launchers, run/shell (exec/workspace/{consumer,schedules,
                     hooks,image}, exec/services)
  gripsack/          ring 4: CLI, render, review, adopt, doctor, config
  griplint/          unchanged (standalone linter)
proofs/              Lean 4 lake project
  Extracted/         Aeneas output, committed; CI checks regeneration matches
  Storage.lean       S1–S7
  Kernel/*.lean      K1–K6
  Protocol/*.lean    P1–P4
  Claims.lean        generates the claim list + axiom audit
specs/               ≤ 5 TLC models kept as counterexample finders
```

`gripsack-policy`, `gripsack-store`, `gripsack-exec`, `gripsack-fs`,
`gripsack-process`, `gripsack-fetch`, `gripsack-conda`, `gripsack-config` and
`gripsack-trace` dissolve into the crates above. Dependencies flow strictly
inward: `gripsack → runtime/sources → host → kernel`, and `ir → kernel`.
`check_architecture.py` becomes a `cargo-deny`/workspace-lint rule enforcing
that direction. The kernel has no dependencies at all.

`gripsack-review` is the one optional extra crate. Semantic review/saved plans
(~7k lines) splits out only if it stays data-only after F4; otherwise it lives
in the CLI.

## 7. Toolchain and gates

All tooling runs in containers built with BuildKit (`docker buildx`, cache
mounts); nothing is installed on the host.

| Compose service | Contents | When |
|---|---|---|
| `test` | fmt, clippy -D warnings, cargo test (includes crash-injection storage and decoder properties) | every PR |
| `ts-test` | Deno frontend tests | every PR |
| `e2e` | real binary flows (offline) | every PR |
| `proofs` | pinned Lean + lake + Charon + Aeneas: regenerate `Extracted/`, check no drift, `lake build`, axiom audit, one calibration mutant per theorem family | every PR touching `gripsack-kernel` or `proofs/`, and on tags |
| `model` | the ≤5 TLC bug-finders | weekly + on demand |
| `fuzz` | 3 parser targets (archive, manifest/journal decode, IR admission) | weekly; a crash opens an issue |
| `release` | musl tarball | tags |
| `diagram` | d2 render of `docs/diagrams/*.d2` | on demand |

The required gate is the owner's **local Buildbot** qualification, not
GitHub Actions (hosted queues ran up to 14 h; 0053). `ci_infra qualify
<checkout>` runs `.ci/pilot.json` and posts `local-practical-qualification`,
which `main` requires. F0 slims that gate list to: test, ts-test, e2e/flows,
audit, package (core + SDK + helper) and an installed-consumer smoke; F1 adds
`proofs`. A release is that qualification green on the exact source, then
one local release command that builds, verifies and publishes. Lean/Charon
images are pinned by digest. Charon's nightly rustc lives only in the
`proofs` image; the shipped binary still builds on stable musl with rustls.

## 8. Trim register

"Now" items are no-regret and land in F0. "After X" items are deleted the
moment their replacement is green.

| Artifact | Fate | When |
|---|---|---|
| `verification/reports/` (51 MB), `delivery.json`, `continuation-handoff.md`, `release-handoff.md` | delete; history stays in git | now |
| `scripts/delivery_*`, `tlaps_*`, `verus_evidence.py`, `check_ci_gate.py`, `local_ci*.py`, `release_completion_witness.py` | delete | now |
| `scripts/release_*.py` + tests (~6k lines) | replace with one `release-core.yml` (build → verify tarball → publish) and a ≤150-line installed-consumer smoke | F0, after one release goes out on the new flow |
| `verification/guarantees.md` | freeze as historical; replaced by generated claims | now (freeze) / F3 (replace) |
| Fuzz in the per-push gate + waiver machinery (`GRIPSACK_FUZZ_WAIVER`, `REL-*` IDs) | remove; weekly only | now |
| Fuzz targets `merge`, `store_gc` | delete | after K4/K5 proofs |
| GNU duplicate build, two-build repro, `mutants` campaign jobs | delete jobs (repro stays an on-demand script) | now |
| TLAPS proofs (`*Proofs.tla`), `check_tlaps.py`, `tlaps` service | delete | after P1–P3 in Lean |
| Remaining TLA+ | keep ≤5 TLC models that found real bugs (Transaction, Activation, UpdatePublication, ProcessSupervision, CredentialRouting candidates); delete the rest and the `MC`/bridge/witness modules | F3 |
| Verus annotations, `check_verus*.py`, `verify` service | delete per kernel as its Lean proof lands; no Verus at end of F2 | F2 |
| Rust explorers (`*_model.rs`) | replace with the crash-injecting storage harness driving the real machines; delete | F2 |
| `rate_limit`, `process_budget`, `retry_budget`, `operation_budget` proofs | move to ring 2/3 as plain Rust plus property tests; no proof (no f64 in the kernel) | F2 |
| Plans 0018–0055 | stay as history; STATUS shrinks to one row each plus this plan's ledger | F0 |
| e2e | keep; delete cases that only back a receipt or a removed feature | F4 |
| AGENTS.md | rewrite build/test, hard rules and "Plan 0048 contract" sections to this plan | F0 |

## 9. Phases

Each phase lands on `main` in small PRs with releases continuing under the
gate in §7 (until §7 exists, under 0055's essential gates). Nothing freezes
releases.

**F0 — Trim and reset the gates (≈1 week).** The "now" rows of §8. Collapse
CI to `gate` = test, ts-test, e2e. Rewrite AGENTS.md. One release goes out on
the simplified flow before the old release scripts are deleted.
*Exit:* CI ≤ 4 jobs; `scripts/` ≤ 3k lines; release from tag with no manual
evidence.

**F1 — Aeneas pilot (2 weeks, time-boxed; go/no-go).** Create
`gripsack-kernel` holding *moved* (not copied) `selection::classify`, one
journal transition (`RecordPublication`) and GC retention admission. Wire
production to call them. Add the `proofs` service, translate, and prove the
existing Verus postconditions in Lean. Axiom audit. One mutant per function
must fail.
*Falsifiers:* Charon/Aeneas cannot translate the needed subset without
contorting the code; a proof needs an axiom equivalent to its claim; the
generate-and-check loop exceeds ~10 min in CI.
*No-go fallback:* the kernel crate and sans-IO shape still land; proofs stay
in Verus against a Lean spec, with that link labelled *tested*. Ring 1 still
moves to Lean over the spec. The owner decides at the F1 review.

**F2 — Kernel extraction (≈8–10 weeks; the first estimate ignored the
~3.7k-line directory domain and ~3k lines of non-transaction Verus).** Ten
mergeable steps; `main` stays releasable throughout. Old and new engines run
side by side behind a debug-only `GRIPSACK_ENGINE=legacy|kernel` switch, with
the crash e2e run both ways for one release before the legacy path goes.

1. Vocabulary + `gripsack-host::Storage` trait with `RealFs` and `MemFs`.
   *Exit:* `MemFs` finds the 0.19.1 resurrection bug when the entry barrier
   is removed; ported persistence-model cases pass.
2. Recover machine (decide, order, cleanup, seal); directory records still
   delegated to legacy code. Delete `journal/model.rs`, `repeated_model.rs`.
3. Leaf + journal-run sub-machine; port `journal_protocol` to Lean.
4. GC machine incl. root derivation from manifests; `retention` Verus goes.
5. Activation machine (ledger order, authorize/supersede, attempts).
6. Planner into the kernel (copy/template/link/merge/remove/restore per
   unit; block generation on bytes). Fix FND-1. `lineage_model`,
   `ops/model`, `mode_model` become kernel property tests, then go.
7. Apply machine end to end (`mutated` flag replaces `ReportKind`); directory
   transitions are an opaque `DirectoryStep` still run by legacy code.
   Reproduce/fix FND-3.
8. Rollback machine (per-unit loop, fresh selection); fix FND-2.
9. Directory domain fenced into host as a sub-protocol; its TLC models stay
   as bug-finders. Moving it into the kernel is F5 work — the largest risk.
10. Dissolve `gripsack-store`/`-policy`/`-fs`; port or demote the remaining
   policy kernels (`semantic`, `graph`, `task`, `target`, `conda_admission`,
   `workspace_command`, `update_survey`); delete Verus.

Each step's exit runs the matching e2e group (persistence matrix, recovery,
GC, activation, ownership, merge, rollback, directory evolution).
*Phase exit:* K1–K6 proved; no Verus; `gripsack-store`, `gripsack-policy`,
`gripsack-fs` and `gripsack-process` gone.

**F3 — Protocol proofs (≈3–4 weeks).** `Storage.lean` (S1–S7). P1–P4 over
the translated machines. Generated claims and `grip doctor --guarantees`.
Delete TLAPS and the excess TLA+.
*Exit:* P1–P4 proved without axioms beyond S1–S7; ≤5 TLC models; website
claim page generated.

**F4 — Feature crates (≈2–3 weeks, can overlap F3).** Split the remainder of
`gripsack-exec` into `gripsack-sources` / `gripsack-runtime` / CLI; dissolve
`gripsack-config` and `gripsack-trace`; enforce dependency direction; prune
e2e.
*Exit:* layout of §6; no crate over ~15k lines.

**F5 — Expand rings (ongoing).** Per candidate: move the decision into the
kernel, state the theorem, prove it, delete the tests and models it makes
redundant. One PR per candidate.

**Handover reconciliation.** Done early (read-only), see
[0056a](0056a-handover-reconciliation.md). After F0, produce a landed/pending/dropped table
for the original handover (0048 R1–R8 and M-V*, 0049 H0/A0, 0054 "original
handover completion"). Every pending item is re-homed into a ring, dropped as
superseded by this plan, or kept as product work.

## 10. Risks

| Risk | Mitigation |
|---|---|
| Aeneas cannot handle collections, traits or loop shapes the kernel needs | Pilot first. The kernel uses `Vec`, sorted vectors instead of maps, explicit loops, and error enums. F1's fallback keeps every other benefit |
| Lean proof effort exceeds TLAPS/Verus effort | P-theorems are rewritten fresh and small against the sans-IO machines, not ported obligation by obligation. Time-box each family; an unproven theorem is listed as such, never claimed |
| Rewriting the transaction core introduces regressions | The crash-injecting storage harness and the existing e2e/persistence flows run against old and new code paths during F2; switch over per machine |
| Charon nightly drifts | Pinned image digest; a regeneration diff is a reviewable PR |
| The external-writer window (S5) is misread as solved | It appears in the generated claim list as a named residual risk |
| Trimming deletes something that was catching bugs | The survival rule's case (c): TLC models with a recorded real bug survive; deleted harnesses stay in git history |

## 11. Non-goals

- Proving Deno, Docker, systemd, Conda, rustls, the Rust compiler, or the OS.
- Proving liveness or physical power-loss behavior.
- Concurrency proofs of the scheduler's thread coordination (stays a Loom test
  in ring 2; scheduling *decisions* may move into the kernel).
- Platform expansion beyond Linux/WSL x86_64.
- Signing and supply-chain policy changes.

## 12. Decisions recorded

| Decision | Choice | Alternative left open |
|---|---|---|
| Ring-1 proof language | Lean (one language across the seam) | TLA+/TLAPS for ring 1 (owner's first sketch). Reversible until F3 |
| Kernel↔code link | Aeneas translation of shipped Rust | Lean spec + Verus code proofs (F1 fallback) |
| Releases during the epic | Continue; this plan removes release gates rather than adding them | — |
| Fuzzing | Keep 3 edge parsers, weekly, never release-blocking | Remove fully |
| Review | Independent model review 2026-11-08 adopted: observe/re-check interface, S5 explicit, rate limits out of kernel, storage model reused, pilot falsifiers. Overruled: retaining Verus/TLAPS as sunk cost | — |

## F0b record

Scope: the local CI and release machinery (stream F0b, branch
`foundation/f0-ci-release`). §7's single hosted `gate` job does not apply:
the required check on `main` is the local Buildbot qualification
(`local-practical-qualification`); hosted workflows are manual diagnostics.

**Deleted** (≈8.5k lines; `scripts/` 14,559 → 6,566 Python lines):

| What | Lines |
|---|---|
| `release_{admission,evidence,hosted,inventory,operator,prepare,publish,verify}.py` + tests (`test_release_manifest.py` included) | 5,106 |
| `check_ci_gate.py`, `check_delivery.py`, `delivery_{scope,evidence,checker_calibration}.py`, `release_completion_witness.py` | 1,848 |
| `local_ci_{checkpoints,partition,persistence,runtime}.py` + test | 1,067 |
| `smoke_package_registry.py` (unreferenced) | 337 |
| workflows `examples.yml`, `qualify-conda-helper.yml`, `repro.yml`; `ci.yml` 239 → 89 lines (4 manual jobs, no waiver/gate aggregation) | 363 |
| compose `gnu-test`, `mutants` services and Dockerfile stages; stale `ldd` probe in the `release` stage | — |

Kept on purpose: `release_manifest.py`, `release_consumer.py`,
`release_installer.py` — `install.sh` and the embedded self-updater
(`crates/gripsack/src/commands/self_update/installer.sh`) are generated from
them byte-for-byte, so they are product code; `release-core.yml` stays as a
no-op because the installer names it as signer identity. `fuzz.yml` stays
manual-only (owner: never dispatch fuzz). `repro` service and
`scripts/check_reproducible.sh` stay on demand. `model`/`tlaps`/`verify`/`fuzz`
services untouched.

**Gates** (`.ci/pilot.json`, each `python3 scripts/local_ci.py <gate>`):
`test` (compose `test` + `scripts/test_*.py`), `ts-test`, `e2e`, `audit`
(cargo audit, both locks), `package` (`release.py prepare`: core tarball from
the Dockerfile `release` stage, Conda helper built fresh with `--check`
against the committed pin, SDK `npm pack`, `install.sh`), `smoke`
(`release.py verify`: exact inventory, checksum sidecars, static ELF and
embedded cargo-auditable SBOM parsed from the packaged bytes, `grip
--version`, helper manifest/licences, SDK name/version, installer bytes, then
the installed-consumer smoke `local_artifact_consumers.py` against the pinned
website/example commits). `proofs` is wired in `local_ci.py` and named under
`external_inputs.pending_gates`; add it to `gates` once the service lands.

**Release sequence** (tag `core-vX.Y.Z`; `ts-v*` tags ended at 0.46.0):

```sh
python3 -m ci_infra qualify <clean checkout on a local branch>   # → candidate id
python3 scripts/release.py status <candidate_id>                 # posts local-practical-qualification
python3 scripts/release.py publish <candidate_id> --notes N.md   # dry run; add --yes to publish
```

`publish` refuses an existing tag, a commit not on `origin/main` or a missing
status; copies the qualified bytes from the ci-infra archive (digest-checked
against `qualification.json`), re-runs the artifact checks, creates the
release at the qualified commit and reads every asset back. Asset set = the
0.51.0 set minus evidence files (`gripsack-release-X.json`,
`*-runtime-evidence.json`, qualification-bundle tarball): core tarball +
`.sha256`, helper + `.sha256` + `.manifest.json` + 3 licences, SDK `.tgz`,
`install.sh`.

**Validation** (2026-11-08, worktree at `277ba2f`): `local_ci.py test` 0,
`ts-test` 0, `audit` 0, `e2e` 0 (706 passed, 12 deselected, 25 min),
`package` 0 (fresh helper reproduced pin `60d66f94…`; tarball, SDK and
installer produced user-owned), `smoke` 0 (88 consumer commands, 227 installed
flows). `release.py verify --no-smoke` 0. `publish` against the archived
0.51.0 candidate `8c0bc53c…` refused (tag exists); its collect+verify path
passed on those archived bytes. Real Buildbot qualification **not run**:
`ci_infra storage-check` failed (state volume 62.5 GB free < 64 GiB floor);
a clean checkout is ready at `ci-checkouts/gripsack-foundation-f0` (branch
`qualify/f0-ci-release`).

**Left for the owner**: run that qualification and `release.py status` once
storage allows; one release on this flow before calling F0 done (§9);
`release_manifest.py` still names `REL-*` exclusions and lazily imports deleted
modules in `validate_bundle` (dead in the installer path) — clean it together
with the embedded installer in `crates/`; the mutant-calibration scripts that
only the removed `mutants` stage ran (`check_{journal_admission,
journal_protocol,gc_roots,rate_admission,http_budget,process_bounds,
update_survey}.py`) are now unreferenced; per-attempt compose project names
leave tagged `ci-…-<service>` images behind (layers shared).
