# 0008 — Store canonicalization and satisfaction

- Status: draft
- Date: 2026-08-22
- Amends: 0001 §3.4 (store), 0003 §5 (e2e contract)

## 1. The two questions

1. Is the store permission-aware — should `chmod +x` on a payload file
   trigger a reinstall?
2. When `grip apply` runs and nothing changed, what runs? (Nothing.)

## 2. Canonical content identity

Adopted wholesale from nix's NAR model (and git's tree objects):

```
canonical(file) = type + exec-bit + (contents | symlink-target)
```

- **Included**: file type (regular/dir/symlink), the executable bit for
  regular files, symlink targets, file contents.
- **Normalized away**: mode bits beyond exec, mtimes, ownership.

Consequences:

- `chmod +x` a payload file → new canonical hash → new store path →
  redeploy. A permission change IS a content change.
- Fresh clones, umask variance, editor mode noise, mtime bumps → same
  identity. Lockfiles and store paths travel between machines intact.
- Drift detection (`tracked-copy`) uses the same hash, one rule in both
  directions: chmod the deployed file → drift reported; chmod the repo
  file → redeploy.

What we skip from nix: a physically read-only store. Nix enforces it
with root + daemon; we're user-scoped, so read-only bits would be
theater. Immutability is by convention plus regeneration (`grip gc` +
apply rebuilds a tampered path).

Implemented in `gripsack-store`: `canonical_file_hash`,
`canonical_tree_hash`.

## 3. Satisfaction — what runs on a no-op apply

Four layers, each independently cheap:

1. **Eval — always runs, always cheap.** Milliseconds. Never cached:
   correctness comes from content addressing downstream, not from mtime
   heuristics (make-style timestamp checks are the "why didn't it
   rebuild" bug factory).
2. **Resolution — lockfile short-circuits.** Unpinned refs matching the
   lockfile resolve with zero network. Re-resolution only on
   `grip update` or spec change.
3. **Fetch+build — store path existence IS the satisfaction check.**
   The store path is the hash of the module's resolved inputs; presence
   is proof. No up-to-date logic to get wrong. (`grip store verify`
   re-hashes for integrity paranoia — a slow path, not the gate.)
4. **Deploy — manifest diff.** Every generation records a manifest:
   each deployed path with mode, store target, canonical hash. Apply
   computes the desired deployment, diffs against the *current
   generation's manifest* (not the live filesystem — user drift is
   handled separately per ownership mode), and touches only what
   changed. Unchanged files keep their mtimes.

**If desired state == current generation, apply is a no-op and creates
NO new generation** — it reports `already satisfied (generation N)`.
Generations represent state transitions; empty ones are noise.

Second consecutive `grip apply`: eval → lockfile match → store paths
exist → empty deploy diff → done, well under a second.

## 5. Updates

The flake cycle, verbatim:

- **`grip update [MODULE...]`** re-runs resolution and rewrites
  `locks/<host>.lock` — nothing deploys. `grip plan` then shows the
  diff; `grip apply` executes it. Per-module update is free because
  lockfile entries and resolution are both per-module.
- **apply never re-resolves.** It verifies fetched payloads against the
  locked hash; a mismatch is the E2xx tampering signal, never a silent
  re-pin.
- **First apply pins** (trust-on-first-use, 0002 §3): the payload hash
  is recorded on first fetch. The hash participates in store-path
  identity from the *first* apply — resolved before the existence
  check, so a lockfile write never changes a module's path.
- Re-resolution is a core operation (0002 §7): built-in fetchers
  resolve in the engine at lock/update time, keeping API traffic inside
  the throttle domains (0007 §4c).

## 6. The custom_shell contract

Opaque actions can't be satisfied automatically. Rule: a `custom_shell`
step MUST declare `outputs = [...]`; satisfaction = declared outputs
exist under the step's input hash (which includes the script text).
Undeclared outputs → always runs, flagged cache-busting in `plan`.
Verify steps run only when their step ran — a no-op apply runs zero
verifies.
