# 0031 - Mode-aware identity (the full FsObjectIdentity)

Status: **implemented in 0.27.0**. Completes 0026 §7's remainder and
0030 #17. Owner override: the post-0.26.0 soak is waived for this
item — land what is right while the project is alpha.

## Scope (as accepted in the audits)

Mode bits join the manifest/journal identity: **chmod-only drift
detection** and **exact mode restoration on rollback**. Mode
*management* (declaring modes in the repo) is NOT in scope — a
tracked copy's mode stays the 0644|exec rule derived from the
payload; what changes is that the identity gripsack records, compares,
and restores is the full permission set, not just the exec bit.

## Design decisions

1. **One preimage, extended compatibly.** `canonical_bytes_identity`
   takes the full mode. Modes 0.26 could ever have recorded — 0644
   and 0755, the only values its own writes produce — keep the exact
   0.26 preimage bytes, so an upgraded home's first apply reads its
   manifests as satisfied instead of preserve-limbo. Every other mode
   (a hand-chmodded 0600, a 0777) hashes under a mode-extended
   preimage: drift is detected, never silently absorbed.

2. **The manifest records the landed mode.** `DeployedEntry` gains
   `file_mode: Option<u32>` (serde-default: pre-0.27 manifests parse;
   None means "not recorded", restored by the legacy rule). Rollback
   re-applies it after the bytes land — exact restoration, matching
   what the crash journal's priors have done since 0.24.

3. **Fresh writes land deterministic modes.** `atomic_write` on a new
   file landed `0666 & ~umask` — unpredictable under umask 077 and
   impossible to predict in the journaled precondition. Fresh
   templates and merge-created files now land 0644 absolutely
   (tracked copies already did, via `atomic_write_with_mode`).
   Behavior change only for umask != 022 fresh creates.

4. **Merge keeps its distance.** A merge-managed block lives in a
   foreign file; the whole file's mode is not ours to record or
   restore. Merge entries carry no `file_mode`.

5. **Templates detect content drift only.** A rendered file's mode is
   not managed (0030 §H3): the manifest identity for templates stays
   bytes-only, but the *landed* mode is still recorded and restored
   exactly. The journal/precondition domain is mode-aware for every
   file, templates included — a chmod between decision and mutation
   aborts the run.

## The model dimension

The lineage explorer gains `ExternalChmod`: the user changes a
managed file's mode without touching its bytes. The oracle, driven
through the shipped `plan_copy`: chmod-only drift is *preserved*
(never silently reverted, never overwrite authority), exactly like
content drift. The TLA+ note: `specs/Ownership.tla` models content
values; the mode dimension adds nothing the content domain doesn't
already exercise (drift is inequality), so it stays out — recorded
here so the question doesn't reopen.

## Acceptance

- e2e: a chmod-only change to a tracked copy is preserved-and-warned
  on the next apply; a repo-side exec change still applies; rollback
  restores the recorded mode exactly.
- An upgraded 0.26 home's first 0.27 apply over 0644/0755 files is
  satisfied (no spurious drift, no new generation).
- Harness: `ExternalChmod` explored to depth 6, zero violations.
