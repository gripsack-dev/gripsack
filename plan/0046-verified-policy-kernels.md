# 0046 — Verified decision kernels: the Verus pilot and its gates

Adopts the hardening handoff's PR3 (small production Verus pilot),
PR4 (ownership/lineage proofs) and PR5 (GC planning + composition
proof), plus §6.3's structural op contracts, and its proof-runner/CI
requirements (§10). The pilot is deliberately smaller than the
highest-value theorem; PR4/PR5 then land because their kernels were
already pure and explorer-driven.

- **`crates/gripsack-policy`** — the new dependency-light crate for
  verified decision kernels (handoff §6.1). No filesystem, network,
  subprocess, tracing or async dependencies; effects and rendering
  stay in the calling crates. Its first resident is the commit
  classifier (`classify`, `RecoveryFacts`, `Classification`), moved
  verbatim from `journal/marker.rs`, which re-exports it — production
  reconcile, both Rust explorers, and the verifier now share ONE
  implementation (the handoff's forbidden split-brain patterns are
  structurally impossible).
- **The contract is the commit rule, not the branches** (handoff §11's
  spec-review warning): three biconditionals over the facts —
  Committed ⟺ current == target; Uncommitted ⟺ fresh machine with no
  current, or current == previous ≠ target (target precedence stated,
  never assumed); Ambiguous ⟺ neither. The absence of inequalities is
  the machine-checked form of "numeric ordering and direction labels
  do not decide commitment".
- **`cargo verus verify -p gripsack-policy --locked`** via
  `scripts/check_verus.sh` — positive proof plus seeded-mutant
  calibration: ambiguity-misread-as-commitment (the 0.22 bug class)
  must fail naming its postcondition; a crash, parse error or missing
  solver fails the harness instead. Zero/subset verification fails the
  gate (obligation floor).
- **Toolchain pins** (handoff §10.2): Verus release
  `0.2026.09.06.8dea4a2` (sha256-pinned zip), its matched Rust 1.98.0
  — already the repo's pinned toolchain, no downgrade — and Z3 4.16.0
  (sha256-pinned). `vstd` is the crates.io build published from the
  same commit (`=0.0.0-2026-09-06-0133`), so `--locked` and the musl
  release path are untouched: plain cargo compiles the annotated
  source with specifications erased.
- **CI**: a `verify` compose service (glibc/amd64 only — Verus ships
  no musl or aarch64-linux prebuilt) called from the existing required
  `test` job; proofs cannot fail in an optional check.
- **Ownership kernels (PR4)**: `plan_copy`/`plan_link` move to
  `gripsack-policy::ownership` with the authority table as
  biconditionals — preserved drift never authorizes, updates require
  agreement with the last managed write, take-over always absorbs.
  Production planning and the lineage explorer call the proved code;
  specs speak in `Seq<char>` views while exec `==` is unchanged.
- **GC retention kernels (PR5)**: `admit_gc` / `plan_prune` /
  `plan_delete` + `lemma_delete_monotone` in
  `gripsack-policy::retention` — admission failure yields no
  destructive plan, pruning never names the current generation, the
  deletion set is extensionally candidates-minus-roots, and growing
  roots cannot grow deletions. `gc()` consumes the plans under its
  `LifecycleSession`; roots/inventory arrive as validated UTF-8
  strings (non-UTF-8 fails closed, the 0045 rule extended to GC).
  Root COMPLETENESS at production time stays a documented exclusion
  (handoff §5.3) — a perfect collector still needs complete roots.
- **Structural op contracts (§6.3)**: `Op`'s fields are private with
  one planner constructor enforcing the coherence table (deploys carry
  authority, removals carry their `RemoveTarget` in the variant,
  markers carry only a note); `ExecutableOp` gates the executor, so a
  preview-only op reaching execution is a classified error, never an
  `unreachable!`. One planner still serves plan/apply/rollback.

## Non-goals this round

`verification/guarantees.md` gains `CLASSIFY-001` and `OWNERSHIP-001`
and upgrades `GC-RECOVERY-001` to `checked`; the remaining entries
stay `implemented-unverified` until their kernels migrate. The
explorer/TLA+ evidence stays exactly as load-bearing as before — the
explorers now drive the verified functions, which strengthens them.
Merge-splice, scheduler and HTTP-budget kernels stay on the roadmap
in the established ROI order.

## Acceptance

- `cargo build --locked -p gripsack-policy` (and the musl release
  build) compiles with no verifier installed — proof erasure holds.
- `docker compose run --build --rm verify` passes: ≥20 obligations, 0
  errors, and the seeded mutant rejected on its postcondition.
- Store/exec suites green: reconcile, both explorers, the lineage
  explorer, the ops VM harness and the repeated-crash model drive the
  migrated kernels unchanged; the exec models caught — and the final
  contracts now pin — the planner's late-produces shape.
