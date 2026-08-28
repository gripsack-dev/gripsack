# 0014 — Content-addressed fetches, input-addressed builds

- Status: draft
- Date: 2026-08-28
- Amends: 0001 §3.4 (store addressing), 0008 (identity), 0013 (nothing —
  orthogonal layer)

## 1. The question a store path answers

Today every store path is input-addressed (0001 §3.4):
`<input-hash>-<name>` covers the *resolved module plan* — fetch spec,
pinned refs, build recipe, install/config mappings, dependency hashes.
The path answers "what did we **do** to produce this."

That is the wrong answer for the common case. The overwhelming majority
of real modules are fetch-only or config-only: their content is fully
determined before execution. For those, the path should answer "what
**bytes** are these" — because:

- **A self-verifying store.** If the path name IS the content hash,
  corruption detection needs no lockfile consultation: re-hash the tree,
  compare to the name. `store verify` (0.16.1) currently proves the
  machinery; content addressing makes the expectation intrinsic.
- **A content-keyed fetch cache, nearly free.** Presence of the
  content path in the store *is* the cache hit — no network, even when
  the recipe changed (mirror swap, URL edit, version rename) but the
  bytes didn't.

## 1a. Erratum: two hashes, not one

Design review against the code surfaced two facts that shape this doc:

1. **The lockfile pin is the TRANSPORT hash** — sha256 of the raw
   download bytes (`archive::sha256`), not the canonical tree hash of
   the staged payload. The tree hash only exists implicitly, computed
   over staging at fetch and never recorded.
2. **`grip store verify` is latently broken for fetched modules.** It
   compares `canonical_tree_hash(store_path)` — the merged tree
   (payload + repo config files) — against that transport hash, a
   comparison that can never match for archives. It escapes
   false-positives only because its lock lookup is keyed on the
   ambient hostname with no `--host` flag, so it usually finds no lock
   and silently skips the check.

So 0014 introduces the split explicitly: **`sha256` = transport
integrity** (verified at download, as today) and **`tree256` = store
identity** (canonical tree hash of the published store tree — payload
plus merged repo files — recorded in the lock's `resolved` and in the
generation manifest). Content addressing keys on `tree256`. Store
verify becomes host-independent: the manifest carries `tree256`, so no
lockfile lookup is needed at all.

## 2. Why builds stay input-addressed

Input-addressing does not require build reproducibility — gripsack
builds a recipe once and satisfaction means never rebuilding it unless
inputs change. What it genuinely buys is **plan-time path
computability**: dependents reference each other's store paths (an
ephemeral toolchain's path feeds the dependent's build env), so every
path in the DAG must be nameable before anything executes. That is what
makes `grip plan` a complete diff and no-op applies free.

A built artifact's content hash is unknowable until the build finishes.
Content-addressing builds would defer path naming past planning and
break the "plan shows everything before anything moves" invariant —
Nix's ca-derivations needed years of deferred-resolution machinery for
exactly this. Rejected.

Fetches face no such problem: the hash **precedes** the bytes (lockfile
pin, or offline-computable for `file`/repo payloads). Content-addressed
fetches keep plan-time naming AND gain self-verification. The hybrid is
the same split Nix makes (fixed-output vs derivations), adopted for the
opposite reason: Nix input-addresses because its builds are pure; we do
because ours aren't.

## 3. The rule

Per module, decided statically at plan time:

- **Content-addressed** when content is fully determined before
  execution:
  - fetch-only modules (fetch + install, no build/custom steps) — the
    key is `tree256`: the locked value when present, otherwise
    deferred until first fetch (transport hashes cannot name an
    unextracted tree — deferred identity, 0002 §3 TOFU, is the honest
    flow);
  - config-only modules — the key is the canonical tree hash of the
    config payload sources, computed by the core at plan time
    (`hash.rs` already canonicalizes: type + exec-bit + contents;
    mtimes and mode noise are not identity).
- **Input-addressed** when any build or custom step exists
  (`custom_shell`, `run`, builder steps). Unchanged from today: recipe
  hash, dependency *content* hashes as ingredients (they already flow
  in via the `|payload=` projection).

The content key covers the **staged payload tree only**. Install
mappings, config destinations, ownership modes, env contributions, and
activation intents are deploy concerns: editing them must NOT refetch
or change the payload's path. (Today they ride the input hash, so
editing an install mapping needlessly re-fetches; content addressing
fixes that as a side effect.)

First fetch of an unpinned module keeps the existing deferred-identity
flow: the path finalizes at publish, when the merged staging's tree
hash exists, and lands in the lock as `resolved.tree256`
(trust-on-first-use, 0002 §3 — invariant 6 untouched). The path keeps
the `-<name>` suffix: dedup targets the same module across recipe
changes (the mirror swap), not cross-module content collisions —
cross-name dedup is a non-goal, and the name keeps `ls store/`
legible.

## 4. Consequences

- **Satisfaction** is unchanged in shape: `path.exists()` means done.
  But "exists" now also means "provably the right bytes" for
  content-addressed paths.
- **`grip store verify`** becomes host-independent and correct: the
  generation manifest records `tree256` per content-addressed module,
  so verification compares the tree hash against the manifest — no
  ambient-hostname lock lookup (the latent bug in §1a dies here).
  Per-entry manifest hash checks continue to cover every module kind.
- **Dedup across recipes.** Same module, new URL, identical bytes:
  same path, no refetch, no new copy. (Cross-module-name dedup:
  non-goal, §3.)
- **`grip plan`** can distinguish "content already in store" from
  "will fetch" precisely, per module, before any network.
- **GC and generations** are untouched: generations reference store
  paths regardless of addressing regime.
- **Store sharing/sync (future)** may replicate content-addressed paths
  with verification-on-arrival. Input-addressed (built) paths are
  provenance-named, not content-guaranteed: they must NOT participate
  in content-verified sharing. Docs must say so.
- **Migration**: content-addressed paths differ in shape from the
  input-addressed paths existing stores hold, so the first apply after
  upgrade re-stages fetch/config modules once (identical bytes, new
  names) and `gc` collects the old ones. One-time cost; alpha
  tolerance. Folds into the pending 0.17.0 release — one breaking
  release, not two.

## 5. Non-goals

- Content-addressing builds (§2).
- A remote cache/sync protocol (the addressing enables it; the protocol
  is its own doc).
- Changing the canonical identity rules (0008 §2 stands verbatim) or
  the path shape (`<hash16>-<name>`, same `HASH_LEN`).

## 6. Acceptance

- A fetched module whose URL changes but whose payload bytes don't
  keeps its store path and does not refetch (e2e mirror-swap).
- Editing an install mapping or config destination does not refetch
  (e2e: satisfied no-op, same path).
- `grip store verify` flags a flipped byte in a FETCHED module's store
  path (e2e — today uncovered; §1a's latent bug dies with coverage).
- Verify no longer depends on the ambient hostname's lockfile.
- Built modules keep input-addressed paths and plan-time naming (e2e:
  the class-style/patched fixture).
- `grip plan` output distinguishes cached vs to-fetch (unit or e2e).
