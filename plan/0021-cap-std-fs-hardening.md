# 0021 — Capability filesystem access (cap-std/openat migration)

Status: **planned, not started** — handover document. Read plan/0020
§"cap-std/openat, the decision restated" for why this was deferred
and its trigger. This doc is the implementation plan for when it
fires (one real TOCTOU report, or the approach of 1.0).

## Problem

gripsack's filesystem code navigates by path strings:
`canonicalize(dest)` → validate → act on the string later. Between
check and use, a swapped parent symlink changes what the path
resolves to (TOCTOU). No bug of this class has occurred in practice
(~30 fixed bugs across three fuzzing/review rounds were crash-
consistency, validation, panics, hangs — zero fd-races), which is
why this is queued, not done.

## Target design

Two crates, layered:

- **rustix** — safe bindings for `openat(2)`, `O_NOFOLLOW`,
  `fstatat`, fd-relative reads/writes. The mechanics.
- **cap-std** — `Dir` capabilities on top: APIs accept *relative*
  names only, so code cannot escape the directory it was handed.

Roots gripsack would hold (open once per process, pass explicitly —
no globals):

| Capability | Opens | Scopes |
|---|---|---|
| `home: Dir` | `$GRIPSACK_HOME` | store, generations, journal, prior, locks, runs |
| `repo: Dir` | the env repo | module sources, configs, locks/<host>.lock |
| `dest_parent: Dir` | per-deploy, opened at check time | the one destination being written |

The invariant this buys: `dest_resolves_into` (deploy.rs) and the
subsequent write are pinned to ONE parent-dir inode — the check and
the use cannot observe different filesystems.

> **Standing policy from the owner**: backward-incompatible
> changes are fine (pre-1.0, no users) whenever they serve
> readability or simplicity — bump the minor, write the changelog.
> The quality bar is high: modular, readable, why-comments where
> the code surprises. The acceptance criteria below are the floor.

## Migration order (each phase ships green independently)

1. **journal.rs + prior blobs** (smallest, newest, best-tested —
   34 unit tests already pin its behavior). `capture`, `record`,
   `restore` take `&Dir` for home; entry paths become relative.
2. **deploy.rs single-file writes** — `atomic_write` gains a
   `Dir`-relative sibling (`atomic_write_in(&Dir, name, bytes)`).
   The dest-parent Dir is opened where `dest_resolves_into` runs
   today; the drift-guard hash and the write both use it.
3. **fs.rs core** — `atomic_write`, `symlink_replace`,
   `publish_dir` (temp sibling + rename already correct; make the
   handles relative). Keep the string-path versions as thin
   wrappers over an ambient Dir during migration, delete them after.
4. **store paths** — `store_path`/`content_path` return relative
   names + the home Dir; generations.rs joins under the capability.
5. **Delete the wrappers** — the string API is gone; the compiler
   finds every survivor.

## What NOT to change

- The journal protocol, marker grammar, hashes — this is a
  mechanics swap, zero semantic change. All existing unit + e2e
  tests must pass UNMODIFIED (that's the acceptance bar; any test
  edit is a redesign smell).
- The frontend, IR, lockfile — untouched.

## Known pitfalls

- **tempfile interplay**: `tempfile::NamedTempFile::new_in` takes a
  path; cap-std's `Dir::create` + manual `O_EXCL` replaces it in
  `atomic_write`. The fsync-parent contract must survive.
- **Non-UTF-8**: rustix takes `OsStr` natively — this migration is
  also the moment to carry bytes end-to-end (the journal currently
  rejects non-UTF-8 symlink targets; with `OsStr` it can preserve
  them — keep the rejection initially, relax in a follow-up with a
  test).
- **EXDEV publish**: the temp-sibling rename stays same-filesystem
  by construction under a Dir — simpler than today's string dance.
- **Scope creep**: resist rewriting `why-owns`/`doctor` read paths
  in phase 1–4; reads through `std::fs` are not the risk surface.

## Acceptance

- `cargo test --workspace`, docker gates (test/ts-test/e2e), and the
  macOS job all green with zero test edits.
- New unit test: an adversary test that swaps a parent symlink
  between check and write (spawn a thread flipping the link in a
  loop) fails WITHOUT the migration and passes WITH it.
- plan/0020's queued item checked off; changelog entry.

## Estimate

Phase 1–2: one focused session. Phase 3–5: a second. Do not land
phases out of order — the wrappers exist so each phase is shippable.
