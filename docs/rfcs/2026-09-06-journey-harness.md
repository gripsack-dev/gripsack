---
rfc: "2026-09-06-journey-harness"
title: "Stateful journey testing"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-06"
updated: "2026-11-20"
issue: "https://github.com/gripsack-dev/gripsack/issues/288"
milestone: "core-v0.34.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0038"
---

# RFC: Stateful journey testing

> **Backfill note (2026-11-20).** The journey campaign supplies bounded evidence of user flows, not an exhaustive concurrency or crash proof.
> [Original design 0038](https://github.com/gripsack-dev/gripsack/blob/7924bc2b844d109dbb520040f5954ad237007ff3/plan/0038-journey-harness.md) · [tracking issue #288](https://github.com/gripsack-dev/gripsack/issues/288). The text below is the historical record.

Status: **implemented in 0.34.0**. The 0035 meta-lesson, owner-approved:
our e2e tests what we built; nobody fuzzed what a user DOES over time.
The 0035 headline bugs (spelling-delete, verify-retry, stale builds)
were all journey-shaped: state evolving across runs, not single runs.

## Design

`e2e/test_journey.py`: seeded random operation sequences against a
sandbox, driving the real `grip` binary. The world: a tracked copy,
an owned link (fileFetch), a merge block, and a custom hook. The
operations: edit repo content, apply, drift a live file, apply,
take-over, remove/re-add a module (with a spelling variant), apply,
update a payload, rollback to a random generation.

The harness keeps a per-destination expectation model (what the
semantics SAY the live file should hold, per generation), and after
every operation asserts the oracles:

- every declared destination holds the expected content (drift
  preserved exactly when the semantics say so)
- every undeclared destination is absent or restored to its origin
- `grip check` passes; `grip store verify` passes; `current` resolves
- a still-declared file is NEVER deleted; a failed apply leaves the
  machine as it was
- generations are monotonic and never reused

Seeds are fixed and printed on failure — a red run reproduces with
the same seed. Oracle failures become named regression tests.

## What it is not

Not a concurrency model (the lifecycle lock holds; parallel-apply
races stay the TLA+ specs' domain). Not a replacement for the unit
harnesses — it covers THEIR blind spot: the seams between eval, ops,
and time.
