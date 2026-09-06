# 0037 - Rollback activation

Status: **implemented in 0.33.0**. The roadmap's "rollback adapters"
item, confirmed as the priority by the 0.31.0 review (files and
running processes disagreed after rollback).

## Problem

`grip rollback` restored generation N's files but left services and
caches as the NEWER generation left them: a rolled-back service keeps
running the newer config until something else restarts it. The
restored environment and the running environment disagree.

## Design (the 0032 seam, no new machinery)

Rollback joins the durable-activation protocol:

1. After the flip, write the pending record for the TARGET
   generation: its recorded intents (ModuleState.intents, 0035 F9),
   minus on_remove. Modules that exist now but not in the target are
   undeclared BY the rollback — their recorded on_remove hooks join
   the record.
2. commit, run the adapters (idempotent refreshes; failures warn,
   never un-rollback — 0001 §3.8), clear the record.

Crash coverage comes free: a kill mid-adapters leaves the pending
record; the next run resumes it (0032's machinery, TLC-checked).

## Deliberately not done

- No per-entry diffing of intents: the target generation's intents
  all re-run (they're idempotent; the refresh is the point).
- Service adapter semantics unchanged (`enable --now` + daemon-reload)
  — a config-only change under the same unit name restarts via the
  same commands. A finer "restart only when the unit file changed"
  diff is its own item if anyone asks.
