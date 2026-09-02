# 0019 — The deploy journal: crash recovery for pre-flip mutations

## Problem

`apply` mutates real destinations — owned links, tracked copies,
templates, merge blocks — while `current` still points at the old
generation. The flip at the end is atomic, but the deploy phase is
not: a `kill -9`, power loss, or panic between a destination write
and the flip leaves the filesystem between generations with no record
of what to undo. The in-process run-level rollback (0001 §9) handles
failures gripsack sees; it cannot handle gripsack dying.

The external review ranked this the top priority: the core product
promise is "safely let this thing manage my machine," and crash
consistency sits directly underneath it.

## Design

A write-ahead journal in `$GRIPSACK_HOME/journal/`, one JSON entry
per destination mutation, with the same all-or-nothing semantics as
the run-level rollback:

1. **record** (`journal::record`) — before each mutation, the
   destination's prior state is captured: file bytes go to the
   content-addressed prior blob store (`$home/prior/`, shared with
   take-over), symlink targets recorded verbatim, absent recorded as
   `Absent`. The entry is fsync'd (atomic_write) BEFORE the mutation.
2. **after** (`journal::mark_after`) — once the mutation lands, the
   entry gains the post-mutation identity (canonical content hash or
   link target).
3. **commit_run** (`journal::commit_run`) — the flip is the run's
   commit point. After it, every entry is deleted: the generation now
   owns the truth. Per-entry commits do not exist, matching the
   rollback's semantics — a crash at destination 5 of 10 means ALL
   ten recover, because the generation never flipped.
4. **reconcile** (`journal::reconcile`) — at the start of every
   apply, under the lifecycle lock, every uncommitted entry is
   restored to its prior state. Restores are idempotent. The entry is
   consumed either way.

### The drift guard (never delete user edits)

When an entry knows its post-mutation identity (`after`) and the
destination no longer matches it, someone edited the file after the
crash — their edit stands, the entry is dropped with a "kept" line.
When `after` is absent (killed between record and mutation, or
between mutation and the after-mark), the prior restores
unconditionally — the same choice the in-process rollback makes.
`remove_file` never removes directories; a destination that grew
into one after the crash is left alone.

### Integration points

- `deploy.rs`: every mutating write site (owned link swap, tracked
  update, take-over absorb, fresh copy, merge block) routes through
  one helper, `journaled(home, dest, after, mutate)`. Satisfied and
  drift-kept paths never journal — no mutation, no entry.
- `apply.rs`: `reconcile` runs immediately after the lifecycle lock,
  before anything deploys; recovered destinations surface as one
  report line per run plus per-entry run-log warnings.
  `commit_run` runs immediately after the flip.
- `rollback`/`update` do not deploy; they neither journal nor
  reconcile (the next apply picks up any stragglers).

### What is NOT journaled (deliberate scope)

- Store publication (`publish_dir`) — store paths are immutable and
  content-addressed; a partial publish is re-fetched by the next
  apply's satisfaction check failing.
- Generation manifests — written before the flip and named by number;
  an orphaned manifest directory is inert garbage.
- Rollback's own restores — a crash mid-rollback leaves destinations
  restored toward the PREVIOUS generation, which is the direction
  rollback was going anyway; the flip already happened, so the
  filesystem matches the now-current generation for everything
  restored so far. Journaled rollback restores are future work if
  real-world reports ask for them.
- The env profile (`env/profile.sh`) — rendered before the flip;
  a crash mid-render leaves the old profile naming the old
  generation's store paths, which are still valid.

## Testing

- Unit (`journal.rs`): crash-between-record-and-write restores prior;
  crash-after-write restores prior bytes; user-edit-after-crash wins;
  absent/symlink priors recover; commit_run closes the window.
- e2e (`test_apply_lifecycle.py`): a crafted crashed state (exactly
  what a kill between mutation and flip leaves) — the next apply
  restores the prior, reports the recovery, redeploys, drains the
  journal, and ends satisfied; the drift-guard case keeps the user's
  edit. The state is crafted rather than a literal SIGKILL because
  the record→mutate→mark window is microseconds wide — wall-clock
  kills cannot land in it deterministically; the unit tests cover the
  mechanics, the e2e covers the apply-side integration.

## History

Designed and implemented 2026-09-02 (0.18.1), closing the roadmap's
"mutable destinations aren't crash-recoverable yet" item.
