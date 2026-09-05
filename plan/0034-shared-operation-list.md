# 0034 - One shared operation list for plan/apply/rollback

Status: **implemented in 0.30.0**. The 0.27.0 review's central architectural
point, pulled forward by owner decision (overriding the soak — the
consolidation IS the correctness work). The shape 0007/0026
converged on, named by the review: one planner, three consumers.

## Problem

Three code paths answer "what happens to this destination":
render.rs's diff_section (plan preview), deploy_entry (apply), and
rollback's plan()/execute(). They have already diverged twice in
production-visible ways (plan omitting run-step mutations; rollback's
identity domains). Every future fix lands three times or lies once.

## Design

One planner produces the operation list; the consumers are thin:

```
DesiredState (from IR + fetched content, or from a target manifest)
    │
    ▼
plan_ops() ──► Vec<Op> ──┬──► plan renders it (the preview)
                         ├──► apply executes it (journal precondition
                         │    re-validates each op's observation)
                         └──► rollback plans with the target
                              generation's manifest as the desired
                              state
```

An `Op` carries, per the reviewer's list: the destination, the source
provenance (store path + entry key), the observed object identity at
plan time, the intended end state (journal domain), its authority
(Fresh / Ours / TakeOver / PreservedDrift — the lineage answer), and
its recovery behavior (the prior to capture/restore). Kinds: `Link`,
`Write` (tracked copy / template), `MergeUpsert`, `Remove` (prune,
restore-prior, block splice), `Satisfied`, `Preserved`, `Refused`
(plan renders; apply errors), `RunEffect` (opaque — marked, never
pretended previewable), `Activate` (the durable intents, 0032).

Content identity for fetched payloads stays deferred at plan time
(pin-resolved at apply) — the op exists, its identity is filled at
execution. Plan/apply agreement is by construction: one planner.

## The honest contract (unchanged)

Plan is a preview computed from observed state. Apply re-validates
every observation at the mutation (the journal precondition stays).
External state between preview and execution is never predictable;
the op list makes the DECISIONS identical, not the world still.

## Modeling

- The protocol layer is already covered: a journaled, precondition-
  verified mutation IS Transaction.tla's step, whether the mutation
  comes from an op or from 0.29's inline code. Activation.tla covers
  the adapter op. No new TLA+ spec: the op list is data, and the
  state machines over it are modeled.
- The NEW property — plan never disagrees with apply — is a
  refinement property, and the strongest check is the Rust explorer
  driving the shipped planner over the state space (as it already
  drives plan_copy/plan_link). TLA+ would model a parallel
  approximation of the planner; the harness runs the real one.
- The lineage explorer extends: the planner's authority decisions
  (Fresh/Ours/TakeOver/Preserved) over the enumerated sequences.

## Acceptance

- `grip plan` output IS the op list render; `diff_section`'s parallel
  compare logic is deleted.
- rollback constructs ops through the same planner (Transition dies).
- The apply transcript for a state equals the plan transcript for
  that state (e2e: plan-then-apply over the reviewer's divergence
  cases, plus the golden pair).
- All gates green; the explorer drives plan_ops authority branches.
