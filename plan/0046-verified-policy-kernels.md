# 0046 — Verified decision kernels: the Verus pilot and its gates

Adopts the hardening handoff's PR3 (small production Verus pilot) and
its proof-runner/CI requirements (§10), with the pilot deliberately
smaller than the highest-value theorem: the recovery commit classifier.

## What lands

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

## Non-goals this round

No other kernel moves yet. `verification/guarantees.md` gains
`CLASSIFY-001` with status `checked`; the remaining ledger entries
stay `implemented-unverified` until their kernels migrate. The
explorer/TLA+ evidence stays exactly as load-bearing as before — the
explorers now drive the verified function, which strengthens them.

## Acceptance

- `cargo build --locked -p gripsack-policy` (and the musl release
  build) compiles with no verifier installed — proof erasure holds.
- `docker compose run --build --rm verify` passes: ≥3 obligations, 0
  errors, and the seeded mutant rejected on its postcondition.
- Store suite green: reconcile, both explorers and the repeated-crash
  model drive the migrated classifier unchanged.
