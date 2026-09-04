# 0027 — Provable transactions and validated generations (0.23.0 fresh-eyes audit)

Status: **implemented** (0.24.0). Source: the fourth
fresh-eyes review ("a credible generation-transaction design, but the
implementation can still commit a generation when some destination
operations silently failed, and its persisted generation state is not
yet treated as strictly validated authority"). All nine findings
verified against the code. All nine real. The two recurring principles
are the release's theme:

> A transaction must not commit unless every destination is verified
> to match its declared result.
>
> Persisted state is never interpreted as absent merely because it
> could not be read or validated.

## Verdicts

| # | Finding | Verdict |
|---|---------|---------|
| 1 | bool/Option mutation helpers + discarded outcomes → commit despite failed mutation | **Adopt (P0)** — the `journaled` wrapper gains a postcondition check: after the mutation, re-read the destination through the pinned capability and require live == intended. Plus the bool helpers become `Result`. One central check, every journaled path |
| 2 | GC fail-open on generation enumeration | **Adopt (P0)** — `generations::list` becomes `io::Result`; GC aborts on any enumeration error, requires current ∈ inventory, and validates every retained manifest before computing the deletion plan |
| 3 | current manifest `.ok()` in apply/rollback | **Adopt (P0)** — known current + unreadable manifest blocks every mutating command |
| 4 | persisted generations parsed, not validated | **Adopt (P0)** — `read_manifest` becomes the strict boundary: embedded number == directory id, no duplicate destinations (case-folded), hash fields well-formed. Prior-blob readability stays lazy (restore-time errors already abort the transaction) |
| 5 | capture through one capability, mutation through a reopened path | **Adopt (P1)** — prune/rollback helpers take the pinned `(Dir, name)` instead of reopening the absolute path |
| 6 | journal prior loses the original mode | **Adopt (P1)** — `PriorSerde::File` gains `mode`; capture records it, restore applies it exactly (temp → chmod → fsync → rename). Covers file→symlink→crash |
| 7 | rollback planner's Option-collapsed state reads | **Adopt (P1)** — `live_intent_identity` and friends go `io::Result<Option<…>>`; NotFound-only-is-absent |
| 8 | generation not published as one immutable object | **Adopt (P1)** — manifest + profile build in `generations/.staging-<N>/`, then one no-clobber rename. Rollback only backfills a MISSING profile (byte-identical render), never rewrites history |
| 9 | generation IDs reused after GC of the tip | **Adopt (P1)** — durable high-water mark (`generations/high-water`), allocated and persisted under the lifecycle lock |
| — | Typed `TransitionDisposition` algebra replacing all bools | **Subsumed** — the postcondition check in `journaled` delivers the invariant ("no commit without verified post-state") without a type-theatre refactor; the bool helpers that encode real failures become `Result` where failure is possible |
| — | Persistent in-repo fuzz harnesses | **Roadmap** — P2; deterministic smoke budgets first, longer runs scheduled |
| — | Dedicated site safety page | **Roadmap** — good idea, owner-visible docs work |
| — | Data-only TOML/JSONC frontend | **Rejected, third time** — plans 0020/0025 stand: two frontends is the parity-corpus trap; modules-as-values is the language asset. Not on the roadmap |
| — | curl\|sh install ordering | **Already on the roadmap** under the signed-channel/attestation item |
| — | Changelog site lag at release time | **Noted** — deployment timing, not code; the site rebuild dispatch is best-effort by design |

## Implementation order (each step compiles green)

1. **Postcondition verification** — `journaled` verifies live ==
   intended after the mutation (and after a removal, live == absent);
   mismatch is a hard error, the run compensates via reconcile. The
   `false`-returning helpers (`restore_prior`, `remove_entry_deployed`,
   `remove_or_restore_prior`) become `Result<bool>` — Err for real
   failures, bool only for the drift-keep policy outcome.
2. **Journal prior mode** — schema, capture, restore, tests
   (0600/0640/0755, file→symlink→crash).
3. **Fail-closed GC** — `list()` Result; gc preflight.
4. **Validated manifests** — strict `read_manifest`; current-manifest
   fail-closed in apply + rollback CLI.
5. **Complete-generation publish** — staging dir + no-clobber rename;
   rollback backfills a missing profile only.
6. **High-water mark**.
7. **Pinned threading + planner error algebra** (findings 5+7 together —
   same code).

## Acceptance — the reviewer's regression list, as tests

- generations/ unenumerable → gc deletes nothing (unit)
- prior blob unreadable → apply/rollback abort before flip (unit)
- mutation fails (chmod 0o555 parent) → transaction aborts, journal
  restores (e2e)
- current manifest malformed → apply/rollback/gc block (unit)
- manifest number ≠ directory → rejected (unit)
- duplicate destinations in a persisted manifest → rejected (unit)
- 0600 file → owned symlink → kill → recovery restores bytes AND 0600
  (unit + e2e)
- rollback 3→1, gc removes 2+3, apply → generation 4+ (e2e)
- profile render failure before flip → no `generations/N` visible
  (unit)
- rollback to a historical generation → no byte inside it changes
  (e2e)
- PermissionDenied destination inspection → rollback aborts (unit)

## Release

core-v0.24.0. No IR/frontend changes.
