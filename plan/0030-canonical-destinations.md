# 0030 — Canonical destinations and composed authorization (0.25.0 fresh-eyes audit)

Status: **implemented in 0.26.0**. Source: the sixth
fresh-eyes review ("a credible transaction protocol and a credible
ownership algebra, but not yet a fully credible composition between
them"). Both P0s verified against main. The theme: stop reasoning in
destination strings; reason in unique, authorized, physical
transitions.

## Verdicts

| # | Finding | Verdict |
|---|---------|---------|
| P0-1 | logical ≠ physical destination uniqueness (`~/x` vs `$HOME/x`, `./`, symlinked ancestors, same-module dups; concurrent workers) | **Adopt** — a canonical destination key (expand `~/`, lexical normalize, canonicalize the deepest existing ancestor) computed once per run; a second non-merge declaration for the same key is a hard error before any mutation, in `check` AND apply; E111's same-module suppression removed; the journal keys on the canonical form |
| P2-2 | precondition derived from a SECOND observation | **Adopt** — one `Observed` snapshot (type, bytes, link target, mode) drives the decision AND the precondition; both identity domains derive from the same bytes. (My own 0029 bug.) |
| H3 | exec-bit split between identity domains | **Adopt, Option A** — tracked copies manage executability: fresh writes apply the source's exec bits, updates apply exec changes, and the journal identity for file deploys is exec-aware. Templates stay bytes-only (documented: a rendered file's mode is not managed). |
| H4 | rename transfers origin but not last-written authority | **Adopt** — lineage lookup is destination-global: prev entry by canonical dest across ALL modules, so a rename keeps full authority (update, not preserve) |
| H5 | takeover semantics: production rebases, models retain | **Adopt the models' semantics** — take-over RETAINS an existing origin (captures only when none). An explicit origin-rebase command goes on the roadmap as a deliberate product feature |
| H6 | merge ownership inconsistent across sema/validator/rollback | **Adopt single-owner for now** — E111 already rejects cross-module merge sharing; the 0029 validator reverts to dest-only keys; rollback's first-wins map never fires on valid input. Cross-module merge aggregation (one whole-file transition per shared file) goes on the roadmap — concurrent read-modify-write of one file is unsound today |
| H7 | merge prune/rollback fail-open reads | **Adopt** — Result<Option>, NotFound-only-is-absent |
| H8 | compute_restore None → silent Noop | **Adopt** — rollback surfaces skips in its output ("unchanged" vs "could not restore" vs "preserved") |
| H9 | lexical-only manifest validation | **Adopt** — `entry.from` must be relative without root/parent components; `store_path` confined by canonical comparison |
| H10 | recovery's current reader weaker than normal | **Adopt** — `flip` writes RELATIVE `generations/N` targets; one shared parser validates both readers (relative canonical form, or legacy absolute-under-home) |
| 11 | legacy markers auto-classified by a proven-unsound rule | **Adopt** — legacy markers refuse with guidance ("inspect journal, delete to accept current state"). The TLA+ legacy cfg and Rust counterexample stay as archaeology |
| 12/13 | end_run/rollback cleanup errors swallowed | **Adopt** — cleanup failure surfaces as "satisfied/active; cleanup pending" in both |
| 15 | broad prefix removal guard | **Adopt** — removal requires the exact expected target |
| 16 | lossy path keys | **Adopt the cheap half** — journal keys hash raw path bytes; non-UTF-8 `$HOME`/destinations refuse loudly at expansion. Full byte-exact OsStr lineage stays on the roadmap |
| 18 | staging dir predictability | **Adopt** — pid-tagged staging names, removal errors propagate |
| 19 | `$` live in env values | **Adopt** — backticks escaped; `$VAR` expansion stays (documented feature); `$(...)` called out in docs |
| 14 | activation hooks outside durable state | **Roadmap** (high) — durable activation-pending + idempotent resume is real work; after the soak |
| 17 | full FsObjectIdentity enum | **Roadmap** — folds into the mode-aware identity item |
| 20 | installer attestation/signed manifest | **Already on the roadmap** |
| — | compare-and-swap displacement (renameat2/renameatx_np) | **Roadmap** — platform-conditional strong semantics after the soak; precondition-at-mutation holds meanwhile |
| — | `grip resolve --keep-live/--apply-repo/--adopt-live`, origin rebase | **Roadmap** — the explicit drift-resolution UX; the reviewer's framing is the spec |
| — | hero / "no takeover" table wording / install order | **Owner's call** (third time) — noted |
| — | safety page: guarantee-levels presentation | **Adopt** — each guarantee marked machine-checked / fault-injection / integration / best-effort |
| — | soak period | **Accepted** — after 0.26.0 the transaction schema freezes for a cycle; the roadmap says so |
| — | TOML frontend | **Rejected, fifth time** |

## Model work (owner ask: violations belong in the harness)

- **Lineage explorer**: apply decomposes into observe → decide →
  mutate with ExternalWrite interleavable between them — the oracle:
  abort-or-preserve, never clobber. Plus aliasing: two logical
  spellings with one canonical key must be rejected pre-mutation.
- **Ownership.tla**: production's take-over now matches the spec
  (retain origin) — no spec change needed; the divergence the review
  found is gone. The alias/interleaving additions land in the Rust
  harness first (they're string/path mechanics, awkward in TLA+).

## Acceptance

The reviewer's regression list as e2e/unit, incl.: `~/x` + `$HOME/x`
collision rejected in check and apply; same-module dup rejected;
symlinked-ancestor collision rejected; single-observation tracked copy
(external write between decision points aborts); fresh 0755 tracked
copy lands executable and stays satisfied; exec-bit update applies;
rename + content change updates (not preserves) and undeclare restores
the origin; double take-over then undeclare restores the FIRST origin;
legacy marker refuses with guidance; `current -> /tmp/42` errors in
recovery too; manifest with `from: "../x"` rejected; two merge modules
one file = E111.

## Release

core-v0.26.0. Manifest/behavior changes are serde-compatible; the
transaction protocol itself does not change shape (preconditions slot
into the existing record/mutate/verify).
