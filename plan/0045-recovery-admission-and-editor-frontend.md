# 0045 — Recovery admission hardening, mutation sessions, and editor-reachable frontends

Response to the 0.39.0 field report (IDE typechecking of the embedded
frontend) and the verification-hardening handoff (2026-09-09, baseline
43c1e43 = this repo's HEAD at adoption time, so no baseline drift).

The handoff proposes a ten-PR programme (baseline reproductions → F1–F4
fixes → Verus pilot → proofs → optional Lean/TLAPS). This plan adopts the
defect fixes and the structural hardening now, records pushback where the
handoff's suggested shape was not the right one, and defers the proof
programme to the roadmap in ROI order. Nothing deferred is silently
dropped; every deferral names its trigger.

## Adopted in this round (ship as 0.40.0)

### F1 — journal identities are tagged on the wire

Defect (confirmed at source): `Entry.after` is a bare string.
`Intended::Removed` → `"gripsack:removed"`, a file → its `FileIdentity`
hex, a link → its target verbatim. The three encodings are not disjoint,
and `decide_from` compares them as strings: a post-crash symlink whose
target is `gripsack:removed` compares equal to a removal intent and gets
"restored" (deleted) although it is a user edit. No hash collision
required — a representation collision.

Change:

- `Entry.after: IntendedSerde` — a tagged enum
  (`{"kind":"removed"}`, `{"kind":"file","identity":<FileIdentity>}`,
  `{"kind":"link","target":<string>}`) next to the already-tagged
  `prior`. `FileIdentity` rides as its transparent serde newtype — no
  naked hash strings. The entry gains a `v` field (1).
- Admission is fail-closed and typed, in one place
  (`Entry::from_wire`): legacy entries (bare-string `after`, no `v`)
  and unknown `v`/kind are quarantined like any malformed entry, with
  the rejection reason (legacy / unsupported version / malformed) in
  the error. Legacy entries are NOT reinterpreted: a 0.39 crash-window
  journal blocks reconcile until the user inspects `journal/quarantine/`.
  No migration heuristic — the handoff's "do not guess the variant"
  stands. Journal entries are transient crash windows, so the
  compat surface is one release's interrupted runs.
- `decide_from` takes typed values
  (`Option<&ObjectIdentity>`, `&Intended`, `Option<&ObjectIdentity>`)
  and returns a pure `RecoveryDecision` (Restore/Keep/Unchanged);
  message rendering moves out of the kernel into the caller.
  Cross-variant equality is now unrepresentable, not unlikely.
- Consistent UTF-8 admission/observation (handoff F1 item 7):
  `capture` already refuses non-UTF-8 link targets; `live_identity`
  now refuses them too (error, object untouched, journal retained)
  instead of lossy-comparing replacement characters; `record` refuses
  a non-UTF-8 destination spelling instead of journaling a lossy one.
- The Rust explorers (`journal/model.rs`, `journal/repeated_model.rs`)
  keep calling the production `decide_from`; their symbolic vocabulary
  maps to typed identities. Cross-variant inequality (file identity
  string == link target string ⇒ still unequal) is pinned by unit
  tests with a real `FileIdentity`, including the handoff's semantic
  witness (post-crash `gripsack:removed` symlink ⇒ Keep).

### F2 — the run marker's `previous_generation` is really required

Defect (confirmed): the field is `Option<u64>` with a comment claiming
"missing is rejected (no serde default)". Serde supplies `None` for a
missing Option field — the torn-marker e2e passed only because its
fixture classifies Ambiguous anyway.

Change: a manual `Deserialize` for `RunMarker` — missing
`previous_generation` is a parse error naming the field; explicit `null`
stays legal (fresh machine); duplicates, wrong types, oversized numbers
stay errors. Parser-level tests distinguish missing / null / value /
duplicate / oversized / wrong-type / malformed-nested. The torn-marker
e2e now proves parser rejection (the entry never reaches classify).

### F3 — GC admits unfinished recovery

Risk (confirmed at source): `gc` derives roots from retained manifests
and never inspects the journal. A crash between record and flip leaves
prior blobs only the journal references; `gc` under the lifecycle lock
still collects them, and recovery then cannot restore.

Change: `journal::pending_recovery` reports unfinished state (run
marker, entries, or a non-empty quarantine; unreadable → error).
`gc` refuses — dry-run included (its deletion set is unsound while
recovery is pending, so a "preview" would lie) — naming what is pending
and prescribing `grip apply`/`rollback` to reconcile first. Conservative
by design: a precise classify-aware policy (a committed run's entries
need no priors) is a deferral, not guessed at. E2E: crash window → gc
refused with nothing deleted → apply reconciles from intact priors.

### F4 — the lifecycle lock is a type, not a convention

Change: `gripsack_exec::LifecycleSession` owns the `FlockGuard` and the
home it locked. `gc` and `rollback_generation` take `&LifecycleSession`
instead of `home: &Path` — a session for home A cannot authorize a
mutation in home B, and library consumers get the CLI's guarantee.
`apply`/`update` acquire the session internally (public signatures
unchanged). `acquire_lifecycle_lock` is removed (clean cutover; the CLI
commands acquire sessions). Store-side `reconcile` keeps its prose
contract — it lives below exec and cannot name the type.

### Frontend ergonomics — the editor can reach the embedded frontend

Field report: the materialized `$GRIPSACK_HOME/frontend/ts-<version>/`
carries a `package.json` whose `main`/`types` point at a `dist/` that
is never materialized, so no tsconfig can resolve it.

- The npm package and the materialized tree resolve TYPES from
  `./src/index.ts` — editors read the same source Deno evaluates —
  while the package keeps a compiled `dist/` as its runtime entry.
  **Finding folded in from CI:** the docs gate proved Deno never
  type-strips under a real (non-symlinked) `node_modules`, so a
  src-only package would break the deliberate pin at eval; `dist/`
  stays for exactly that consumer, and `pin.ts` gained an existence
  fallback — a package whose compiled entry is absent on disk (the
  materialized tree symlinked into `node_modules`; its realpath
  escapes the type-stripping ban) resolves through its `types` entry.
  **Pushback:** the report's first option — embedding compiled
  `dist/` in the *materialized* tree — is still rejected: nothing
  executes it there (the embedded driver imports `src/` directly),
  and the symlink fallback covers the doctor-advised wiring. The
  npm artifact's dist is not the rejected shape; an embedded copy
  would be dead weight.
- Materialization flips `$GRIPSACK_HOME/frontend/current` →

### Guarantee ledger

`verification/guarantees.md` opens with the four entries above (IDs
`RECOVERY-IDENTITY-001`, `MARKER-PARSE-001`, `GC-RECOVERY-001`,
`LOCK-SESSION-001`), each recording the claim, admission boundary,
trusted components, coverage bounds, bridges (unit/e2e/model), and
status `implemented-unverified` — evidence, not aspirations. The
handoff's "limits that stay visible" list is reproduced there.

## Deferred to the roadmap (ROI order)

1. **Verus pilot on the commit classifier** (handoff PR3): `classify` is
   pure, small, and now has typed inputs — the cheapest real proof with
   the highest precedent value (toolchain pin, cargo-verus gate,
   musl compat). Trigger: this plan lands.
2. **Pure GC planner + composition proof** (PR5): needs F3's admission
   (landed here); medium effort, protects the destructive boundary.
3. **Ownership/lineage decision proofs** (PR4): `plan_copy`/`plan_link`
   are already pure and explorer-driven; proof adds universal coverage.
4. **Structural op contracts** (handoff §6.3): private constructors so
   incoherent ops are unrepresentable. Type refactor, no proof needed.
5. **Retry-budget / credential-selection kernels** (PR8): small pure
   transitions, real blast radius (0.39's transport work).
6. **Merge splice kernel proof** (PR7): larger; fuzz + model already
   cover the boundary.
7. **TLAPS inductive safety** for the transaction spec: evaluate before
   any Lean rewrite of a TLA+ protocol.
8. **Lean/Aeneas feasibility study** (PR9): only for a named unbounded
   theorem that survives 1–7. Also deferred: classify-aware precise GC
   admission (needs a shared validated inventory — do it with #2), and
   `frontend/ts-*` directory cleanup in gc.

## Rejected (settled)

- **Shipping compiled `dist/` inside the embedded frontend** — a second,
  unexecuted artifact form; the src-pointing manifest closes the gap.
- **Replacing the Rust explorers / TLA+ suite with proofs of
  abstractions** — the explorers drive shipped decision code; a proof
  that production never calls is negative evidence. (The handoff agrees;
  recorded here so it stays settled.)
- **Guessing legacy journal variants** — fail closed with the evidence
  retained in quarantine.

## Acceptance

- F1 witness regression: post-crash `gripsack:removed` symlink is kept,
  file-digest-vs-link-target spelled identical strings stay unequal;
  legacy/tagged/malformed entries round-trip or quarantine as designed.
- F2: parser tests above; torn-marker e2e fails at parse.
- F3: gc refuses pending recovery in both modes; crash → gc → apply
  recovers from intact priors (e2e).
- F4: `gc`/`rollback_generation` require `&LifecycleSession`; no public
  mutation path takes a bare home.
- Editor: `node_modules/@gripsack/core` symlinked at
  `$GRIPSACK_HOME/frontend/current` typechecks a module repo with the
  doctor-printed tsconfig; embed regen diff is clean; the npm tarball
  guard checks `package/src/index.ts` and `package/dist/src/index.js`.
- Gates: `test`, `ts-test`, `e2e`, `model` compose services green;
  STATUS.md, CHANGELOG, and the website roadmap updated in the same PR.
