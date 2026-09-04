# 0024 — Review response: the 0.21.0 migration report

Source: the real-world user's 0.18.1→0.21.0 migration report (WSL
laptop + TLS-intercepted RHEL container, 39/41 modules, full upgrade
on both hosts). Every claim verified against the code before acting;
all three new findings were real and are fixed in 0.21.1. The one
sub-item rejected is rejected with evidence, not preference.

## Adopted (fixed in 0.21.1)

### 1. Merge-block scan was first-match-only — FIXED

Verified in `template.rs`: `find_block` returned the first block;
`extract_block`/`marker_sha` rode it; the duplicate-strip lived only
in `upsert_block`'s replace path. Consequences, all reproduced:
duplicates invisible in steady state, the `sha=` content guarantee
covering only the first block, and a drifted first block's repair
deleting every later block with nothing in the report.

The fix is at the level the reviewer pitched: the scan sees every
block (`find_blocks`), and one behavior applies in all cases —
reconcile down to one block and SAY SO:

- `satisfied` now requires exactly one block AND a content match —
  a duplicate is never steady state.
- The apply report names removals: `merged … (removed 1 duplicate
  block)`, composed with the hand-edit note when both happened.
- `remove_block` removes ALL of a module's blocks — upsert
  reconciles duplicates, so prune/rollback leaving extras behind
  would resurrect them on the next deploy.

Regression coverage: e2e `test_duplicate_merge_blocks_are_reconciled_and_reported`
(steady-state dup, tampered-second-block, convergence, user content
preserved) + unit `remove_block_removes_every_block_the_module_owns`.

### 2. doctor's stale-pin line rendered as `ok` and advised an
impossible npm install — FIXED

Two bugs, both verified. The label: `mark(true).replace('✓', "!")`
was a no-op — `mark(true)` returns `"ok  "`, which contains no ✓ —
so the warning printed as a green pass. Now `palette.warn("warn")`;
e2e asserts the line starts with `warn`.

The advice: `npm i -D @gripsack/core@^{embedded}` interpolated the
CORE version while npm's latest was 0.18.0 — the frontend changed in
0.19.0 (steps.ts argument guards) and no ts release was ever cut.
Fixed at the root, not the message: `@gripsack/core` 0.21.0 is
published (ts-v0.21.0), making the existing advice followable. The
check still compares against the embedded frontend — doctor must
work offline (the reviewer's own container), so "query npm for the
latest published" was not an option; lockstep publishing is. Noted
in the release skill: when the embedded frontend source changed since
the last ts release, cut the matching ts tag.

### 3. plan's phantom updates for template/merge — FIXED

Verified: `diff_section` hashed the raw repo source and compared
against the manifest's deployed-form hash (rendered for template,
trimmed block for merge) — never equal by construction. Plan now
computes the source hash in the mode's own terms (template vars are
in the IR; eval already computed them).

One deeper dishonesty surfaced while testing the fix: the comparison
was source-vs-manifest only, so destination drift was invisible to
plan — and merge/template drift is NOT kept, apply regenerates it.
"satisfied" while apply would write is the same lie as a phantom
update, in the opposite direction — exactly the miscalibration pair
the report named. Plan's merge/template entries now also consult the
destination (block count + block hash for merge — the `sha=` marker
makes this manifest-free; rendered hash vs dest for template), so
`(update)` means "apply would write" in both directions. Owned and
tracked-copy keep source-vs-manifest: their drift is deliberately
kept, so plan's answer is already honest for them.

Regression coverage: e2e `test_plan_compares_template_and_merge_in_deployed_terms`
(steady-state satisfied, real drift reports update).

## Rejected (with reasons)

### `store verify` should cover merge destinations

Not adopted. Destination drift is legitimate user content by design —
the drift guard's whole contract is "never delete user edits", and
apply already surfaces it (`drifted — kept`, or regenerates merge
blocks by design). A store-verify that flags kept user edits as
corruption would contradict that contract. `store verify` answers "is
the STORE intact"; "is anything drifting" is `plan`'s job — which
finding 3's fix now makes honest for exactly these modes, including
from the file alone via the marker sha. No doc in this repo claims
store-verify re-hashes deployed destinations (checked README, skills,
site settings reference — all say store paths); the expectation came
from a paraphrase, not the text.

## Amendment (same day): 0.21.2

Smoke-testing the SHIPPED 0.21.1 binary caught a residual in finding
2's fix: the advice interpolated the exact embedded patch
(`^0.21.1`), which cannot resolve while npm's latest is 0.21.0 — the
same ETARGET one patch later. The advice now pins the minor line
(`^M.m.0`), which any published patch satisfies. Verified on the
shipped 0.21.2 binary.

## Carried, not re-examined

- The update→apply sha256/tree256 two-commit dance and the pixi hash
  split: the reporter explicitly did not re-examine them (no fresh
  reproduction; proxy-forced host split). Recorded here so the
  silence is not read as a fix.
