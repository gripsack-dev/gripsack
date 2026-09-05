# 0029 — Ownership lineage and authorized transitions (0.24.0 fresh-eyes audit)

Status: **implemented** (0.25.0). Source: the fifth
fresh-eyes review ("a credible crash-consistency protocol, but not yet
a correct ownership-lineage model"). Verified against main (0.24.0 +
the model commit); the two P0s are real and deterministic, and one
shares a root cause with a bug the review did NOT list (below). This
is the state-representation round: make incorrect ownership semantics
difficult to express.

## The root cause the review named correctly

One field — the manifest entry's `hash` — currently means four things:
desired identity, last-written identity, observed-at-commit identity,
and the authority to overwrite later. And `prior` is attached to a
single deployment result instead of the ownership lifetime.

## Verdicts

| # | Finding | Verdict |
|---|---------|---------|
| P0-1 | origin `prior` dropped by the next ordinary apply; GC can then free the blob | **Adopt** — prior is carried forward per DESTINATION across every generation (including module renames); an epoch ends only on successful restore/forget. GC already pins via retained manifests — carrying forward makes that sufficient |
| P0-2 | preserved drift recorded as deployed hash → next apply overwrites | **Adopt** — `DeployedEntry` gains `preserved_drift` (serde default false); a preserved entry never authorizes an update. Same fix covers "foreign file promoted by observation" |
| (unlisted) | **prune DELETES a drift-kept file on undeclare** (recorded observed hash makes the intact check pass) | **Adopt** — same fix: preserved-drift entries are skipped by prune and rollback (gripsack never wrote them) |
| P0-3 | precondition not authorized at the mutation boundary | **Adopt with stated limits** — `journaled` takes `expected_before`; capture verifies it and aborts on mismatch (drift appeared between decision and mutation). There is no portable content-CAS (renameat2 RENAME_EXCHANGE is Linux-only; macOS has none): the residual capture→rename window is documented on the safety page. Merge upserts recompute from the latest read inside the mutation |
| 4 | recovery cleans entries after a silently failed restore | **Adopt** — recovery verifies post-restore identity before dropping the entry; `let _ = remove_file` is gone from recovery |
| 5 | dangling/foreign symlinks misclassified by `exists()` | **Adopt** — copy/template branch on `symlink_metadata` object type; a foreign symlink refuses without --take-over |
| 6 | merge prune/rollback read with `unwrap_or_default` | **Adopt** — `io::Result<Option>`, NotFound-only-is-absent |
| 7 | manifest validation rejects what merge allows | **Adopt** — validation key: dest for non-merge, (dest, module) for merge; generation CONSTRUCTION validates too (publish runs the same validator) |
| 8 | content-addressed paths trusted by name | **Adopt** — prior blobs recompute on reuse; rollback preflight verifies `tree256` against the actual tree |
| 9 | publish window: rename before high-water | **Adopt** — high-water durably written before the rename |
| 10 | `current` read by basename | **Adopt** — target must canonicalize under `$GRIPSACK_HOME/generations/<N>` with N == parsed id |
| 11 | owned-link intactness is prefix-broad | **Adopt** — intact ⟺ target == that entry's exact store source |
| 12 | outside-home destinations undocumented | **Push back** — outside-home deploys are real, in-use behavior (the migration report's `bpkg` at /usr/local/bin) and same-privilege by design. The safety page documents the actual boundary instead. An `allow_outside_home` setting with louder plans goes on the roadmap |
| 13 | post-commit cleanup failure reads as failed apply | **Adopt** — commit-phase errors report "generation N active; cleanup/activation pending"; reconcile already resumes it |
| 14 | "restores before returning" too absolute | **Adopt** — safety page: attempted-immediately + durable-journal blocks |
| — | ownership-lineage formal model | **Adopt** — extends the Rust harness (new lineage model); TLA+ ownership spec if it stays small |
| — | TOML/JSONC frontend | **Rejected, fourth time** (0020/0025/0027 stand) |
| — | reproducible-build verification, SPDX sidecar, external audit | **Roadmap** |
| — | soak period | **Accepted as process** — after 0.25.0, no transaction-schema churn next cycle; the reviewer's "slow down" is noted on the roadmap |

## The state change

```rust
pub struct DeployedEntry {
    // ...from/to/mode/vars unchanged
    pub hash: String,                  // what gripsack last WROTE (or observed, when preserved)
    pub prior: Option<Prior>,          // the origin — now carried for the epoch
    #[serde(default)]
    pub preserved_drift: bool,         // observed user bytes, never authority
}
```

Rules that become structural:

- `preserved_drift` entries: apply re-evaluates drift fresh (live ==
  desired → converged/managed; live == observed → still drifted, keep;
  else → new drift, keep); prune and rollback never touch them.
- `prior` propagates by destination from the previous manifest into
  any entry that didn't just capture a new one.
- The overwrite authorization for copy/template is: live ==
  last-written AND the entry is not preserved-drift.

## Acceptance

The reviewer's regression list as e2e/unit, incl.: origin restored
after satisfied-apply → undeclare; origin restored after update →
update → undeclare; origin survives gc of old generations → undeclare;
drift preserved across two applies without hand-resolution; foreign
file never promoted across two applies; undeclare of a drift-kept
module leaves the file; take-over precondition abort when the dest
changes between decision and capture; foreign symlink at a copy dest
refuses; dual merge owners in one file validate and deploy; rollback
refuses a tree256-mismatched store; `current → /tmp/42` errors.
Lineage model in the harness with its own oracle.

## Release

core-v0.25.0. Manifest schema additions are serde-defaulted (old
manifests read fine). No IR change, no ts release.
