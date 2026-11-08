# 0056 — Verified foundation: a Lean-proved kernel, rings outward, and a trim

- Status: proposed (owner review)
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
