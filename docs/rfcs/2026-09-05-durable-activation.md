---
rfc: "2026-09-05-durable-activation"
title: "Durable activation hooks"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-05"
updated: "2026-11-20"
issue: "https://github.com/gripsack-dev/gripsack/issues/282"
milestone: "core-v0.28.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0032"
---

# RFC: Durable activation hooks

> **Backfill note (2026-11-20).** 0037 extended activation to rollback; later transaction-bound intent outcomes are subsequent amendments.
> [Original design 0032](https://github.com/gripsack-dev/gripsack/blob/7924bc2b844d109dbb520040f5954ad237007ff3/plan/0032-durable-activation.md) · [tracking issue #282](https://github.com/gripsack-dev/gripsack/issues/282). The text below is the historical record.

Status: **implemented in 0.28.0**. Closes 0030 #14 (activation hooks
outside durable state), the last crash window with no record.

## Problem

Adapters (fc-cache, update-desktop-database, systemctl, custom hooks)
ran after the flip with no durable record. A kill between the flip
and the adapters — or mid-adapters — silently skipped them: the
generation is committed, the unit file is deployed, and the service
never restarts until the next unrelated apply.

## Design

A pending record closes the window, written BEFORE the flip:

1. `apply` computes the deduped intent list, then writes
   `$GRIPSACK_HOME/activation.json` — `{generation, intents}` —
   atomically, before the flip. Pre-flip is what closes the window
   fully: a crash before the flip leaves a pending record naming a
   generation that never became current; the next run discards it.
   **Verified by mutant**: the spec variant that writes the record in
   a separate step after the flip (the pre-0.28 shape) reaches
   `current = g ∧ idle ∧ pending = NONE ∧ never-activated` and
   violates NoSilentSkip; the shipped shape is TLC-clean.
2. Flip, then adapters run. Failures warn and never roll back
   (0001 §3.8 stands). All attempted → the record is removed.
3. Every lifecycle run (apply, rollback) resumes first, after
   reconcile, under the lock: a pending record naming the CURRENT
   generation re-runs its intents; anything else is discarded
   (superseded or rolled back — never run adapters for a generation
   that isn't current).

Semantics that follow, documented:

- **Intents may run more than once across a crash** — they are
  idempotent refreshes by contract (caches, daemon-reload, enable
  --now); `custom` hooks must be written idempotent. This is the
  at-least-once trade for never-skipped.
- **A crashed-then-superseded generation's adapters never run** —
  the newer generation's own pending record covers the refresh.
- **A failed adapter does not retry** on the next run (it warned; the
  record is cleared). Durability covers crashes, not poisoned hooks.

## The model

`specs/Activation.tla`, TLC-checked in the `model` gate (CI).
Variables: `current`, `pending`, `activated`. The core invariant:

> A generation that committed with intents is either activated or has
> a pending record — a skip is unreachable.

Plus: adapters never run for a non-current generation; a pending
record never survives a run that started. The spec models crash at
every step boundary (kill) — no power-loss subsets here because every
write in the protocol is atomic (temp+rename+fsync), so durability
subsets don't arise (unlike the journal's entry stream).
