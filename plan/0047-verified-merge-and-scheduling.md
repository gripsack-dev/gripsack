# 0047 — Verified merge splice, graph closures, and scheduler transitions

Second verification round (after 0046's toolchain pilot), taking the
merge and scheduling foundations in the roadmap's ROI order. Also
records two tooling decisions: TLAPS is the next proof step (not
Lean), and Lean is deferred to a separate-repo educational exercise.

## What lands

### 1. The merge splice kernel (`gripsack-policy::merge`)

`ManagedBlockSet::splice` is the byte-integrity heart of merge mode:
given the hosting text, the owned block ranges and a replacement, the
output must keep every foreign byte verbatim and in order. The kernel
moves to the policy crate over `&[u8]` — actual input/output bytes,
per the handoff ("prove against actual bytes, not an abstract
'foreign text preserved' flag"):

- `splice_bytes(text, spans, replacement)` with `requires`: spans
  sorted, non-overlapping, in bounds. (UTF-8 boundary alignment is the
  parser's obligation — Layer 2 — not the kernel's; the exec wrapper
  converts through `String::from_utf8` and fails closed, never
  panics.)
- `ensures`: output equals the spec mirror exactly (prefix-recursive
  `spec_splice`, matching the loop's induction direction — the
  retention proof's lesson).
- Lemmas: single-span identity replacement is the identity function
  (`splice(t, [s], t[s]) == t`); the complement intervals of the text
  appear verbatim and in order in the output.
- Calibration: a mutant dropping the final tail fails the spec-mirror
  postcondition; a mutant skipping the first-span replacement fails
  the identity lemma.

The `ManagedBlockSet` wrapper (`remove`, `upsert`) keeps its shape and
calls the kernel. Duplicate-block removal is splice with a
multi-span input — covered by the same contract.

### 2. Build-closure graph kernel (`gripsack-policy::graph`)

`build_closure_names` (transitive build-edge closure) and
`build_only_modules` (build-minus-runtime set) move behind a
name-indexed kernel: the IR adapter hands the kernel a
`Vec<(name, [build-edge targets])>` view once; the kernel computes
closures over indices. Contracts:

- closure is exactly the build-reachable set — soundness (every member
  reachable by a build-edge path) and completeness (every reachable
  name is a member), with reachability as a spec-level path
  definition; the worklist proof discharges both;
- termination on cyclic input (decreasing measure over
  unvisited-seen structure);
- ordering-only edges never enter the closure (they're not in the
  kernel's input view — unrepresentable, not just unused).

If the least-fixpoint proof fights the solver beyond a reasonable
effort, the fallback is a bounded-fuel reachability spec with the
bound recorded in the ledger — documented, not silent.

### 3. Scheduler transitions (`gripsack-policy::schedule`)

The threaded `run_all` keeps its Mutex/Condvar mechanics; the
DECISIONS become an index-based pure kernel (`PureScheduler`):
`start` requires all dependencies finished and the error latch clear;
`finish_ok` releases dependents exactly when their last dependency
completes; `finish_fail` latches. Proved invariants (`inv()` preserved
by every transition, plus the safety readings):

- a module starts only after every dependency has finished
  successfully (the `remaining` count is the exact not-yet-finished
  dependency count, by construction from the edge list);
- each module starts at most once;
- after any failure, no further starts (so a failed dependency never
  authorizes a consumer).

The worker threads translate names↔indices at the boundary; the
mutex/condvar bridge stays tested (journeys, e2e), never wrapped and
claimed verified (handoff §5.6).

### 4. Merge parser range invariants (Layer 2, time-boxed)

`scan`'s ranges are sorted, non-overlapping, in-bounds and
line-aligned by construction — the loop invariant is small. Prove
that parse output satisfies `splice_bytes`' `requires` for all
inputs, malformed markers included (typed errors, never garbage
ranges). Time-boxed: if the marker grammar fights the solver, land
with the bound recorded in the ledger.

## Tooling decisions

- **TLAPS is next** (after this round's follow-ups): an inductive
  safety proof over the EXISTING `specs/Transaction.tla` — no new
  model, bounded TLC evidence upgrades to unbounded. Liveness stays
  TLC's (TLAPS's temporal reasoning is limited).
- **Lean/Aeneas is deferred**: a separate-repo educational exercise,
  not a dependency. If it matures, CI can pin and pull that repo's
  checked proof artifacts as an advisory gate — the core repo never
  carries the Lean/Charon/Aeneas/mathlib toolchain, and drift is
  impossible by construction (a pinned artifact either matches the
  named production commit or the gate says so).

## Solver lessons (recorded, not re-learned)

The scheduler round surfaced four Verus behaviors that each cost
verification rounds; future kernel work should reach for these
directly:

- **`assert forall … by` assumes the antecedent only under
  `implies`.** With the spec-level `==>` spelling the by-block does
  NOT get the antecedent's facts (observed on Verus
  0.2026.09.06.8dea4a2; minimal repro verified both ways). Write
  `assert forall|…| … implies …` in proof blocks.
- **Snapshot entry state into ghost locals; never reason about
  `old(self)` after a mutation.** Post-mutation `old(self).field@[i]`
  terms failed to connect to current-state facts even with matching
  triggers; `let ghost pre_x = self.x@;` at entry gives plain locals
  that cannot be re-seated.
- **int↔usize comparisons do not collapse across casts.** `i != m as
  int` does not yield `i as usize != m`. `lemma_cast_collapse`
  (schedule.rs) discharges it; the seq-length bound comes from
  `axiom_spec_len` (broadcast).
- **Loop bodies drop pre-loop facts about untouched fields.** Facts
  like "module is not finished yet" must be restated as loop
  invariants even when no statement in the loop writes that field.

Layer 2's bound: the marker-grammar parser port (scan → valid_spans)
was time-boxed and exceeded; the splice kernel's admission predicate
is the contract boundary until it lands (MERGE-SPLICE-001 records the
exclusion).

## Non-goals

No permission-policy, marker-style or CRLF proof work (0044's models
and e2e own those). No scheduler threading verification. No Lean in
this repo.

## Acceptance

- `verify` gate: the three kernels' contracts hold, each with a named
  mutant failing its named postcondition; obligation floor updated.
- `ManagedBlockSet` unit tests, the merge fuzz target, and
  `test_merge_boundaries.py` exercise the kernel-driven path
  unchanged; journeys and scheduler e2e drive the kernel-driven
  `run_all`.
- Ledger gains `MERGE-SPLICE-001`, `GRAPH-CLOSURE-001`,
  `SCHEDULER-001` (`checked`) with the Layer-2 bound recorded.
