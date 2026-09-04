# 0026 — Generation identity and path-centric transactions (0.22.0 fresh-eyes audit)

Status: **implemented** (0.23.0). Source: third fresh-eyes
review ("a credible transaction core, but not yet a completely closed
transaction model"). All nine findings verified against the code;
eight were real, one is adopted in part. The reviewer's own summary
is right: this is the natural completion of the journal, not a
redesign.

## Verdicts

| # | Finding | Verdict |
|---|---------|---------|
| 1 | Successful rollback overwrites tracked-copy drift | **Adopt** — the rollback planner is drift-aware: live == current → restore; live == target → no-op; else preserve + warn |
| 2 | One destination journaled twice in a rollback (module rename) | **Adopt** — rollback plans per-DESTINATION, one transition each. The apply-side merge ghost (same file, renamed module) is fixed by keying merge-block prune by (module, dest) |
| 3 | Generation numbers reused after rollback (`current + 1`) | **Adopt** — allocate `max(on-disk, current) + 1`; write_manifest is no-clobber (a pre-existing generation ID is a hard error) |
| 4 | Roll-forward via rollback breaks commit detection | **Adopt, stronger than asked** — the marker gains `previous_generation`; reconcile uses EXACT equality: current==target → committed, current==previous → uncommitted, else ambiguous → error. Roll-forward stays allowed (it's undo-the-undo); no new command |
| 5 | Cleanup can lose commit evidence (marker deleted before entries durable) | **Adopt** — two durability barriers: entries deleted + fsync, THEN marker deleted + fsync, in commit_run and both reconcile branches. Invariant: marker durably absent ⟹ entries durably absent |
| 6 | Intended post-state recorded AFTER the mutation | **Adopt** — protocol change: `record` persists prior AND intended-after before the mutation; `mark_after` is deleted. decide() goes three-way: live==intended → restore prior; live==prior → never landed, nothing to do; else → user edit, keep |
| 7 | File modes outside transaction identity | **Adopt in part** — atomic_write now preserves an existing destination's mode on content update (a real behavior bug: updates re-created files at 0644&umask). Full mode-in-identity (chmod-only drift detection) is a manifest/IR schema change — deferred with a note; exec-bit drift is already covered by the canonical hash |
| 8 | current-generation readers fail open | **Adopt** — `generations::current` becomes `Result<Option<u64>>`; unparseable `current` target is InvalidData, not None. Callers propagate at mutation paths |
| 9 | Rollback has no preflight | **Adopt** — the planner validates the target generation's store paths and entry sources before the first mutation; a missing artifact aborts before anything moves |
| — | Full persisted `Transaction` struct + TLA+ | **Form, deferred.** The marker gains previous_generation (exact equality) — that's the substance. A persisted transition log beyond per-destination entries adds recovery surface without new invariants; revisit if a third operation kind appears |
| — | `--force` flag for drift overwrite | **Not this round** — preserve+warn matches apply's drift semantics; a flag is a product decision for the owner |
| — | Install-order / hero / "no takeover" row wording | **Owner's taste** — recorded, not changed. The two guarantee qualifications the site needed (tracked-copy drift, generation immutability) become TRUE under this plan, so no site disclaimer is needed after all |

## Design notes

### The transition planner (rollback)

Both manifests normalize to destination-keyed maps; one transition
per destination:

- in current only → remove/restore-prior (journaled, drift-guarded
  as today)
- in both, live == current identity → restore target (journaled,
  intent = target identity)
- in both, live == target identity → no-op
- in both, live matches neither → keep + warn (drift preserved)
- in target only → restore/deploy (journaled)

Preflight before the first mutation: every target store path exists
and every entry source resolves inside it — else abort.

restore_entry splits into plan (`compute_restore` → intent identity +
bytes/target to write) and execute (the capability write), so the
journal records intent BEFORE mutation without re-reading the
destination afterward.

### Journal v2 wire shape

```json
{ "dest": "...", "prior": {...}, "after": "<intended identity>" }
```

`after` is now required and means INTENDED post-state, written by
`record` before the mutation. The run marker:

```json
{ "previous_generation": 3, "target_generation": 1, "op": "rollback" }
```

Pre-0.23 markers (no `previous_generation`) reconcile by the 0.22
direction rule (apply: current >= target; rollback: current <=
target) — correct for those versions' semantics.

### Reconcile decision

```text
current == target   → committed: two-barrier cleanup
current == previous → uncommitted: three-way decide per entry, restore
else                → ambiguous: ERROR, journal retained (fail closed)
```

## Acceptance

- Reviewer's regression matrix, as e2e (kill-point via
  GRIPSACK_CRASH_AFTER): rollback with a drifted tracked copy
  (preserved); module rename sharing a destination (one transition,
  true prior restored after a kill); rollback 3→1 then apply →
  generation 4, and generation 2 untouched; roll-forward 1→2 killed
  mid-run → recovered as uncommitted; chmod-preserved update (0600,
  0755); unreadable `current` pointer → apply errors before mutating.
- All existing unit + e2e green (journal unit tests adapted to the
  v2 protocol — deliberate; the assertions encode the protocol
  change itself).
- Docker gates + macOS e2e green; changelog; core-v0.23.0.
