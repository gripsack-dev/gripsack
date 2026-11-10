# 0056 — Verified foundation: a Lean-proved kernel, rings outward, and a trim

- Status: approved by the owner 2026-11-08; F0 and F1 landed in 0.52.0 (F1: GO); F2 in progress (step 1 done)
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
| FND-1 | A recorded (owned) symlink is re-pointed whatever is live: if the user replaced it with their own file, the next apply overwrites it. Breaks the drift promise for links | `gripsack-policy/src/ownership.rs:103` (`recorded ⇒ Link`), `ops/plan/link.rs:18`; `lineage_model.rs:199` encodes it as intended | **Fixed** in F2 step 6 part 1 (§15), together with the store-prefix widening (design review finding 3) |
| FND-2 | Rollback backfills `generations/<n>/env/profile.sh` into an existing generation and deletes the legacy env file outside the journal | `gripsack-exec/src/env.rs:27-49`, called from `rollback.rs:72` | **Fixed** in F2 step 8 (§16): the backfill is gone, a target without its own profile is refused |
| FND-3 | Suspected: on a case-insensitive mount, re-spelling only a destination's case deploys the new spelling, then prunes the old one — the same inode | admission folds (`deploy/destinations.rs:26`) but prune compares raw `PathBuf` keys (`apply.rs:443-486`) | needs a reproducer first; F2 step 7 |
| FND-4 | A managed block whose payload has a carriage-return run before a line break (`a\r\r\nb`) never matches its own marker sha: content normalization (trim, then `\r\n` → `\n`) is not a fixed point of render-then-normalize. The second apply rewrites the block with CRLF line ends and reports it "hand-edited"; every later apply still cuts a new generation for a no-op | `managed_blocks/mod.rs` `normalized_content` + `str::lines` rendering at `524f576`; e2e reproducer in §17.1 | **Fixed** in F2 step 6 part 2 (§17) |

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
*Exit:* six required gates; `scripts/` ≤ 3k lines excluding the Verus/TLAPS/TLC
runners that retire in F2/F3 and the installer generators (product code);
release from qualified bytes with no hand-assembled evidence.

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
   *Exit:* `MemFs` finds the 0.19.1 (no barrier) and 0.19.2 (trailing
   barrier only) resurrections; ported persistence-model cases pass.
   **Done 2026-11-09** (`crates/gripsack-host`): the shipped two-barrier
   cleanup passes 574 cuts / 1749 kill and power-loss images; the 0.19.2
   order fails at cut 23 (entry unlink lost, marker unlink kept). MemFs
   loss granularity is one update (one write call, one entry, one side of
   a cross-directory rename); it is deliberately pessimistic about renames
   of never-durable sources, so a step-2 finding there is checked against
   S1–S3 before it is called a bug. Not yet modelled: S5 external writers
   (step 2 adds the injection hook) and directory birth time (before step 9).
2. Recover machine (decide, order, cleanup, seal); directory records still
   delegated to legacy code. *Exit:* `journal_cleanup.rs` drives the kernel
   recover machine through a driver generic over `Storage` (not a
   hand-written recover) over MemFs, including an S5 external-write hook;
   then delete `journal/model.rs`, `repeated_model.rs`. Also closes the
   unrestorable-entry wedge (0058 item 1).
   **Done 2026-11-09** (branch `foundation/f2-recover`, stacked on step 1);
   record in §14.
3. Leaf + journal-run sub-machine; port `journal_protocol` to Lean.
   **Done 2026-11-10** (branch `foundation/f2-journal`): the kernel journal-run
   machine (`journal_run`: marker, per-destination leaf protocol, flip,
   cleanup) drives every product leaf mutation and the run's completion over
   `RealFs`; P1 and the cleanup half of P3 proved; the Verus
   `journal_protocol` (`CleanupProgress`) and `admit_mutation` deleted;
   rollback's writes go through the leaf protocol and are crash-explored over
   MemFs; record in §18.
4. GC machine incl. root derivation from manifests; `retention` Verus goes.
5. Activation machine (ledger order, authorize/supersede, attempts).
6. Planner into the kernel (copy/template/link/merge/remove/restore per
   unit; block generation on bytes). Fix FND-1. `lineage_model`,
   `ops/model`, `mode_model` become kernel property tests, then go.
   **Part 1 done 2026-11-09** (branch `foundation/f2-ownership`, taken ahead
   of steps 3–5 by owner priority): the per-unit ownership decision
   `ownership::decide_unit` for copy/template/link/remove/restore, K1 proved,
   FND-1 and the store-link widening fixed, `lineage_model` moved to a
   kernel property test; record in §15. Remaining: merge (K5, block
   generation on bytes), and `ops/model`/`mode_model` (they check the host
   lifting, which the kernel cannot see).
   **Part 2 done 2026-11-09** (branch `foundation/f2-splice`): the managed
   block in the kernel (`block`: marker grammar, scan, normalization,
   rendering, splice, upsert/remove, and the per-block decision), K5 and K1
   for blocks proved, rollback's block restore checked against its record
   (§16.7 deviation 2 closed), FND-4 fixed; the Verus splice/scanner,
   `MergeBoundary.tla` and the `merge` fuzz target deleted; record in §17.
7. Apply machine end to end (`mutated` flag replaces `ReportKind`); directory
   transitions are an opaque `DirectoryStep` still run by legacy code.
   Reproduce/fix FND-3.
8. Rollback machine (per-unit loop, fresh selection); fix FND-2.
   **Done 2026-11-09** (branch `foundation/f2-rollback`, stacked on step 6
   part 1, taken ahead of steps 3–5/7 by owner priority): the kernel
   rollback machine, K2 for rollback proved, exact restore or refusal
   before the flip (design review finding 2), FND-2 fixed; record in §16.
   Delegated, not decided by the kernel: managed blocks (K5) and the
   destinations a directory transition owns (step 9).
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
| Kernel↔code link | Aeneas translation of shipped Rust — **F1 GO (2026-11-08)**, §13 | Lean spec + Verus code proofs (F1 fallback, not needed) |
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

## 13. F1 pilot record

- Branch: `foundation/f1-aeneas-pilot` (based on `187d741`); date 2026-11-08.
- Question: can the shipped Rust kernel be translated by Charon/Aeneas into
  Lean 4 and proved there, end to end, inside docker, within the falsifiers
  of §9 F1?

### 13.1 Pins (every input of the `proofs` image, `proofs/Dockerfile`)

