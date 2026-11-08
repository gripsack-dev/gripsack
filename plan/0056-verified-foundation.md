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
The interface therefore makes the check explicit:

```text
events   Observed{dest, Identity|Absent}  CurrentRead  MarkerRead  Inventory
         EffectLanded{cmd, Identity}       EffectFailed{cmd, Errno}   Crash
commands Capture{dest, expect}  Mutate{dest, expect, intended}  Publish{gen}
         Flip{from, to}  Barrier{scope}  Cleanup{entries}  Remove{object}
```

Each mutating command carries `expect`. The driver re-reads the target, refuses
on mismatch (`EffectFailed{Drift}`), acts, then reports the landed identity.
The kernel decides only from events, so every proof states what it assumed
about the world.

**Storage model** (ring 1's only axioms, one Lean file, mirrored in
`gripsack-host` docs and fault-injection tests):

- S1 `rename` within a directory is atomic and replaces the target.
- S2 A file's data is durable after `fsync(file)`. A name is durable after
  `fsync(parent dir)`.
- S3 A crash discards an arbitrary subset of non-durable operations,
  consistent with S2 (prefix-closed per file/directory).
- S4 One lifecycle writer at a time: apply, rollback, gc and recover hold
  `LifecycleSession`. Concurrent `grip` invocations serialize; the scheduler's
  workers never run lifecycle commands concurrently.
- S5 External writers exist, but are only *detected*, never prevented. Between
  the driver's re-read and its act there is a window. "Never overwrite unowned
  data" is proved for every external write the re-read sees. The window itself
  is a named, documented residual risk, not a hidden premise.
- S6 Case-insensitive aliasing is handled by kernel destination keys (0023,
  0030), not by the storage model.

The existing `gripsack-fs` persistence trace oracle becomes the test-side
mirror of S1–S3 (the crash-injecting in-memory storage in ring 2).

## 5. Theorems (the end-state claim list)

Ring 0 (all inputs):

- K1 Ownership: a mutation is planned only for Fresh, consented TakeOver,
  or an Update whose live identity equals the last managed write. Preserved
  drift is never overwritten.
- K2 Plan totality and fidelity: the plan is a function of
  (desired, observed, records); executing it to completion yields desired
  for every planned destination and leaves unplanned ones untouched.
- K3 Commit classification: Committed iff current = target, plus the existing
  biconditionals (CLASSIFY-001); numeric order never grants commit authority.
- K4 GC retention: current, every retained generation, priors needed for
  recovery and transitive build inputs are never collected.
- K5 Managed-block splice: foreign bytes outside markers are preserved
  exactly, and splice is idempotent.
- K6 Generations: every mutating transaction produces exactly one generation;
  generation ids never repeat (high-water floor).

Ring 1 (under S1–S6):

- P1 Mutation never precedes durable intent and prior evidence.
- P2 For any execution with crashes at arbitrary points (unbounded), followed
  by recovery: the store satisfies the K-invariants, `current` names a durably
  published generation, and each destination is either its prior or its
  intended identity — never a third state.
- P3 Cleanup never destroys evidence needed after a later crash.
- P4 Rollback reaches the target generation's state with a fresh transaction
  identity. Hooks are at-least-once and never auto-rolled back (0001 §3.8).
- Liveness ("eventually recovers") stays claim-gated and TLC-only.

`grip doctor --guarantees` and the website list exactly these, generated from
the Lean sources with their axioms.

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
  Storage.lean       S1–S6
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

The required check on `main` is one `gate` job: `test`, `ts-test`, `e2e`,
`proofs`. A release tag runs the same gate, builds the tarball, publishes.
Lean/Charon images are pinned by digest. Charon's nightly rustc lives only in
the `proofs` image; the shipped binary still builds on stable musl with rustls.

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

**F2 — Kernel extraction (≈4–6 weeks).** Move every ring-0 decision into
`gripsack-kernel`. Rewrite apply/rollback/recover/gc/activation as sans-IO
machines (§4). Build `gripsack-host` with the driver and the crash-injecting
storage. Port the remaining Verus kernels to Lean or demote them per §8.
Delete the explorers.
*Exit:* K1–K6 proved; no Verus; real CLI e2e green; `gripsack-store`,
`gripsack-policy`, `gripsack-fs` and `gripsack-process` gone.

**F3 — Protocol proofs (≈3–4 weeks).** `Storage.lean` (S1–S6). P1–P4 over
the translated machines. Generated claims and `grip doctor --guarantees`.
Delete TLAPS and the excess TLA+.
*Exit:* P1–P4 proved without axioms beyond S1–S6; ≤5 TLC models; website
claim page generated.

**F4 — Feature crates (≈2–3 weeks, can overlap F3).** Split the remainder of
`gripsack-exec` into `gripsack-sources` / `gripsack-runtime` / CLI; dissolve
`gripsack-config` and `gripsack-trace`; enforce dependency direction; prune
e2e.
*Exit:* layout of §6; no crate over ~15k lines.

**F5 — Expand rings (ongoing).** Per candidate: move the decision into the
kernel, state the theorem, prove it, delete the tests and models it makes
redundant. One PR per candidate.

**Handover reconciliation.** After F0, produce a landed/pending/dropped table
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
