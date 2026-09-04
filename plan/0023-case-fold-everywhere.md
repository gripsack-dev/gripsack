# 0023 — Case-fold destination dedup everywhere (in progress, stashed)

Status: **decision made, ~90% implemented, stashed mid-flight.**
Apply with `git stash pop` (message: "case-fold-everywhere: …") and
finish per the checklist below. Read this doc first — the rationale
is settled, don't re-litigate it.

## Decision

E111 (duplicate destination) case-folds destination strings on
**every host**, unconditionally — not just `ir.host.os == "macos"`
as 0.20.0 shipped. The 0.20.0 conditional is already removed in the
stash.

Rationale (owner-approved):
- **The check protects the repo, not the host.** A repo with
  `~/Foo` and `~/foo` is written on Linux (where they're "fine"),
  then corrupts on a MacBook — gripsack's thesis is one portable
  bag; host-conditional checks defeat that.
- Case-variant destinations are ~always typos; on case-sensitive
  filesystems they silently create two confusing files.
- Precedent: npm bans uppercase package names for this exact class;
  Bazel bans case-variant targets via CI; git's non-normalization
  is the cautionary tale; Nix forces a case-sensitive volume (too
  heavy).

## What the stash contains

- `crates/gripsack-ir/src/sema/destinations.rs`: `fold_case`/
  `key()` closure deleted; owners map inserts
  `entry.to.to_lowercase()`; message says "case-insensitive
  filesystems treat these as one file" (no host suffix).
- The test renamed to `case_variant_destinations_fold_on_every_host`
  (fires on macos AND linux; exact-case duplicates still fire).

## Finish checklist

1. `git stash pop` on a fresh branch off main.
2. `cargo test -p gripsack-ir` — the check body and test were last
   left mid-edit; fix any compile residue (a `fold_case` reference
   may linger in the doc comment block above the map).
3. `cargo fmt && cargo clippy --workspace --all-targets -- -D
   warnings`; full workspace tests.
4. Grep e2e for case-variant destinations — none expected; if any
   test uses them, that test was asserting the old rule: update it
   deliberately.
5. Changelog (0.20.1): "E111 case-folds destinations on every host
   — a repo written on Linux no longer corrupts on macOS
   (case-insensitive filesystems treat variants as one file)."
6. Docker gates, PR (normal flow), merge, tag `core-v0.20.1`.

## Working agreements for the implementing session

- **Breaking changes are allowed** — this is pre-1.0 alpha with no
  users to carry (the owner's standing policy; 0.18.0 removed an
  entire authoring style on this basis). If a backward-incompatible
  change makes the code simpler or the behavior more consistent,
  take it: bump the minor version and write the changelog entry.
- **Code quality bar**: high — readable over clever, modular over
  monolithic, no 800-line files unless the domain genuinely demands
  it, no terse conditions/types that are easy to write and hard to
  read. The codebase's existing style (why-comments at the point of
  surprise, one concern per file/pass) is the floor, not the target.
- Every behavior change lands with its regression test; every fix
  at the root, never the symptom.

## Sequencing for the fresh session

Do this **first** — it's an afternoon and it lands a decision the
owner already made. Then plan/0022 (SBOM, small), then plan/0021
(cap-std, the big one, phases 1–5). All three docs are
self-contained.
