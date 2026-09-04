# 0025 — Transaction coverage: rollback, prune, env profile (0.21.1 fresh-eyes review)

Status: **implementation plan, accepted**. Source: a fresh-eyes
architecture review of 0.21.1 ("credible transaction core, but
transaction coverage is still incomplete around rollback,
undeclaration, and exported environment state"). Every finding was
verified against the code before deciding. All four main findings
were real; two additional findings (5, 7) were real — one of them a
regression introduced by 0021 itself.

## Verdicts

| # | Finding | Verdict |
|---|---------|---------|
| 1 | Rollback is not journaled | **Adopt** — run it through the run-marker protocol |
| 2 | prune_undeclared bypasses the journal | **Adopt** — journal prune mutations |
| 3 | Env profile outside the generation commit | **Adopt** — generation-local profile, sourced through `current/` |
| 4 | Late apply errors lack one compensating path | **Adopt** — any post-schedule failure runs the run-rollback |
| 5 | capture_prior fails open (`.ok()?`) | **Adopt** — `Result<Option<Prior>>`, only NotFound is None |
| 6 | reconcile cleanup lacks dir fsync; marker reads fail open | **Adopt** — fsync after reconcile deletions; NotFound-only-is-absent for run marker and `current` |
| 7 | EXDEV copy drops mode/exec/read-only bits, no tree fsync | **Adopt** — a 0021 regression: the rename path preserved everything, the copy path re-creates files with default perms. Fix + test the copy path directly |
| 8 | prune/rollback removals via ambient paths | **Adopt** — removals go through the pinned dest capability too |
| — | Grand `Transaction` struct owning all mutation | **Not as form.** The journal + lifecycle lock already ARE the transaction; after 1–4 every destination mutation is journaled. A unifying type adds no guarantee beyond that — revisit if a third caller shape appears |
| — | TOML/JSONC data-only frontend | **Rejected, again** — plan/0020's reasoning stands verbatim: two frontends is the parity-corpus cautionary tale, and modules-as-values is the product's language asset. The 40MB Deno download is once per machine |
| — | TLA+/Stateright model | **Deferred, again** — 0020's trigger was "when the protocol next grows". It grows in this release (rollback/prune join it). The model is queued behind the failpoint e2e below; a machine-checked spec is worth it once the protocol stops moving for a full cycle |
| — | Full failpoint matrix | **Partially adopt** — targeted kill -9 e2e at the two newly closed windows (prune-before-flip, rollback-restore-before-flip) rather than a per-syscall injection framework this round |
| — | install.sh attestation verification, signed channel manifest | **Queued** — 0020's queue item 4 stands; needs a design that doesn't make `gh` an install-time dependency |
| — | Hero rewrite / comparison table nuance | **Owner's call** — site taste items; this round fixes the guarantee language and status drift only |
| — | Freeze ecosystem breadth for a cycle | **Accepted as process** — no new fetcher/resolver kinds this release (none were planned) |

## Implementation

### A. Journal rollback (finding 1)

`rollback` currently: prune-styled removals → restore_entry per entry
→ flip → render env. No run marker, no entries. Crash mid-restore
leaves mixed destination state with `current` unchanged and no
record.

Fix: rollback runs the same protocol as apply:
`begin_run(home, target)` (target = the OLDER generation), each
destination mutation journaled (capture → record → mutate →
mark_after), flip, `commit_run`. The existing reconcile decision
works unchanged: a rollback marker whose target is AHEAD of current
never happens (target < current by construction), so a crashed
rollback reads uncommitted (target > current is false... wait:
uncommitted means current < target; for rollback current > target,
so `current >= target` reads COMMITTED even when the flip never
happened). **This needs care**: reconcile's rule is `current >=
marker.target → committed`. For rollback, target < current before
the flip, so a crashed rollback would misread as committed and skip
restoring.

Resolution: the marker gains an operation kind (`apply` | `rollback`)
— for rollback the committed condition inverts: committed iff
`current <= target` (the flip moved current BACK to target). Wire
format change to journal/run.json; pre-1.0, no compat owed, but a
malformed/old marker hits the quarantine path (fail-closed, correct).

### B. Journal prune (finding 2)

apply's prune phase mutates destinations directly (block removal,
prior restore, file removal) after the scheduler and before the
manifest write + flip. A kill between prune and flip leaves gen-N
destinations pruned with gen-(N-1) current and no record.

Fix: prune mutations go through `journaled` like deploy's: capture
prior, record, mutate, mark_after (after = "absent" for removals —
decide() already restores priors for missing-after entries, and a
recreated file reads as user content and wins). The run marker from
begin_run already covers the window; commit_run clears.

### C. Generation-local env profile (finding 3)

Today: `$GRIPSACK_HOME/env/profile.sh`, rendered before the flip on
apply (new profile + old current window) and after the flip on
rollback (committed rollback + stale profile window).

Fix: render into `generations/<N>/env/profile.sh` before the flip
(both apply and rollback), sourced through
`$GRIPSACK_HOME/current/env/profile.sh` — a stable path string that
follows the flip atomically. Removes both windows structurally; the
`env/` directory goes away (stale `env/profile.sh` removed once on
next apply). The profile header's sourcing instruction, e2e, and the
site docs update. Changelog calls out the path change for users who
source it (pre-1.0 breaking-ok; the old path is removed, not
shadowed).

### D. One compensating path for late apply errors (finding 4)

Post-schedule failures (lock write, prune, manifest, env render,
flip) return early with journaled-but-uncompensated deploys; the
next run's reconcile is the only cleanup, but the website says a
failed apply restores before returning. Fix: a small guard in apply
— any error after the scheduler runs `deploy::run_rollback` exactly
as a scheduler failure does, before returning the error. (The flip
already happened → no rollback; commit_run covers.)

### E. capture_prior strictly fallible (finding 5)

`capture_prior` returns Option and `.ok()?`-collapses permission/
I/O/UTF-8 errors into "no prior" — then takeover proceeds and the
pre-adoption state is unrecoverable, breaking the product's central
promise. Fix: `Result<Option<Prior>>`; only NotFound → Ok(None);
non-UTF-8 symlink targets error like journal::capture does (the
lossy conversion there goes away). Both takeover call sites in
deploy_entry propagate.

### F. reconcile durability + fail-closed reads (finding 6)

reconcile deletes entries and the run marker without fsyncing the
journal dir — a power loss can resurrect them. Fix: fsync the
journal dir before returning from both reconcile branches. Also:
`run_marker` maps any read error to None (→ uncommitted → restore),
and `current_in` maps any error to None — in recovery, unreadable
commit evidence must ERROR, not choose a branch. NotFound-only
means absent; anything else propagates.

### G. EXDEV copy correctness (finding 7 — a 0021 regression)

`copy_into_dir` recreates files with `Dir::write` — default perms.
read_only_files ran on STAGING before the copy, so the copy loses
exec bits AND the store's read-only policy; no fsync of copied
files/dirs either. The rename path preserved all of it. Fix:
copy_into_dir copies permissions (and fsyncs each file; dirs fsync'd
on the way out), then the parent fsync in publish_dir covers the
final rename as today. Test: call the copy path directly (staging →
sibling under a capability) asserting exec bit + read-only + symlink
survival. A true two-mount EXDEV test is container-layout-dependent;
noted, not built this round.

### H. Removals through the dest capability (finding 8)

remove_entry_deployed / prune / rollback use ambient
`std::fs::remove_file`. They already open dest_capability for
writes — extend it to removals (remove_file via the pinned Dir).

### I. Site + CI

- Replace "one atomic flip" claims with the accurate line:
  "journalled destination updates with an atomic generation commit
  and crash recovery" — and after A–D land it becomes true for
  rollback/prune too; the site's guarantee language gets the
  reviewer's table (store publish atomic / generation flip atomic /
  destinations journaled / external PM effects best-effort).
- pre-alpha vs alpha wording drift in settings docs → alpha.
- Pin third-party GitHub Actions to commit SHAs (checkout, setup-node,
  setup-deno, dtolnay/rust-toolchain, taiki-e/install-action,
  attest-build-provenance, upload/download-artifact) with the version
  in a comment. dependabot handles SHA bumps.

## Found while implementing: the drift guard never matched

The crash-window e2e (acceptance tests for §A/§B) exposed a latent
bug predating this round: `journal::decide` compared `mark_after`'s
identity against RAW sha256 of the destination bytes, but deploy has
always recorded `canonical_bytes_hash` (type tag + exec byte +
contents). The two never match, so production reconcile took the
drift-guard's Keep branch for every file entry — "changed since the
interrupted run — your edit stands" — and restored nothing. The
journal's own unit tests and the two recovery e2e tests pinned the
buggy pairing (they crafted entries with raw sha256), which is why
it survived three hardening rounds: the tests were as wrong as the
code. Fixed in decide (+ post_identity for the new journaled paths);
the tests now craft/use the canonical identity. This is the strongest
argument yet for the reviewer's kill-point testing over
crafted-state testing — the crafted states encoded the assumption
instead of breaking it.

## Acceptance

- Targeted e2e: kill -9 between prune and flip → next apply restores
  the pruned destination; kill -9 mid-rollback → next apply restores
  pre-rollback state; both green. (Kill-point injection via a
  GRIPSACK_CRASH_AFTER=<phase> env hook in the binary — test-only,
  documented as such.)
- All existing unit + e2e pass (assertion-unchanged rule from 0021).
- Docker gates + macOS e2e green; changelog; site edits live.
- Releases: core-v0.22.0 (behavior changes: env profile path,
  rollback protocol, fail-closed capture) — no ts changes, no IR
  change.
