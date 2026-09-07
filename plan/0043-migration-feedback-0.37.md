# 0043 — Migration feedback 0.21→0.36: modes as identity, honest doctor, update --check

Status: **landed in core/TS 0.38.0**. Feedback source: the 0.21.0→0.36.0
migration report (laptop-only, WSL Ubuntu 24.04 / glibc 2.39). Four
findings; the mode finding is systemic and gets the model treatment the
owner asked for.

## The findings

1. **`doctor` reported `ok` on a frontend that cannot evaluate.** With
   `node_modules/@gripsack/core@0.17.9` installed and `package.json`
   pinning `^0.36.0`, doctor compared the *declared* pin against the
   embedded frontend and printed ok — but eval loads the *installed*
   copy (the deliberate-pin rule), and `grip check` failed E100
   immediately. Doctor must inspect what eval actually resolves.
2. **`template()` drops the executable bit** (fresh templates land
   0644 via `WritePermissions::Preserve`; `symlink()` keeps the payload
   mode). Worked around with a chmod hook. This is the fifth landing-mode
   incident (0025 §G, 0026 §7, 0030 §H3, 0031 §3, 0041's template
   rollback) — the class needs a systematic answer, not another patch.
3. **`grip update` has no dry-run.** `plan`/`check` are side-effect-free;
   update rewrites the lock to show drift. `--check` fits the vocabulary.
4. **The bundled tuicr linter pack is stale** (`supported = ["0.2"]`
   covers 0.2.x numerically, never 0.2x) and its forge/export tables are
   flat where the tool nests them — every check/apply printed an
   unactionable W10.

Not a gripsack bug (recorded for context): the reporter's artifactory npm
mirror stops at 0.17.5, so the "update the pin" remedy was unavailable —
removing the pin was correct there. Doctor's advice should keep naming
both exits.

## F2 — Modes as identity, across every surface

Research inventory (surface-by-surface, with file:line provenance):

| Surface | Mode today (0.36) | Gap |
|---|---|---|
| trackedCopy | `WritePermissions::Source`: 0755/0644 fresh, exec-delta on update; mode-aware identity | none (0041 fixed) |
| **template** | `Preserve`: fresh = fixed 0644, source exec ignored | **exec dropped; identity bytes-only — chmod invisible to satisfy/prune** |
| merge (existing file) | preserves observed file mode via atomic_write | entry records no file_mode |
| merge (new file) | fixed 0644, decided in the *executor* | decision buried; not planner-owned |
| symlink | mode rides payload | none |
| rollback/prune/priors | Exact(recorded); trackedCopy guard mode-aware | **template intact-guard bytes-only — chmodded template prunes as intact** |

### The change

- **Templates follow payload executability.** `WritePermissions::Source`
  is shared with tracked copies in deploy and read-only preview. Fresh
  whole-file outputs normalize to 0755 if executable, otherwise 0644;
  a private 0600 source is not executable. Takeover retains the destination's
  full mode; source execute deltas never widen acquired read/write access.
- **Template identity becomes mode-aware** (`FileIdentity`, not
  `BytesHash`) in the planner's desired/live compare, the manifest
  entry, prune's intact guard, and store-verify's template arm. A
  chmod-only change on a template is now drift: observed, preserved and
  warned on re-apply (the 0.27 contract), visible in plan, and it guards
  prune exactly like content drift.
  Historical template receipts remain readable: their bytes hash plus
  separately recorded mode must both match before authorizing an update or
  prune. An unknown mode grants no removal authority. New receipts always
  use full file identity, including after rollback.
- **Marker metadata for merge blocks.** The open marker gains the
  hosting file's mode: `>>> gripsack module=M sha=<16hex> mode=<0oct> >>>`.
  Parsing tolerates the old spelling (mode unknown → re-emit on next
  deploy). The manifest's merge entries record `file_mode` (the observed
  hosting-file mode at deploy). Chmod drift is preserved and warned, including
  repeated reapply; prune checks block identity and hosting mode. Store-verify
  reports active destination chmod separately from artifact corruption:
  `--repair` never deletes a healthy store payload because its output drifted.
- **Merge new-file mode moves to the planner**: a `WritePermissions`
  decision (deterministic 0644 — 0031 §3's rationale, now made in the
  same place every other mode decision lives) instead of an executor
  literal.
