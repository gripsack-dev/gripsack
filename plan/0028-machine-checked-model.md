# 0028 — The machine-checked transaction model

Status: **implemented** (merged; no release — no behavior change).

## Tool choice, on the record (an external TLA+/Stateright comparison)

Considered against this round's evidence. **Stateright**: Rust-native
and charming, but last commit 2025-07-27 with an open toolchain-compat
PR since October — too weak a pulse for a load-bearing verification
dependency, and its actor-model API is a poor fit for a single-writer
protocol. **TLA+ + TLC**: the right tool for distributed/temporal
protocols, but ours is single-process crash-nondeterminism with
safety-only invariants — and its spec would be a PARALLEL
implementation, the exact drift risk that produced four rounds of
decision-logic bugs. So: the shipped proof is a hand-rolled
exhaustive explorer in Rust (`journal/model.rs`, cfg(test), zero
deps) driving the extracted real classify/decide_from — and the TLA+
spec (`specs/Transaction.tla`, TLC in the docker `model` gate) is the
learning artifact + independent second opinion. Both agree: shipped
protocol clean (TLC: 95 distinct states, no error, 4 configs; Rust:
76 states, 4 configs, zero violations), legacy 0.22 markers provably
broken in both. The explorer is mutation-calibrated: a one-line
classify mutant produces 136 violations.

**Owner override of the
"quiet release cycle" trigger — the protocol's core state machine has
been stable since 0.23; 0.24 added validation around it, not new
transitions). This document is written to be read: it is the learning
artifact for what the model is, why it has this shape, and exactly
what it does and cannot prove.

## What a state-machine model is

A filesystem transaction protocol is, underneath the Rust, a state
machine: a small set of states (what the destination holds, what
`current` points at, what's in the journal) and transitions (begin,
record, mutate, flip, clean up, crash, recover). Every bug in four
review rounds lived in the EDGES of that machine — a crash landing
between two steps, a comparison that misclassifies the resulting
state.

Tests sample the machine: each test drives one path and checks one
outcome. The 0.22 roll-forward bug survived because nobody wrote the
test "rollback to a NEWER generation, killed before the flip." A model
checker doesn't sample — it enumerates. You describe the machine's
states and legal transitions; the explorer walks every reachable
state, in every order, and evaluates an invariant in each. If a
violating schedule exists, you get the exact step sequence that
produces it. If none exists, you get something tests can never give:
**no schedule violates the invariant** — a proof, over the model.

## The three commitments this design makes

### 1. The checked decision logic IS the shipped code

The classic failure of model checking: the model is a second
implementation, and the two drift apart until the proof covers a
protocol nobody runs. Our answer: the two places where every
historical bug lived — *is this interrupted run committed?* and
*should this destination be restored?* — are extracted from
`journal.rs` as pure functions (`classify`, `decide_from`), called by
the production reconcile path AND by the model. The checker explores
states; the decisions at each state are the real ones.

What stays modeled (not shared code): the filesystem mechanics —
atomic writes, fsync barriers, the flip's rename. Those are covered
against the real binary by the kill-point e2e (`GRIPSACK_CRASH_AFTER`
windows) and the capability tests. Two layers, each honest about its
boundary:

```text
  protocol decisions  →  model-checked (this document's artifact)
  filesystem mechanics →  kill-point e2e + unit tests on real fs
```

### 2. The durability model is explicit, not assumed

A process kill and a power loss are different events, and conflating
them is how cleanup-ordering bugs hide:

- **kill -9**: every write the process issued reaches disk eventually
  (the kernel's page cache outlives the process). Volatile state is
  simply *lost from the program's view* — it all persists.
- **power loss**: only fsync'd data is guaranteed. Writes since the
  last fsync may persist in ANY subset — the disk is free to reorder.

The model therefore tracks two copies of the filesystem: `volatile`
(what the running process has written) and `durable` (what has
survived an fsync barrier). Steps write to volatile; barrier points
flush volatile into durable. A kill exposes volatile. A power loss
exposes durable plus EVERY SUBSET of the pending volatile writes —
which is precisely the nondeterminism that made the journal's
two-barrier cleanup necessary (0.23): "marker durably gone" must
imply "entries durably gone", and only an explicit barrier pair
proves it.

### 3. The oracle is the product's sentence

From plan/0020, restated for the machine:

> After any crash and any recovery, every managed destination is the
> previous generation's content, the committed target's content, or a
> user edit made after the crash that is kept — never an unexplained
> mixture — and the journal drains completely: no marker, no entry.

Plus the fail-closed arm: when `current` matches neither the marker's
previous nor its target, recovery must change NOTHING and keep the
journal intact.

## The model's vocabulary

One destination is enough — the protocol journals each destination
independently, so multi-destination runs are this machine iterated;
the shared part (marker, flip, cleanup) is modeled. Contents are
abstract: `0` = the old deployment, `1` = the new one, `2` = a user
edit. Both run directions are exercised (apply 1→2, rollback 2→1):
since 0.23's exact-equality classification, the op kind is provably
irrelevant to recovery — the model demonstrates that by checking both
with the same oracle.

The step sequence of a run, with `[F]` marking an fsync barrier:

```text
begin_run(prev, target)   [F]   marker durable
record(prior, intended)   [F]   entry durable — BEFORE the mutation
mutate: dest := new       [F]   atomic_write: temp, fsync, rename
verify postcondition            (read-only; no state to model)
flip: current := target   [F]   symlink rename — the commit point
cleanup entries           [F]   barrier 1
cleanup marker            [F]   barrier 2
```

A crash may replace any prefix of this sequence, and a power loss may
persist any subset of the writes since the last barrier. Then:
optionally the user edits the destination; then the next run's
reconcile resolves the journal.

## What the explorer proved (and the counterexample it keeps)

Checked: every interleaving of both run directions × every crash
point × both crash kinds × every power-loss subset × user-edit-or-not
— the oracle holds in every terminal state, and no reachable state is
misclassified.

Kept as regression documentation: swapping `classify` for the 0.22
direction rule (`committed ⟺ current <= target` for rollbacks) makes
the explorer find the counterexample in milliseconds — a roll-forward
killed before the flip reads committed and abandons a half-restored
destination. The test asserts the violation EXISTS under the old rule
and cannot exist under the shipped one. That is the difference between
"we fixed the bug" and "the bug class is now unrepresentable."

## What this does not prove

- That the Rust implementation matches the model for the fs mechanics
  (the kill-point e2e's job — it caught the decide() identity bug the
  model structurally cannot see, since identity mismatches are
  type-level, not protocol-level).
- Anything about concurrency between grip processes (the lifecycle
  lock serializes runs; the model is single-writer by design).
- Liveness/performance properties — the checker proves safety ("bad
  things never happen"), not liveness ("recovery eventually runs").

If the protocol next grows (a third op kind, multi-writer), the model
grows first — that ordering is the point of having it.