| Input | Pin | How |
|---|---|---|
| Aeneas | release `nightly-2026.11.07-c5eb4a2` (commit `c5eb4a28cef65e0aa27743105308243e535a2a2b`), `aeneas-linux-x86_64.tar.gz` | `ADD --checksum=sha256:14fbaed4ce8624cf3d668f887d6ebb237a59cc37d7498c792d9f3f97332cbb25` |
| Charon | commit `8cae8fd1905946de61147a65ecdd7122cc6b491b` (Aeneas's `charon-pin`; LLBC version 0.1.313) | built from source in the image: no Charon release was cut at that commit (the nightlies before/after carry 0.1.313/0.1.314) |
| Charon's rustc | `nightly-2026-09-17` + `rustc-dev llvm-tools rust-src miri` (Charon's `rust-toolchain`) | rustup in the image only; the shipped `grip` still builds on stable musl |
| Lean | `leanprover/lean4:v4.35.0` (`proofs/lean-toolchain`, = Aeneas's `backends/lean/lean-toolchain`) | elan `v4.2.4`, `ADD --checksum=sha256:42b94d42…431f63` |
| mathlib | `v4.35.0` → rev `56a005855ea17e0b0958f73ec801ae140b0a1679` | `proofs/lake-manifest.json` (generated once with `lake update` in the image, committed) |
| base image | `ubuntu:24.04@sha256:224a1869…da254` | same digest as the `verify` stage |

### 13.2 What moved into `crates/gripsack-kernel` (785 lines, no deps, `no_std` + `alloc`)

- `selection`: `Classification`, `TransactionId`, `SelectionIdentity`,
  `RecoveryFacts`, `classify` (from `gripsack-policy::selection` + `lib.rs`).
- `generation`: `GenerationId`, `GenerationList` (admission) (from
  `gripsack-policy::generation`; `GenerationInventory` dropped — the owned
  list is borrowed instead).
- `retention`: `GcAdmission`, `admit_gc`, `contains_generation`,
  `plan_prune`, `same_bytes`, `contains_bytes`, `plan_delete` (from
  `gripsack-policy::retention`, whole module).
- `journal`: `RecordRole`, `PublicationAction`, `RecordPublication`,
  `DurableRecord`, `MutationAuthority`, `admit_mutation` (from
  `gripsack-policy::journal_protocol`; `CleanupProgress` stays in Verus).
- Production callers (`gripsack-store`, `gripsack-exec`) call the kernel
  crate; the Verus versions are deleted and `scripts/check_verus.py` lost
  the `recovery`, `generation-inventory`, `retention`, `journal-publication`
  and `journal-mutation-authority` families and their nine mutants (the
  floor moves 410 → 384). `scripts/check_architecture.py` protects the
  kernel with an empty dependency set.

### 13.3 What Charon/Aeneas handled

Everything in the four modules, first try, except nested borrows (below):
structs, fieldless and payload enums, `Option`, `match` with guards (after
Charon's match reconstruction), `[u8; 32]` arrays, slices, `Vec` (`push`,
deref), `&mut self` methods (translated to returning the new value), `while`
loops with early `return` (translated to a `loop` combinator over
`ControlFlow`), `usize` arithmetic with overflow/underflow as `Result`
failures, and a `pub const`. Translation of the crate takes ~1.2 s (Charon
~1 s of cargo build, Aeneas 1.2 s); output is 822 lines of Lean, zero
axioms, fully readable (`proofs/Extracted/GripsackKernel.lean`).

### 13.4 Code contortions required (all recorded in the source)

1. **Nested borrows.** `&[&[u8]]`/`&[&str]` is "unsupported nested borrows"
   in Aeneas. Path identities are now `&[Vec<u8>]` (owned bytes), and
   `plan_delete` returns candidate positions (`Vec<usize>`) instead of
   borrowed `&str`s; the exec adapter owns the str↔bytes seam.
2. **Owned facts.** `RecoveryFacts<'a>` (borrows in an ADT) became an owned
   `Copy` struct; `MutationAuthority<'a>` owns the consumed entry instead of
   borrowing the marker. (Shared borrows in ADTs are not a translation
   blocker, but owned values keep the Lean types first-order.)
3. **No derives on kernel types.** Charon translates every impl in a module
   it starts from, derived or not; `#[derive(Debug, PartialEq, …)]` dragged
   in `core::fmt` and produced one Aeneas `axiom`
   (`core.option.Option.Insts.CoreFmtDebug.fmt`). The kernel modules carry
   no derives; `Debug`/`Clone`/`Copy`/`PartialEq`/`Eq`/`Ord`/`Hash`/
   `Display`/`FromStr`/`Deref`/`IntoIterator` are hand-written in
   `convenience.rs`, which the translation never reaches
   (`[package.metadata.charon] start_from`). `PartialEq` delegates to the
   kernel's `same`, so `==` and the proved identity agree by construction.
4. **Explicit equality.** Equality the theorems depend on is a kernel
   function (`same`) — a byte loop for `TransactionId`, field comparison
   for `GenerationId`, nested matches for enums — never a derived trait,
   so the proved call graph reaches only Aeneas's standard-library models.
5. **Identifier/namespace clash.** A Rust local named `generation` inside
   the `generation` module made Aeneas emit `generation.GenerationId.same …`
   where `generation` is the local → Lean parsed a field projection. Locals
   are not named like their module (`candidate`).
6. **No casts in the kernel.** `keep: Option<u32>` became `Option<usize>`
   (the caller casts) because the `u32 → usize` `UScalar.cast` made the
   Lean side unwieldy. Semantics unchanged.
7. **Lean side.** Aeneas's `do`-notation extensions make `name` a reserved
   token and reject tuple patterns in some positions; the audit avoids
   both. The `step` tactic simp-normalises the final goal of a branch
   (re-associates conjunctions, discharges arithmetic), so the loop proofs
   close their last step with an explicit `WP.spec_bind` and `and_intros`.

### 13.5 Proofs (`proofs/Kernel/`, 742 lines of Lean for 34 theorems)

All theorems are total-correctness Hoare triples `f args ⦃ r => P r ⦄`
over the extracted functions (termination and no failure included):

- K3 / CLASSIFY-001: `classify_committed_iff`, `classify_uncommitted_iff`,
  `classify_ambiguous_iff`, on top of `GenerationId.same_spec`,
  `TransactionId.same_spec` (byte loop), `SelectionIdentity.same_spec`.
- K4 (statements per the revised §5): `admit_gc_spec` (RecoveryPending ↔
  pending; CorruptCurrent ↔ current ∉ inventory; Admitted otherwise),
  `plan_prune_spec` (result = oldest `len − keep` identities, in order,
  minus current — hence ⊆ inventory, never current, nothing newer),
  `plan_delete_spec` (position deleted ↔ candidate no root names),
  `strictly_ascending_spec` / `GenerationList.new_spec` (admission ↔
  strictly ascending).
- Journal: `PublicationAction.successor_spec`,
  `RecordPublication.{new,action,acknowledge,finish}_spec`,
  `admit_mutation_spec` — the former Verus `ensures`, verbatim.

Effort: one working day for the whole pilot (toolchain image, kernel move,
proofs, gate); the proofs themselves ~4 hours including learning the
Aeneas `Result`/`loop` model. Hardest part: loop invariants against the
`loop` combinator (`loop.spec_decr_nat`) and the `Usize`/`Nat` coercions;
`scalar_tac` and `step` carry most of it.

### 13.6 Gate (`docker compose run --build --rm proofs`)

Steps: regenerate → `diff -ru` against committed `proofs/Extracted/` →
`lake build` → axiom audit (`#print axioms`-equivalent over every theorem
under `Gripsack.Kernel`, allow-list `propext`, `Classical.choice`,
`Quot.sound`; the 12 headline claims must exist) → mutant calibration
(`proofs/mutants.py`: 14 semantic mutants of the Rust source, each
re-translated and built against the committed theorems; the build must fail
inside the named theorem(s) and nowhere else).

- Axiom audit: `AXIOM_AUDIT ok: 34 theorems, 12 required claims, axioms ⊆
  [propext, Classical.choice, Quot.sound]` — no `sorryAx`, no project axiom,
  no Aeneas external axiom.
- Drift check: the committed translation equals a fresh one in the gate
  (byte-identical; paths are relative to a shadow workspace so the output
  is independent of the checkout location).
- Mutants: 14/14 rejected, each by exactly the expected theorem(s)
  (`PROOF_CALIBRATIONS=14`): three `classify` mutants (generation-only
  commit; fresh run ambiguous; previous-equals-current committed) → the
  CLASSIFY-001 biconditionals; a 31-byte transaction identity →
  `TransactionId.same_loop_spec`; ignoring pending recovery / admitting a
  corrupt current → `admit_gc_spec`; newest-prefix pruning and pruning the
  current generation → `plan_prune_loop_spec` (+ `plan_prune_spec`);
  deleting the roots → `plan_delete_loop_spec`; weak ascent (duplicates
  admitted) → `strictly_ascending_loop_spec`; skipping the file barrier,
  accepting a failed effect, early durability, marker-OR-entry authority →
  the four journal theorems. Loop mutants are attributed to the loop lemma
  (where the invariant breaks); the top-level theorem elaborates against the
  failed lemma, as Lean does.
- Wall time. Cold image build: tools stage 228 s (rustup nightly 25 s,
  Charon from source 77 s, elan+Lean 36 s, Aeneas 20 s) + deps stage 423 s
  (mathlib cache download 169 s, Aeneas library build 253 s) ≈ 11 min, plus
  the first in-image translation (Charon's full-MIR sysroot) ≈ 1 min; the
  images are 9.4 GB (tools) and 19.7 GB (deps). Warm gate run, unloaded
  host: regenerate 2 s, drift check <1 s, `lake build` 13 s, audit 3 s,
  14 mutants ≈ 170 s (each: Charon+Aeneas ≈ 3 s, scratch `lake build`
  ≈ 9 s) → 191 s total; 213 s including compose's image freshness check.
  Measured while the `test` and `e2e` gates ran concurrently.

### 13.7 Other gates

- `docker compose run --build --rm test`: **passed** (`gate passed: fmt +
  clippy -D warnings + cargo test`; workspace step 219 s, conda-helper step
  244 s). The first run failed only in the conda-helper step because its
  own `Cargo.lock` lacked the new `gripsack-kernel` entry (it depends on
  `gripsack-ir` → `gripsack-store`); the lock gained exactly that entry.
  `scripts/check_architecture.py` passes with the kernel's empty
  dependency set.
- `docker compose run --build --rm e2e`: run once at the end; result in the
  branch's final hand-off (see the pilot report).
- `docker compose run --build --rm verify` (Verus, remaining kernels):
  **could not run** — the pinned asset
  `verus-0.2026.09.06.8dea4a2-x86-linux.zip` now downloads as a
  well-formed 456,338,154-byte zip whose sha256 is
  `bef140a1a96994029153dca8c00b1750b9a5a764fb9db2dc68d7bb40e8a29e8a`, not
  the Dockerfile's `13d01e13…`. The same failure reproduces on `main`;
  it is an upstream/pin question for the owner, not a change on this
  branch (overriding the pin was deliberately not done). The driver edits
  (`scripts/check_verus.py`: families and mutants of the moved kernels
  removed, floor 410 → 384, unrelated-lemma calibration re-homed to
  `ownership.rs`) were checked statically: every remaining mutant site
  matches its source exactly once and every family module exists; the
  obligation floor is an estimate (−26 moved obligations) to be confirmed
  on the first green run.

### 13.8 Recommendation against the §9 F1 falsifiers

| Falsifier (§9 F1) | Outcome |
|---|---|
| Charon/Aeneas cannot translate the needed subset without contorting the code | **Not triggered.** One real limitation (nested borrows) and one translator naming clash; the other changes (owned facts, hand-written impls, kernel `same`, usize keep) are shape choices that also make the Lean side first-order and axiom-free. Production semantics unchanged; callers adapted at the boundary. |
| A proof needs an axiom equivalent to its claim | **Not triggered.** 34 theorems, axioms ⊆ {propext, Classical.choice, Quot.sound}; the extraction contains no `axiom` at all; every spec is total correctness. |
| The generate-and-check loop exceeds ~10 min in CI | **Not triggered** on a warm image: 3 min 11 s for the full gate including 14 mutants; ~20 s without mutants. The cold image build is ~11 min and 20 GB, so CI must cache the image (registry push of the `deps` stage, or a weekly rebuild) — the pin table in `proofs/Dockerfile` is the cache key. |

**Recommendation: GO**, with these conditions for F2:

1. Cache the `proofs` image in CI (digest-pinned, rebuilt only when a pin in
   `proofs/Dockerfile` or `lake-manifest.json` changes); never build mathlib
   or Charon in the per-PR path.
2. Keep the kernel in the translated subset by rule: no borrows inside
   ADTs or nested borrows, no derives in kernel modules (impls in
   `convenience.rs`), equality as kernel functions, no locals named like a
   module, no `as` casts, `Vec<u8>`/`Vec<T>` for collections. Put these in
   the `gripsack-kernel` crate docs (done in `lib.rs`) and in F2's review
   checklist.
3. Proof style: loop lemmas via `loop.spec_decr_nat` with an explicit
   invariant; close the last step of a branch with `WP.spec_bind` +
   `and_intros` rather than `step`, so goal shapes stay predictable. Budget
   ~1–2 hours per loop function for a new author, minutes per straight-line
   function.
4. Resolve the Verus pin drift (§13.7) before the next release so `verify`
   runs again for the kernels that have not yet moved to Lean; F2 retires
   it anyway.
5. When upgrading Aeneas/Charon, move the five pins together (§13.1) and
   expect the extraction to change (it is a reviewable PR, the drift check
   enforces it).

## 14. F2 step 2 record — the recover machine

- Branch `foundation/f2-recover` (stacked on `foundation/f2-storage`, PR
  #214); date 2026-11-09.
- Question: can crash recovery be the proved kernel's sans-IO machine,
  driven over `Storage`, wired into the product unchanged except for the
  wedge, with its key properties proved in Lean?

### 14.1 The machine (`crates/gripsack-kernel/src/recover.rs`)

`recover_step(RecoverState, Event) -> (RecoverState, Command)`, one command
outstanding, interned `DestId`/`Digest` from `ids.rs`.

- **State:** `phase` (18 phases: `Start`, `ReadingJournal`, `ReadingCurrent`,
  `SealingSelection`, `SettlingDirectories`, `OpeningLeaves`, `ReadingEntry`,
  `Observing`, `Restoring`, `Sealing`, `Preserving`, `ClosingLeaves`,
  `UnlinkingRecord`, `SyncingRecords`, `UnlinkingMarker`, `SyncingMarker`,
  `Finished`, `Aborted`), the admitted `JournalFacts` (entry and directory
  counts, leading reverse directory roots, `MarkerFacts`), `records`,
  the `current` read, `verdict` (`Pending|Committed|Uncommitted`), `index`,
  `leaves_open`, and the `EntryFacts` under decision (`dest`, `prior`,
  `before`, `intended`, `route: Leaf|Delegated`).
- **Commands:** `ReadJournal`, `ReadCurrent`, `ReadEntry(i)`,
  `Observe{entry, dest}`, `Mutate{entry, dest, expect, effect: Restore(prior)}`,
  `Seal{entry, dest, expect, prior}`, `Preserve{entry, dest}`,
  `Delegate(Committed|BeforeLeaves|AfterLeaves)`,
  `Unlink(Record(k)|Marker)`, `Barrier(Journal|Selection)`,
  `Finish(NoJournal|Committed|Recovered)`, `Abort(Refusal)`.
- **Events:** `Start`, `NoJournal`, `JournalRead(facts)`,
  `CurrentRead(Option<SelectionIdentity>)`, `EntryRead(facts)`,
  `Observed{dest, live}`, `Landed{live}`, `Done`, `Failed(Drift|Io(errno))`.
- **Vocabulary:** `Ident = File(Digest)|Link(Digest)`,
  `Live = Absent|Obj(Ident)|Unsupported`, `Prior = Absent|Obj(Ident)`,
  `Intended = Removed|Obj(Ident)`; `Refusal = Ambiguous | MarkerMissing |
  Malformed | Unrestorable{entry,dest} | Drifted{entry,dest} |
  NotRestored{entry,dest} | Failed | Protocol`.
- **Decides:** classification through the proved `classify` (K3); per entry
  `decide` (the legacy `decide_from` order: intended, prior, immediate
  pre-state; absent restores an existing prior) plus `Refuse` for
  `Live::Unsupported`; seal-observed-prior as `Seal`; the cleanup order
  (every record, journal barrier, marker, journal barrier) and the committed
  run's selection barrier before any evidence goes. Directory records stay
  with the legacy domain: three `Delegate` steps at the points the legacy
  reconcile ran them, and `Route::Delegated` for entries that domain settled.

### 14.2 Driver, storage hook, product wiring

- `gripsack-host::recover::recover<H: RecoverHost>` executes commands over
  `H::Storage`; `RecoverHost` carries the caller's journal wire, destination
  resolution, identity hashing and directory domain. `Observe`, the
  re-check and `Landed` share one `observe` (`Live` + a human `Found`).
- The re-check is placed after staging (temp written, mode set, fsynced)
  and immediately before the rename/unlink, and also guards `Seal`.
- `MemFs` numbers every call (`Operation`) and lands an S5 `ExternalWrite`
  before any chosen one (durably, not crash-explored).
- `gripsack-store` `journal::reconcile` (before apply, rollback, gc) drives
  the machine over `RealFs` (`recover_kernel.rs`). Destination parents
  resolve exactly as before (`dest_capability` or the directory-bound
  parent) and are pinned into `RealDir::pin` (cap-std's `O_PATH` handles
  cannot be fsynced). Notes and errors read as before. The legacy reconcile
  remains behind debug-only `GRIPSACK_ENGINE=legacy`; the e2e runs one
  interrupted apply on both engines.
- `journal::inspect` is a read-only view for `grip doctor` that asks the
  kernel's own `decide`. `docs/operations/storage.md` hosts the user repair
  procedure (that file is otherwise the workstation runbook; it was named
  as the home by 0058).

### 14.3 Proofs (`proofs/Kernel/Recover.lean`; `Audit.lean`: 86 theorems, 19 claims, standard axioms only)

- Identities: `Digest.same_spec` (byte loop), `DestId.same_spec`,
  `Ident.same_spec`, `Live.same_spec`, `Prior.matches_spec`.
- `decide_spec`: `Restore → Restorable` (the live object is `Obj i` with
  `intended = Obj i` or `before = some i`, or `Absent` with a prior),
  `Refuse ↔ Unsupported`, `Unchanged →` the prior is live.
- One total-correctness spec per helper, then `step_spec`: from any state
  satisfying `Inv` (records issued before cleanup barriers; pre-
  classification phases pending; `ReadCurrent` asked before read; the loop
  only under an uncommitted verdict; an uncommitted verdict came from
  `classify` or a markerless journal), every (phase, event) step re-
  establishes `Inv`. `Run` is every execution from `RecoverState::new`;
  `run_inv` gives `Inv` on all of them.
- (a) `mutate_guard`: a `Mutate` answers `Observed{dest, expect}` for the
  entry under decision, carries that observed object as `expect`, restores
  that entry's prior, and the object is `Restorable` — never a foreign one.
- (b) `marker_unlink_after_record_barrier`: the marker unlink is issued only
  immediately after `Barrier(Journal)`, and every `Unlink(Record k)`,
  `k < records`, precedes that barrier.
- (c) `unrestorable_refused` (any state in `Observing`: the answer to
  `Observed{d, Unsupported}` is `Abort(Unrestorable{index, d})`) and
  `mutate_never_unsupported`.
- (d) `mutate_after_classification`: a `Mutate` only under
  `verdict = Uncommitted`, and with a marker only after `ReadCurrent` was
  issued and `classify(marker, current) = ok Uncommitted`.
- Mutants (`proofs/mutants.py`, 19 total, 5 new), each rejected by exactly
  the named theorem: `recover-guard-restores-foreign` (`decide_spec`),
  `recover-guard-wrong-expect` (`on_observed_spec`),
  `recover-cleanup-barrier-swapped` — the 0.19.2 order —
  (`next_unlink_spec`), `recover-mutates-unrestorable` (`decide_spec`),
  `recover-restores-committed-run` (`on_current_spec`). As with the loop
  mutants of §13.6, a mutant is attributed to the helper spec where the
  contract breaks; the headline theorems build on those specs.

### 14.4 Deviations from §4 / the step-2 brief, and why

1. **No `CmdId` on commands/events.** One command is outstanding; each
   event is checked against the phase, and anything else is
   `Abort(Protocol)`. `CmdId` stays in `ids.rs`, unused.
2. **Vocabulary adapted to recovery**: `ReadEntry` (entries are admitted one
   at a time, keeping state first-order), `Preserve` (a kept foreign edit is
   reported, so it is a command), `Delegate` (the legacy directory domain),
   `Unlink(JournalName)` and `Barrier(Scope)` instead of host `Name`s,
   `Seal` also carries `expect`. No `CapturePrior`/`PublishRecord` here:
   those belong to the apply machine (step 3/7).
3. **Total steps.** Counters advance with `checked_add` (a pure `Option` in
   the translation); an overflow is `Refusal::Malformed`. The ITree-based
   `Result` of this Aeneas release makes inverting a failed `+` awkward,
   and totality is the stronger statement anyway.
4. **The wedge is a refusal, not "keep".** Treating a directory as a foreign
   edit and dropping the entry would make progress, but the brief requires
   fail-closed with an actionable stop. The refusal changes nothing; the
   repair is "move it aside, run again".
5. **Drift on the re-check refuses** (`Drifted`): legacy recovery would have
   overwritten a destination changed between its decision and its write.
   The S5 window that remains is measured by
   `recovery_never_overwrites_an_external_write_its_recheck_sees`: two
   operations (an external write landing after the re-check's name lookup
   but before its content read through the open handle, or between that
   read and the rename) — the named residual risk of §4 S5.
6. **Host-side residue (ring 2, tested, not proved):** journal admission
   (quarantine, straggler removal, permission repair, marker admission,
   reverse-root ordering) runs inside `ReadJournal`; parents are resolved
   ambiently as before (`open_or_create`), so recovery can still create a
   destination's missing parents.
7. **The numeric-commit calibration** of `repeated_model.rs` is now a test
   host whose `read_current` lies (the kernel's `classify` cannot be mutated
   from a test); the kernel side is calibrated by the Lean mutants.

### 14.5 Ported and deleted

`journal/model.rs` and `journal/repeated_model.rs` are deleted. Their cases
run in `crates/gripsack-host/tests/journal_cleanup.rs` against the kernel
machine over MemFs, every kill and power-loss cut of the run, then every cut
of the recovery, then recovery to the end: deploy and prune (create,
replace, remove, link↔file, link→link) on fresh and existing machines;
repeated writes (immediate pre-state); two destinations in interleaved and
batched record orders; a user edit after the run's crash, after the
recovery's crash, and both; an ambiguous `current` (refused, journal and
destinations byte-identical); the numeric-commit mutant (caught). Plus the
step-1 calibrations (0.19.1 and 0.19.2 cleanups resurrect after power loss;
kill alone does not), the wedge (refused by name, state untouched, repair
completes) and the S5 window.

## 15. F2 step 6 part 1 record — the ownership decision (K1)

- Branch `foundation/f2-ownership` from `main` at `7f1408c`; date
  2026-11-09. Taken before steps 3–5 because the owner prioritised the
  guarantees users feel (design review finding 1).
- Question: can the per-unit "may gripsack touch this destination?"
  decision be the proved kernel's, with the existing planner calling it,
  and does that fix FND-1 and the `$GRIPSACK_HOME` link widening?

### 15.1 Step 3 is not a prerequisite

Step 3 is the leaf + journal-run sub-machine: the *effect* protocol (capture
prior, publish record, mutate under re-check). The ownership question is
answered before any of that, from three identities and consent, and the
existing journal re-check already pins the decision to the observed object
(S5). So the decision is a pure function the planner calls; no machine is
needed, and K1 as stated in §5 is a property of that function. When step 3
and step 7 land, the apply machine calls the same function per unit.

### 15.2 The decision (`crates/gripsack-kernel/src/ownership.rs`)

`decide_unit(&Recorded, &Live, &Intended, &Consent) -> Decision`, reusing
the recover vocabulary (`Ident = File(Digest)|Link(Digest)`,
`Live = Absent|Obj|Unsupported`, `Intended = Removed|Obj`).

- `Recorded = Unrecorded | Observed | Wrote(Ident)`: no record; a preserved-
  drift observation (confers nothing); the exact object gripsack last wrote.
  The host derives it from the previous generation's entry **and its store
  root**: an owned link's recorded identity is the link `store_path/from`, a
  copy/template's is its manifest hash (legacy template receipts admitted
  only by an exact match with the live file). A record whose hash is not a
  SHA-256 identity is `Observed`.
- `Consent { take_over: Withheld | Granted }`: the only consent the product
  has (`--take-over`, `--take-over=<dest>`, the plan/adopt adoption set),
  scoped per destination by the host. There is no separate "adopt" flag in
  the planner; adoption is take-over.
- `Decision = Create | Satisfied | ReplaceOwn | TakeOver | Refuse(Drifted |
  Unowned | Unsupported)`. Order: unsupported refused; absent created (a
  removal is already done); gripsack's own write kept when it is the intent,
  replaced otherwise; consent absorbs anything else (also bytes that already
  match: a take-over opens an epoch); a foreign object equal to the intent is
  satisfied (reconvergence); otherwise refused, `Drifted` when a record
  exists, `Unowned` when none does. Take-over never authorizes a removal.
- Copies/templates: identical to the old `plan_copy` truth table
  (`Preserve` ≡ `Refuse(Drifted|Unowned)`, which the host still records as
  preserved drift). Links: the decision is now the same rule; a refusal
  stops apply (`Authority::Foreign`, new `Authority::Drifted` with the
  message "changed since gripsack deployed it"), rollback keeps and notes.

### 15.3 Proofs (`proofs/Kernel/Ownership.lean`; `Audit.lean`: 110 theorems, 26 claims, standard axioms only)

- `decide_unit_k1` (K1): `Writes d → live = Absent ∨ (∃ o, live = Obj o ∧
  recorded = Wrote o) ∨ (consent = Granted ∧ ∃ o, live = Obj o)`.
- `decide_unit_drift_never_written`: consent withheld and `recorded ≠ Wrote
  o` for the live `Obj o` ⇒ no write.
- `decide_unit_owned_drift_refused` (FND-1): `Wrote w`, live `Obj o`, `o ≠
  w`, no consent, intent not `o` ⇒ `Refuse Drifted`.
- `decide_unit_unrecorded_refused`: `Unrecorded`, live `Obj o`, no consent,
  intent not `o` ⇒ `Refuse Unowned`.
- `decide_unit_identity_only`: for every injective `f : Ident → Ident`,
  renaming recorded/live/intended by `f` leaves the decision unchanged — the
  decision sees a link's target only through identity equality.
- `decide_unit_takeover_acquires`, `decide_unit_progress` (absent ⇒ create;
  own write ⇒ satisfied or replace, whatever the consent),
  `decide_unit_unsupported_refused`, `Decision.writes_spec`.
- Every claim unfolds the shipped helpers itself (one shared tactic), so a
  mutant is rejected by the claims it falsifies, not by a helper lemma.
  Mutants (`proofs/mutants.py`, 22 total, 3 new), each rejected by exactly:
  `ownership-recorded-overwrites-live` (FND-1: `Wrote(_) => true`) by
  `k1`, `drift_never_written`, `owned_drift_refused`;
  `ownership-link-target-confers` (an unrecorded *link* counts as ours — the
  prefix rule's kernel analogue) by `k1`, `drift_never_written`,
  `unrecorded_refused`, `identity_only`; `ownership-consent-ignored` by `k1`,
  `drift_never_written`, `owned_drift_refused`, `unrecorded_refused`,
  `takeover_acquires`.

### 15.4 Wiring

- `ops::plan::DestView` carries `prev: Option<PreviousRecord { entry,
  store_path }>` and `consent: Consent` (no bools); `DestView::decide` builds
  the kernel inputs. `plan_link`, `plan_write`, `plan_migration_write` (the
  admitted predecessor link is the recorded identity; anything but
  `ReplaceOwn` is refused) and the non-merge branch of `plan_remove_op`
  (intent `Removed`, no consent) call `decide_unit`. `previous_ownership`,
  `plan_restore_op` and rollback pass the recording generation's store path.
- Kernel identities are built in one place, `gripsack_store::hash::kernel`
  (also used by the recover driver now): a file is its mode-aware identity,
  a link the SHA-256 of its target bytes.
- Merge stays outside (`Recorded::Observed` if asked): K5 territory.

### 15.5 Tests

- e2e (failing before, passing after; `e2e/test_ownership_modes.py`):
  `test_user_replacement_of_an_owned_link_is_refused_until_take_over[file|link]`,
  `test_user_link_into_the_store_is_not_ours`,
  `test_rollback_keeps_a_user_replacement_of_an_owned_link`. Before: all four
  failed — apply/rollback re-pointed the user's object.
- Two existing e2e encoded the old behaviour and were updated:
  `test_a_repaired_destination_cuts_a_generation` relied on FND-1 (a
  user-re-pointed owned link silently repaired); it now refuses, and the
  repair is `--take-over`, which still cuts a generation that rollback
  undoes. `test_preserved_copy_blocks_a_mode_switch_to_owned` keeps its
  behaviour (refuse, edit stands) and now sees the drift message.
- `crates/gripsack-kernel/tests/ownership_lineage.rs` (from `lineage_model`):
  every sequence to depth 6 over copy and link modes with user files, user
  links and user links into the store, driving `decide_unit` for deploy and
  prune; oracles: a run without take-over never changes a live object that
  is not gripsack's last write, own destinations converge, the origin
  survives the epoch and returns at its end, aliases are rejected. A
  calibration replays the legacy link rule and requires the explorer to
  find both FND-1 and the widening.
- `ops/model.rs` now uses the kernel as its oracle and adds
  `owned_links_follow_recorded_identity_only` (lifting of both defects).
- Copy/template: checked for the same bug class; none. `plan_copy` already
  required live = last managed write (or consent); the kernel decision is
  the same table, and the existing drift e2e pass unchanged.

### 15.6 Deleted

`gripsack-policy/src/ownership.rs` (`plan_copy`, `plan_link`, their Verus
contracts; `check_verus.py` floor 384 → 382, its two mutants and the
unrelated-lemma calibration retargeted to `merge.rs`),
`gripsack-exec/src/lineage_model.rs` (its truth-table tests are the Lean
claims, plan/0057 D7; its explorer moved to the kernel),
`deploy::intact_deployed`, the `deploy` re-exports of the policy functions,
and `ModeInput::Link::already` (equality is the kernel's).

### 15.7 Deviations

1. Step 6 is split: this is the ownership decision only. Merge/block
   generation (K5) and the rest of step 6 stay.
2. `ops/model.rs` and `mode_model.rs` stay: they test the host lifting
   (observation, receipt, permission policy, execution) that the kernel
   cannot see; they now use the kernel as their oracle.
3. One behaviour beyond the two defects follows from deciding by recorded
   identity: when a declaration switches from link to copy without a
   directory migration, gripsack's own intact link is replaced by the file
   (`ReplaceOwn`) instead of being kept as drift. K1 permits it (the live
   object is gripsack's last write).
4. A re-pointed own link now reports `Authority::Update` ("update") instead
   of `Fresh` ("new").

## 16. F2 step 8 record — rollback fidelity (K2 for rollback, FND-2)

- Branch `foundation/f2-rollback`, stacked on `foundation/f2-ownership`
  (PR #222, K1 `decide_unit`); date 2026-11-09. Taken before steps 3–5 and
  7 because the design review's finding 2 is a user-facing guarantee.
- Question: does rollback restore exactly what the target generation
  deployed, and can that decision be the proved kernel's?

### 16.1 The review's claims, checked (code at `0943a35`)

Each claim has an e2e reproducer in `e2e/test_rollback_fidelity.py`, run
against the baseline binary before any change (four failed, four passed).

- **R1 "templates re-render from current source": refuted as stated; the
  residue is real.** Rollback reads the template from the *target's* store
  path (`ops/plan/restore.rs:33,59`), which is input-addressed and checked
  against its recorded `tree256` before any write (`rollback.rs:372-397`).
  Workspace templates (every generation the current frontend produces)
  render into the store at preparation (`workspace/content.rs:281-312`,
  `output` plus `result_digest`) and deploy as `tracked_copy`; no current
  path deploys `Ownership::Template` (`workspace/preview.rs:349` refuses
  it). So for current generations rollback already copied the retained
  rendered bytes — `test_rollback_restores_the_rendered_bytes_after_the_template_source_changed`
  passed before the change. The residue: older `template` receipts (0.16.1
  "vars-in-manifest", 0001 §3.7) are re-rendered with recorded variables
  (`restore.rs:67-76`) and the result was never compared with the recorded
  identity. Reproducer `test_rollback_refuses_an_older_template_receipt_that_no_longer_renders_its_record`:
  before, rollback exited 0, selected generation 1 and wrote
  `email = "someone-else"` where the record named the `a@b.c` bytes.
- **R2 "modes are guessed, nothing is checked against the record":
  confirmed for older receipts.** A receipt without `file_mode` was
  restored at the payload's exec-bit default (`restore.rs:77-90`) and
  `plan_restore_op` compared nothing with `entry.hash`. Current receipts
  always record the landed mode (`ops/plan/write.rs`), and
  `test_rollback_restores_the_recorded_mode` passed before. Reproducer
  `test_rollback_refuses_a_receipt_its_retained_bytes_do_not_reproduce`
  (record says 0600, no mode recorded): before, rollback wrote 0644, exited
  0 and flipped; the live identity differed from the record.
- **R3 "no safe restore plan is a warning, then the flip": confirmed.**
  `rollback.rs:307-314,343-350` pushed a warning and continued to the flip
  (`rollback.rs:80`); reachable when a managed block's host file is no
  longer UTF-8 text or no longer a file (`restore.rs:105-110`). Reproducer
  `test_rollback_refuses_when_a_managed_block_cannot_be_restored`: before,
  exit 0 with "skipped … no safe restore plan" and generation 1 selected.
- **FND-2: confirmed.** `env.rs:27-49` (from `rollback.rs:72`) wrote
  `generations/<n>/env/profile.sh` into the target and removed
  `$GRIPSACK_HOME/env/profile.sh` outside the journal. Reproducer
  `test_rollback_never_writes_into_the_target_generation`: before, the
  profile was written into generation 1 and the legacy file deleted.
- K6 already held apart from FND-2: rollback reserves a fresh transaction
  (`journal/marker.rs:84`, `generations/selection.rs:52`) and publishes no
  generation; `test_rollback_reselects_with_a_fresh_transaction` passes
  before and after.

### 16.2 Root cause: are the deployed bytes retained?

Yes, for every generation the current product writes: tracked copies and
workspace templates deploy from a store file whose bytes are what landed,
the manifest records the landed mode and the mode-aware identity, and the
store tree is verified before rollback. Only pre-workspace `template`
receipts keep variables instead of bytes. No store format change is
needed: rollback now accepts a restore source only if its identity IS the
record (`ops::retained`, S7: equal digests mean equal bytes). An older
receipt whose re-render or chosen mode does not reproduce the record is
refused, not approximated. Storing rendered bytes for those receipts is
impossible after the fact and unnecessary for new ones.

### 16.3 The machine (`crates/gripsack-kernel/src/rollback.rs`)

`rollback_step(RollbackState, Event) -> (RollbackState, Command)`, one
command outstanding, one unit at a time (host order: deepest destination
first).

- **Facts:** `PlanFacts { units, reselect: SelectionIdentity, previous,
  profile: Retained|Missing }`; per unit `UnitFacts { dest, place:
  Outside|Generation, route: Leaf|Delegated, current: Recorded, target:
  Released(Intended)|Wrote{ident, bytes: Retained|Missing}|Preserved }`.
  `Released` is where the target deploys nothing: gripsack's write is
  removed or replaced by the take-over origin the epoch recorded (apply's
  prune intent).
- **Decision** `decide_restore`: the K1 decision `decide_unit` without
  consent, asked about the target's identity (or a removal, for a release);
  `Satisfied` → `Leave(Already)`, `Refuse(r)` → `Leave(Kept(r))`, a
  preserved target → `Leave(Untouched)`; a write → `Write(install)` only if
  the bytes are retained and the place is outside generation directories,
  else `Refuse(Unrestorable|Protected)`.
- **Commands:** `ReadPlan`, `ReadUnit(k)`, `Observe`, `Mutate{unit, dest,
  expect, install}` (driver re-checks `expect`), `Delegate`,
  `Report{unit, dest, settled}`, `Flip(selection)`, `Finish`, `Abort`.
  `Landed{live}` must equal `install`, else `NotRestored`. `admit` refuses a
  missing profile (FND-2) and a reselection whose transaction equals the
  current one.

### 16.4 Proofs (`proofs/Kernel/Rollback.lean`; `Audit.lean`: 157 theorems, 37 claims, standard axioms only)

- `decide_restore_spec`: a `Write i` satisfies `Installs target i` (the
  recorded identity from retained bytes, or the release), `place =
  Outside`, and live is absent or gripsack's current write; the decision
  never claims `Restored`.
- `decide_restore_drift_kept` (K1), `decide_restore_unrestorable_refused`.
- Over every run (`Run`, invariant `Inv`, `step_spec`):
  `rollback_installs_target` (K2: a write answers the observation of the
  unit under decision and installs that unit's target identity),
  `rollback_write_k1`, `rollback_never_writes_generation`,
  `rollback_restored_landed` (restored only when the landed object is the
  install), `rollback_flip_after_every_unit` (the flip names the planned
  selection and every unit `k < units` was reported),
  `rollback_flip_admitted` (fresh transaction, retained profile).
- R3 by name: `rollback_unrestorable_aborts`,
  `rollback_delegated_refusal_aborts`.
- Mutants (`proofs/mutants.py`, 31 total, 9 new), each rejected by exactly
  the named theorems: `rollback-unretained-installed`
  (`decide_restore_spec`, `decide_restore_unrestorable_refused`),
  `rollback-installs-not-target` (`decide_restore_spec`),
  `rollback-drift-overwritten` (`decide_restore_spec`,
  `decide_restore_drift_kept`), `rollback-writes-generation`
  (`decide_restore_spec`), `rollback-restored-unchecked`
  (`on_landed_spec`), `rollback-flip-skips-unit` (`on_plan_spec`),
  `rollback-unrestorable-settled` (`on_delegated_spec`,
  `rollback_delegated_refusal_aborts`), `rollback-reused-transaction`
  (`fresh_spec`), `rollback-profile-ignored` (`admit_spec`).
  `mutants.py` attribution fix: `end <section>` no longer resets the
  namespace (theorems after `end helpers` were reported unqualified).

### 16.5 Wiring

- `gripsack-exec/src/rollback/restore.rs` is the driver: it decodes both
  manifests into `UnitFacts` (current record via `PreviousRecord::recorded`,
  target via `ops::retained`, the release via the entry's prior blob, place
  by the canonical destination under `$GRIPSACK_HOME/generations`), executes
  `Mutate` through `plan_restore_op`/`plan_remove_op` and `execute_op` (the
  journal re-check pins the object; a planner that no longer agrees answers
  `Failed(Drift)`), observes `Landed`, writes the user notes for `Kept`,
  and returns at `Flip`. `rollback_generation` checks the reselection is
  the run's reservation, prepares activation, flips, and answers the
  machine. Any refusal returns an error; the existing compensation
  (journal reconcile) undoes this run's writes.
- `ops::retained` / `plan_restore_op`: the restore source is accepted only
  when it reproduces the record (owned link: payload hash; copy/template:
  `matches_file`, which also admits pre-0043 bytes-only template hashes
  with their recorded mode).
- Delegated units (managed blocks, directory-transition destinations) keep
  the legacy per-destination code, but "no safe restore plan" is now
  `Refused`, which aborts.
- `env::render_env_file` is deleted; `env::retains_profile` is the read-only
  check behind `PlanFacts.profile`.

### 16.6 Tests

- e2e `test_rollback_fidelity.py` (8): four fail before / pass after (§16.1
  R1 residue, R2, R3, FND-2); four pin what held (rendered template bytes,
  recorded mode, a consistent older template receipt, fresh transaction).
  `test_generation_history.py::test_rollback_never_rewrites_historical_generations`
  is unchanged and passes.
- Kernel explorer `crates/gripsack-kernel/tests/rollback.rs`: every plan of
  up to two units over a small identity alphabet (≈33k runs) with a
  simulated host: writes only over absence or gripsack's write, never in a
  generation directory, exactly the target identity; a flip implies every
  needed write happened from retained bytes; every refusal has a cause; a
  host that lands the wrong object never reaches the flip.

Gates (docker, this branch): `test` passed (fmt, clippy `-D warnings`,
cargo test); `proofs` passed in 259 s (no translation drift, 157 theorems,
37 claims, standard axioms only, 31/31 mutants calibrated); `e2e` 679
passed in 11m50s.

### 16.7 Deviations and what is deferred

1. **No MemFs crash exploration of rollback.** The driver executes through
   the real journal and `execute_op`, not `gripsack-host::Storage`; the
   machine fits the driver pattern, but its effects do not yet. It becomes
   possible when step 3 moves the leaf effect protocol (capture prior,
   publish record, mutate under re-check) onto `Storage`; until then the
   restore→flip window is covered by the existing crash e2e
   (`after-rollback-restore`) and recovery by the proved recover machine.
   **Mostly closed by §18:** every rollback write now goes through the
   kernel's leaf protocol over `Storage` (`RealFs` in the product), and the
   kernel rollback machine composed with that protocol is crash-explored over
   MemFs (§18.5). What remains is in §18.7 item 3.
2. **Managed blocks (K5) are delegated.** Rollback splices the retained
   block payload (store bytes verified by `tree256`) but does not compare it
   with the block's recorded hash; that lands with the K5 splice in the
   kernel (rest of step 6). An unrestorable block now refuses instead of
   flipping. **Closed by §17:** the kernel's block decision restores (or
   finds in place) a block only when the retained payload's digest is the
   target's record (`decide_block_restore_exact`), and refuses otherwise.
   The unit stays on the delegated route of the rollback machine.
3. **Directory-transition destinations are delegated** to the directory
   domain (step 9).
4. **The legacy `$GRIPSACK_HOME/env/profile.sh` is no longer removed.** It
   is inert since 0.22 (the rc line sources `current/env/profile.sh`); a
   one-shot cleanup belongs in `grip doctor` or the migration path the
   review proposes for legacy readers, not in rollback.
5. **Behaviour change:** where the current generation recorded preserved
   drift and the destination is now absent, rollback creates the target's
   recorded object (K1 allows writing into absence); the old code skipped
   the unit.
6. Not addressed here (review §2.1): `rollback` with no argument still
   targets current−1 by number rather than the last selected generation.

## 17. F2 step 6 part 2 record — the managed-block splice (K5, K1 for blocks)

- Branch `foundation/f2-splice` from `main` at `524f576`; date 2026-11-09.
- Question: can the managed block — marker grammar, scan, content
  normalization, rendering, splice and the "may gripsack touch this block?"
  decision — be the proved kernel's, with the bytes users keep outside the
  markers preserved exactly, and re-applying a no-op?

### 17.1 Defects, checked on the baseline

Both reproducers live in `e2e/test_managed_block_splice.py` and were run
against the baseline binary (`grip 0.52.0`, the `f2rollback-e2e` image built
from the rollback branch merged as `524f576`) before any change: two failed,
one passed.

- **FND-4 (new): a block never matched its own sha.** Content normalization
  was "trim trailing `\r`/`\n`, then `\r\n` → `\n`", and rendering split the
  normal form with `str::lines`, which drops one `\r` before each `\n`. For a
  payload with a carriage-return run before a line break the rendered block's
  content normalizes to different bytes than the marker's sha was taken
  over. Reproducer `test_a_payload_with_carriage_returns_before_a_newline_settles`
  (payload `export A=1\r\r\r\nexport B=2\n`): before, the first apply
  wrote `export A=1\r\n` inside an LF block, so the second `apply` saw a CRLF
  file, rewrote the whole block with CRLF line ends, printed `merged shell →
  ~/.bashrc (hand-edited block regenerated)` and activated generation 2; the
  third apply changed no byte and still activated generation 3. The explorer
  catches the same class (`the_legacy_normalization_is_caught_by_the_round_trip_oracle`).
- **§16.7 deviation 2 (known): rollback restored a block without comparing it
  with its record.** Reproducer
  `test_rollback_refuses_a_block_payload_that_does_not_reproduce_its_record`
  (generation 1's block record rewritten to another digest, standing in for
  an older receipt in another hash domain): before, `grip rollback` exited 0
  and selected generation 1 with a block that is not its record.
- What held: `test_rollback_restores_a_managed_block_and_keeps_foreign_bytes`
  (CRLF foreign bytes before and after the block survive a rollback).
- Checked for and not found: the marker grammar admits a comment prefix with
  whitespace (`DeploymentEntry.marker`), which would never scan back and
  append a duplicate on every apply — but no current producer sets a custom
  marker (`marker: None` everywhere; only retained deployments could carry
  one). The kernel now refuses such a render instead (`Unrecognized`).

### 17.2 The kernel (`crates/gripsack-kernel/src/block.rs`)

- **Grammar and scan**, moved from `gripsack-exec/src/managed_blocks/parse.rs`
  and `gripsack-policy/src/merge/scanner.rs` and restated over bytes:
  `classify` (a marker is its key preceded by at most one word; Unicode
  `White_Space` exactly as `char::is_whitespace`, so behaviour on foreign
  lines is unchanged), `scan(text, owner) -> Result<Vec<Block>, ScanFailure>`
  (blocks of other owners must nest correctly but are not returned; the
  pre-hash header line is not content).
- **Content and rendering**: `normalize` drops every line's trailing
  carriage returns, then trims trailing `\r`/`\n` — identical to the old rule
  wherever that rule was stable, and a fixed point of render-then-normalize
  (FND-4). `render` writes the open marker (owner, the 16 recorded sha digits,
  mode), the header, one line per content line and the close marker, every
  line ending in the hosting text's newline convention. `mark(digest)` is the
  16 lowercase hex digits; `marks(sha, digest)` compares them
  case-insensitively, as markers always were. The host still computes the
  SHA-256 (`managed_blocks::content_hash` = SHA-256 of `normalize`).
- **Splice**: `ordered` (in order, disjoint, non-empty, in bounds),
  `splice(text, blocks, replacement)`, `append(text, newline, block)`.
  `upsert(text, owner, style, digest, mode, payload)` scans, checks
  `ordered`, renders, splices (or appends), and returns the output only if
  `settled` accepts it: the output's only block of `owner` is exactly the
  rendered block, with the same newline convention, recording `mark(digest)`
  and `mode`, and its content normalizes to `normalize(payload)`. Otherwise
  it refuses (`Unrecognized`), never writes. `remove(text, owner)`.
- **Decision** `decide_block(record, host, blocks, intent)` with
  `Host = Absent | File{mode} | Foreign`, `Seen{content, marked, mode}`,
  `BlockRecord{hash, mode, preserved}` and
  `BlockIntent = Deploy{desired, mode} | Restore{retained, recorded, mode} | Remove`;
  `BlockDecision = Write | Satisfied | Preserve | Remove | Keep | Absent | Refuse(Foreign | Unrestorable)`.
  The table is the one `plan_merge`, `plan_remove_op`, `remove_or_restore_prior`
  and `ManagedBlockSet::{mode_conflicts, satisfied, intact}` implemented,
  plus the restore check.

Product semantics, verified, not invented (CHANGELOG 0.19.0 "merge blocks
carry their content hash", 0.21.1, `e2e/test_ownership_modes.py::test_merge_block_carries_its_content_hash`,
`test_merge_boundaries.py`): inside its markers a block is gripsack's, so a
deploy regenerates a hand-edited block and says so; a removal (prune,
rollback's release) requires the block intact and otherwise keeps it; any
conflicting mode evidence preserves the whole file; malformed markers refuse.

### 17.3 Proofs (`proofs/Kernel/Block.lean`; `Audit.lean`: 216 theorems, 46 claims, standard axioms only)

The K5 claims are partial-correctness statements over the shipped functions
("if the call returns `out`, then …"); the decision claims likewise.

- `splice_ok`: the splice returns exactly `spliceSpec` — the bytes before the
  first span, the replacement, then every byte between and after the spans
  in order (`spliceFrom`, a plain recursion over the spans).
- `upsert_preserves_outside` (K5): an upsert's output is the splice of the
  rendered block into the scanned blocks, which are `Ordered` (in order,
  disjoint, non-empty, in bounds); with no block, the text, one separating
  newline if it lacked one (`separator`), then the block.
- `remove_preserves_outside` (K5): a removal keeps exactly the bytes outside
  the scanned blocks.
- `upsert_idempotent` (K5): upserting an upsert's output again with the same
  inputs returns it unchanged.
- `upsert_round_trip`: the output's only block of the owner is the rendered
  block, recording the digest's sha digits and the mode, with content that
  normalizes to the payload's normalization — the next apply sees it intact
  (FND-4 cannot recur without the kernel refusing instead).
- `decide_block_remove_intact` (K1 for blocks): only a removal removes, and
  only an intact block — a file host, a non-drift record, exactly one block,
  marker sha naming its content, content equal to the record, no mode drift.
- `decide_block_foreign_untouched`, `decide_block_mode_evidence_preserves`
  (conflicting mode evidence never writes, for every intent),
  `decide_block_restore_exact` (rollback writes or claims a block only from a
  retained payload whose digest is the record).
- Helper specs the mutants are attributed to: `splice_loop_ok`,
  `splice_single`, `append_ok`, `ordered_ok`, `settled_ok`, `upsert_ok`,
  `intact_true`, `mode_conflict_false`, `deploy_facts`.
- Mutants (`proofs/mutants.py`, 41 total, 10 new), each rejected by exactly
  the named theorems: `block-splice-drops-tail` (`splice_loop_ok`,
  `splice_ok`, `splice_single`), `block-splice-every-span`
  (`splice_loop_ok`), `block-append-no-separator` (`append_ok`),
  `block-overlapping-spans` (`ordered_ok`), `block-upsert-unchecked` (the
  output check skipped: `upsert_ok`), `block-settled-ignores-bytes`
  (`settled_ok`), `block-edited-intact` (`intact_true`),
  `block-mode-evidence-ignored` (`mode_conflict_false`),
  `block-foreign-written` (`deploy_facts`), `block-restore-mismatch-written`
  (`decide_block_restore_exact`).

What is **not** proved, and is tested instead: that `upsert` never refuses a
well-formed hosting text (liveness), and the marker grammar itself (which
lines are markers). Idempotence and the round trip are proved for every
output the kernel returns because `upsert` checks its own output before
returning it and the proof composes that check with the splice's
specification (translation validation); a failing check is a loud refusal,
never a wrong write. The explorer (§17.5) covers liveness.

### 17.4 Wiring

- `gripsack-exec/src/managed_blocks/mod.rs` is a thin adapter: kernel scan
  offsets to `&str` views, the SHA-256 of the kernel's normal form, the
  comment style of a destination, `record(entry)` and `host(observation)`
  for the decision, `validate_payload` over the kernel's `classify`.
- `plan_merge` asks `decide_block` with `BlockIntent::Deploy` (apply, plan)
  or `BlockIntent::Restore` (rollback, `ModeInput::Merge.source =
  BlockSource::Retained { recorded }`); `plan_remove_op` and
  `remove_or_restore_prior` ask with `BlockIntent::Remove`.
- Rollback's block units stay on the machine's delegated route; a payload
  that is not the record is now refused by the kernel and the rollback stops
  before the flip ("… is not the recorded block; cannot restore it exactly").

### 17.5 Tests

- e2e `test_managed_block_splice.py` (3): two failed before / pass after
  (§17.1), one pins what held. `test_merge_boundaries.py`,
  `test_ownership_modes.py` merge cases and `test_rollback_fidelity.py`'s
  block case pass unchanged.
- Kernel explorer `crates/gripsack-kernel/tests/block.rs`: every hosting text
  of up to four lines over ten line kinds (foreign, empty, both owners'
  markers, header, edited content, two-word prefixes, malformed metadata),
  LF/CRLF, with and without a final newline, × 2 owner spellings × 4 comment
  styles × 8 payloads: a malformed text is refused, a well-formed one never
  is (liveness), and the output matches an independent splice oracle, removes
  back to the foreign bytes, round-trips and is idempotent. Calibration: the
  legacy normalization is caught by the round-trip oracle. Plus the marker
  format golden test, the grammar cases ported from
  `managed_blocks/tests.rs`, and the decision table against the planner it
  replaced.

Gates (docker, project `f2splice`, this branch): `proofs` passed in 799 s
(no translation drift, 216 theorems, 46 claims, standard axioms only,
41/41 mutants calibrated); `test` passed (fmt, clippy `-D warnings`, cargo
test); `e2e` 682 passed in 18m50s. Not run: `verify` (the Verus gate whose
floor and calibration this branch edits), `model` (whose list lost
`MergeBoundary`), `ts-test`, fuzz.

### 17.6 Deleted

`gripsack-policy/src/merge.rs` and `merge/scanner.rs` (the Verus splice and
scanner: 11 obligations, floor 382 → 371, their two Verus mutants; the
unrelated-lemma calibration moved to `graph.rs`), `specs/MergeBoundary.tla`
and its four cfgs (the decision model; its invariants are the decision
theorems), the `merge` fuzz target (target, harness, corpus, seeds — trim
register), `gripsack-exec/src/managed_blocks/parse.rs` (the grammar, now the
kernel's) and `managed_blocks/tests.rs` (ported to the kernel test),
`ManagedBlockSet::{intact, satisfied, mode_conflicts}` and
`normalized_content`. `verification/guarantees.md` is frozen and keeps its
historical MERGE-SPLICE-001 / MERGE-SCAN-001 rows.

### 17.7 Code contortions (recorded in the source)

1. A repeated-byte array constant (`*b">>>"`) is a repeat expression Aeneas
   rejects ("unexpected constant `[62u8; 3]`"): `>>>`/`<<<` are written and
   checked as one byte thrice (`push_triple`, `is_triple`).
2. A `match` on an enum read from a slice element inside a loop ("Not allowed
   to expand enumerations with several variants") moved into a helper
   (`other_mode`).
3. Locals named `block` inside module `block` (§13.4 item 5 again):
   `span`, `rendered`, `only`.
4. The scanner's open block is rebuilt per line instead of updated through
   `&mut`; owner ranges are compared by an explicit byte loop.
5. clippy idioms that go through trait methods or closures
   (`RangeInclusive::contains`, `is_empty`, `Option::map`, `is_some`) are
   allowed in `block.rs`; the proofs are stated over this spelling.
6. Lean side: `Result` is an interaction tree, so constructor facts come from
   `Result.match` (`vis_not_ok`, `div_not_ok`); a tuple pattern
   `let (a, b) ← …` elaborates to `Std.uncurry`; partial correctness of a
   loop is one generic rule (`loop_ok`) over the body.

### 17.8 Deviations and what is deferred

1. **K1 for blocks is stated per the product contract, not §5's wording.**
   §5 says "a managed block only if intact". The product regenerates a
   hand-edited block on deploy (0.19.0); what is proved is: removal only if
   intact, deploy never touches a byte outside the owner's blocks (K5) and
   never writes over conflicting mode evidence, a non-file host is never
   written, and restore only from the record.
2. **Liveness and the grammar are tested, not proved** (§17.3).
3. **Rollback's block units stay delegated** in the rollback machine; the
   kernel decision and splice now decide and perform them, but the machine's
   K2 theorems do not range over them. Folding them in needs a block
   identity in the machine's `Ident` vocabulary.
4. A custom comment marker that the grammar cannot read back (whitespace or
   a marker key in it) is refused instead of appended forever; no current
   producer writes one.
5. Host-computed SHA-256 stays outside the kernel (S7).
6. A generation recorded by 0.19–0.52 for a payload hit by FND-4 keeps a
   digest in the old normal form. `grip verify` still accepts it (one more
   historical block domain in `verify_store`); rolling back to it refuses,
   because no retained payload reproduces that record under the kernel's
   normal form. Every other recorded block digest is unchanged.

## 18. F2 step 3 record — the leaf effect protocol and the journal run (P1, P3)

- Branch `foundation/f2-journal` from `main` at `0625d3e`; date 2026-11-10.
- Question: can the journal's run — its marker, the leaf effect protocol for
  each destination (record prior evidence and intent durably, then mutate,
  then `Landed`) and its completion — be a kernel machine the product drives
  over `Storage`, with P1 and the cleanup half of P3 proved in Lean over the
  shipped Rust?

### 18.1 The machine (`crates/gripsack-kernel/src/journal_run.rs`)

`run_step(RunState, Event) -> (RunState, Command)`, one command outstanding,
living as long as the run (`begin_run` → leaves → flip → `commit_run`, or
`end_run`).

- **Marker:** `Start` → `Publish{Marker, a}` for `a` = stage, sync the file,
  publish the name, sync the journal directory (the F1 `PublicationAction`
  order, its `successor`), each answered `Done`; then `Ready`.
- **Leaf, per destination** (`Unit(UnitFacts{dest, expect, intended,
  follows})` answers `Ready`): `Observe`; refuse `Unsupported` (no journal
  identity), `Drifted` (not the plan's `expect`) or `Interleaved` (a repeated
  write whose destination is not this run's earlier intent); else
  `CapturePrior{expect: observed}` (the driver re-checks, makes the prior
  durable — a file's bytes and mode in the prior blob store — and returns the
  entry); then `Publish{Entry, a}` ×4; only then `Mutate{expect: observed,
  intended}` (the driver re-checks after staging, immediately before the
  rename or unlink); `Landed{live}` is accepted only if it is the intended
  object (`rollback::lands`), else `NotLanded`.
- **Completion:** `Commit` → `Flip` (the caller flips; answering `Done` means
  durable) → `Settle(Committed)`; `End` is refused (`Unfinished`) once any
  unit landed, else `Settle(Ended)`. `Settled{records}` → `Unlink(Record k)`
  for every record, `Barrier`, `Unlink(Marker)`, `Barrier`, `Finish`.
- Directory records stay with the legacy domain: the host seals or checks
  them when settling and reports how many records the cleanup removes.

### 18.2 Proofs (`proofs/Kernel/JournalRun.lean`; `Audit.lean`: 268 theorems, 50 claims, standard axioms only)

An execution is its log of (event, command) pairs (`Run`); `Inv` is kept by
every step (`step_spec`, `run_inv`).

- **P1 `mutate_after_durable_intent`:** in every execution, a `Mutate` is
  issued only in answer to `Done`, after a log that *starts* with the marker's
  complete publication (`pubSteps Marker Start SyncNamespace` then `(Done,
  Ready)`) and *ends* with exactly this unit's leaf protocol: `(Unit f,
  Observe u f.dest)`, `(Observed f.dest x, CapturePrior u f.dest x)`, then
  the entry's stage, sync, publish and sync, each answered `Done`; with
  `f.expect = x` (the plan's observation is what is mutated over) and
  `f.intended` installed.
- **P3 `record_unlink_needs_no_evidence`:** a journal record is unlinked only
  after `(Commit, Flip)` was answered `Done` (so any later crash classifies
  the run committed and needs no entry) or in a run that never published an
  entry.
- **P3 `marker_unlink_after_record_barrier`:** the marker unlink only right
  after a journal barrier that follows the unlink of every record (the
  0.19.x resurrection order, now for the run as for recovery).
- `unit_accepted_only_landed`: the machine is ready again after a mutation
  only when the landed object is the unit's intended one.
- Helper specs the mutants are attributed to: `on_observed_spec`,
  `on_landed_spec`, `on_end_spec`, `next_unlink_spec`.
- Mutants (`proofs/mutants.py`, 48 total: 8 new, 1 removed with
  `admit_mutation`), each rejected by exactly the named theorem:
  `run-mutates-before-entry-barrier` (mutation after the entry's rename,
  before its directory barrier: `step_spec`),
  `run-ready-before-marker-barrier` (units accepted before the marker's
  barrier: `step_spec`), `run-skips-prior-capture` (`on_observed_spec`),
  `run-ignores-plan-drift` (`on_observed_spec`), `run-landed-unchecked`
  (`on_landed_spec`), `run-cleans-before-flip` (`step_spec`),
  `run-ends-after-mutation` (`on_end_spec`), `run-cleanup-barrier-swapped`
  — the 0.19.2 order — (`next_unlink_spec`).

### 18.3 Wiring

- `gripsack-host::journal_run::RunDriver` executes the machine over any
  `Storage` behind a `RunHost` (marker and entry wire, prior capture, identity
  hashing, settling). Installs are `File{bytes, mode}`, `Link{target}`,
  `Remove` and `Splice` (a managed block's upsert or removal, computed from
  the re-checked latest bytes).
- `gripsack-store` (`journal/mutation.rs`) is the product `RunHost` over
  `RealFs`: `begin_run` publishes the marker through the machine and keeps it
  in `JournalRun`; `journal::mutate` replaces `capture` → `record` →
  `MutationPermit::execute`; `flip` asks the machine for the flip and reports
  a failed one; `commit_run` and `end_run` are the machine's cleanup. Error
  texts users saw are kept ("changed between the drift decision and the
  mutation", "did not reach its intended state", "between journaled
  mutations").
- `gripsack-exec`: every `execute_op` arm (link, write, merge upsert, removal
  and prior restore) passes an `Install`; `deploy::removal` replaces
  `remove_or_restore_prior`/`remove_entry_deployed`/`restore_prior` — the
  exact-target re-check is the leaf protocol's, the K1 decision the
  planner's. Rollback writes through `execute_op`, so through the same
  protocol.
- The directory domain's reactivation keeps a prior capture
  (`mutation::capture`, same observation and prior evidence) — its record
  carries the prior; it is not a leaf mutation (step 9).
- `GRIPSACK_ENGINE=legacy` still selects the legacy *recovery* in debug
  builds; there is no legacy leaf path (nothing to switch).

### 18.4 Defects and the S5 window, measured

No new correctness defect was found. One measured change, not claimed as a
reproduced product bug: the 0.53 leaf checked the destination against the
plan once, at capture, then published the entry and wrote with no re-check.
`the_leaf_never_overwrites_an_external_write_its_recheck_sees` lands an
external write before each storage operation of a one-destination run: the
0.53 order (restated in the test, as the 0.19.x cleanups are) overwrites it
at 12 operations (the entry's whole publication and the staging); the kernel
leaf at 2 (between the act's re-check and the rename), and otherwise stops
with a drift refusal and leaves the user's bytes. This was measured on MemFs
over a restatement of the old order, not on the old binary.

### 18.5 Tests

- `crates/gripsack-host/tests/journal_cleanup.rs` now drives the *run* through
  `RunDriver` (the hand-written apply is gone) and the recovery through the
  recover machine, every kill and power-loss cut of each, over MemFs: every
  entry shape on fresh and existing machines, repeated writes, two
  destinations, edits after each crash, ambiguity, the numeric-commit mutant,
  the 0.19.1/0.19.2 calibrations, the wedge, recovery's S5 window. New: the
  leaf's S5 window (§18.4); an ended run recovers to nothing after any crash;
  a run that journaled a destination cannot end; and
  `rollback_survives_every_crash_and_every_recovery_crash` — the kernel
  rollback machine, its writes through the leaf protocol, the flip and the
  cleanup, crashed everywhere (45 cuts / 186 images), each image's recovery
  crashed everywhere (1922 cuts / 7553 images), then recovered: every
  destination holds exactly the generation rolled back from or to.
- Store and exec unit tests moved from `capture`/`record` to `mutate` (a
  crash between entry and act is a failing `Splice`, `crash_after_entry`);
  `unfinished_mutation_cannot_end_the_run` now exercises the kernel's
  `Unfinished` refusal after a landed unit.
- The `Batched` record order (all entries, then all mutations) is gone from
  the explorer: the kernel cannot produce it and no shipped version did.
- No separate kernel explorer: P1/P3 are proved, and the MemFs suites drive
  the machine with a real host.

### 18.6 Deleted

`gripsack-policy/src/journal_protocol.rs` (Verus `CleanupProgress`: the
`journal-cleanup` family, four Verus mutants, floor 371 → 368, an estimate
until the next green `verify`), the kernel's `MutationAuthority` /
`admit_mutation` and `admit_mutation_spec`, `marker::cleanup` and its
`CleanupProgress` bookkeeping (legacy reconcile keeps a plain
`cleanup_legacy`), `mutation::{CapturedPrior, MutationPermit, record}`,
`deploy::{remove_or_restore_prior, remove_entry_deployed, restore_prior}`.

### 18.7 Deviations and what is deferred

1. **TLA+ `JournalLifecycle` stays.** Its TLC calibrations
   (`MarkerCoversEntries`, `MutationHasDurableMarker`, `ExactCommitIdentity`)
   are now Lean theorems at the machine level, but the module is the base of
   `PublicationSelection`, `RepeatedRecovery` and the TLAPS journal stack
   (P2-level), which F3 retires. Deleting its cfgs while the model stays
   would only uncalibrate it.
2. **P1 is about the machine's commands**; that a `Done` means durable is the
   driver's contract over S1–S3 (ring 1, `Storage.lean`, F3), tested by the
   MemFs suites, not proved.
3. **Rollback's exec driver is not generic over `Storage`.** Its writes go
   through the leaf protocol over `RealFs`, and the kernel rollback machine
   composed with the leaf protocol is crash-explored over MemFs, but the
   exploration uses a test host, not `gripsack-exec/src/rollback/restore.rs`
   (manifest decoding, `plan_restore_op` and the store stay on the real
   filesystem). Delegated units (managed blocks, directory transitions) are
   outside that exploration.
4. **Unjournaled effects stay listed, not proved:** prior blobs (deduplicated
   by content, synced before the entry), parent directories created by
   `dest_capability`, the selection reservation.
5. `RecordPublication`/`DurableRecord` remain for directory records
   (`Journal::write`); `RecordRole::RunMarker` is no longer used by the product.
6. **e2e instrumentation:** `test_recovery_durability.py`'s
   `before-second-restore` cut located the trace row `Before Symlink ".b"`
   (gripsack-fs labelled the temporary link by its final name); `RealFs`
   labels the boundary with the staging name it really creates, so the test
   now finds the row by that name. Behaviour is unchanged; the test failed
   on the first e2e run of this branch for that reason alone.

Gates (docker, project `f2journal`, this branch): `proofs` passed in 820 s
(no translation drift, 268 theorems, 50 claims, standard axioms only, 48/48
mutants calibrated — the F1 file-barrier mutant now also breaks
`JournalRun.step_spec`, as it should); `e2e` 682 passed in 15m39s. `test`:
the compose run failed three times in the shared `gripsack-target-dev`
BuildKit cache (an rlib missing behind a fresh fingerprint, other agents'
builds sharing the cache), after all 37 test binaries had passed; the gate's
exact commands (`cargo fmt --check`, `cargo clippy --locked --workspace
--all-targets -D warnings`, `cargo test --locked`, then the conda-helper
fmt/clippy/test) passed from the same `builder` stage with a private, clean
target directory. Not run: `verify` (whose floor this branch lowers), `model`,
`tlaps`, `ts-test`, fuzz.