- **Self-description for template outputs**: no in-file marker (managed
  template files are whole-file outputs; injecting a banner would change
  rendered configs — the identity above already covers content+mode).
  Documented here as the deliberate asymmetry: merge = marker-tracked,
  template = identity-tracked.

### The model (the owner's ask: "stop this from surfacing again")

- `specs/FileMode.tla` — a small spec of the mode lattice over
  {copy, template, merge, link} × {0644, 0755, 0600} with the deploy /
  chmod / redeploy / prune / rollback transitions. Transition snapshots keep
  deploy/record/rollback claims scoped to the action that establishes them;
  user chmod is not incorrectly asserted to preserve the old mode.
  Twelve positive lanes and four calibrated negatives run in the model gate:
  fixed-0644 template deployment violates `ExecSurvivesDeploy`, bytes-only
  satisfaction violates `ChmodIsDrift`, unsafe prune violates
  `PruneRespectsDrift`, default-mode rollback violates `RollbackIsExact`.
- `gripsack-exec/src/mode_model.rs` drives the **shipped** planner and executor:
  fresh and private acquired modes, content updates, source-execute deltas,
  exact rollback, repeated drift and guarded prune. Merge and link lanes
  verify their different permission policies on real files.
- e2e: an executable template payload survives apply and re-apply;
  chmod-only drift on a template is preserved and warned; a chmodded
  template is not pruned as intact; merge mode rides the marker.

### Why the earlier models missed templates

`Ownership.tla` uses opaque content values (`C0`–`C3`) and never models a
permission mode. `Transaction.tla` proves journal recovery relative to an
already-selected intended identity; it cannot reject the wrong policy that
produced that identity. `ops/model.rs::check_one` only instantiated tracked
copies and `WritePermissions::Exact(0644)`, bypassing template source-policy
selection. A correct abstract protocol is not proof that every caller supplies
the correct identity domain.

The in-flight FileMode files were not yet wired into `check_models.sh`; one
negative also omitted a required constant. Its initial invariants treated 0600
as executable, asserted record equality after arbitrary user chmod, and tested
rollback membership in the mode set rather than exact restoration. Those
predicates are replaced, not counted as evidence. The model's header now states
its scope and exclusions. Executable-template e2e covers the source-selection
boundary the planner explorer alone cannot exercise.

## F1 — Doctor inspects what eval loads

`doctor` resolves the frontend the way eval does: if
`node_modules/@gripsack/core` exists, its `package.json` version is the
pin that answers. A stale *installed* copy is a MISS (check/apply will
fail E100 on it) with the same remedies E100 names; the declared-spec
warning remains for repos with no installed copy. e2e: the migration
report's exact shape — 0.17.9 installed, `^0.36.0` declared — must
report MISS, not ok.

## F3 — `grip update --check`

Resolves and acquires into private scratch, reports every would-be pin change,
and performs no lockfile write or source-cache publication. Exit 0 when current;
exit 1 when a pin would move. Comparison includes complete resolved metadata
and the fetch declaration. Normal update's publication behavior is unchanged.
Eval/runtime provisioning and run logs retain their normal command behavior;
this is a read-only source update, not a promise of zero host bookkeeping.
`plan`'s vocabulary is unchanged.

## F4 — tuicr pack refresh

Per official research (github.com/agavra/tuicr, config surface verified
per-tag from src/config/mod.rs and docs/CONFIG.md): the key set is
additive-only across 0.20→0.25 (0.22 adds show_pr_checks,
show_pr_comments, search_highlight, diff_watch_interval_ms; 0.23 adds
show_reviewed; 0.24/0.25 unchanged). `supported` becomes the explicit
minor list ["0.20"–"0.25"] (numerical matching — "0.2" never covered
0.2x); forge/export keys move under their real sections; the deprecated
`export_legend` keeps a superseded note; the coverage warning names the
real range. A v0.22.0 fixture must lint clean; a v0.19 pin still warns.

## Boundaries

No IR or DSL change (modes come from payload stat; the frontend is
unaffected). No new settings. Fetch acquisition modes are untouched —
the inventory found them honest. Non-unix stays on documented defaults.

## Delivery

Compose gates green; twelve FileMode positives and four calibrated negatives
in `scripts/check_models.sh`; the Rust explorer in `cargo test`; offline e2e
for all four findings; a real CLI smoke transcript for update/check, executable
template, chmod/apply/prune, and stale-installed doctor. Website mode/settings,
CLI, roadmap and changelog docs updated; STATUS row; core+TS 0.38.0 in lockstep.

