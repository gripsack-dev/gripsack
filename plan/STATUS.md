# STATUS — what landed, what is open, what is settled

One row per plan; the plan holds the detail. Update the row in the same
change as the plan. Long-form release/evidence narratives were removed by
[0056](0056-verified-foundation.md) F0; they remain in git history
(`git log -p -- plan/STATUS.md`).

## Now

- **Latest release:** [core-v0.52.0](https://github.com/gripsack-dev/gripsack/releases/tag/core-v0.52.0) (2026-11-09), Linux/WSL x86_64 only.
- **Active plan:** [0056 — verified foundation](0056-verified-foundation.md), approved 2026-11-08.
  Its ledger (§9 phases, §13+ records) is the source of truth for the epic.
- **Independent design review:** [2026-11](../docs/reviews/2026-11-design-review.md) — findings are leads; each is verified before it changes a plan.
- **Superseded by 0056:** 0047 tooling decisions; 0048 §6 (M-V1–M-V8), §9–§10 as
  release gates and its evidence/receipt contract; the formal and evidence
  parts of 0049–0055. Runtime safety contracts are kept and become theorems.
- **Open product work** from 0048–0055 (the original handover) is re-homed by
  0056's handover reconciliation, not tracked here.

## Plans

| Plan | Title | Shipped |
|---|---|---|
| 0001 | Architecture: modules, store, generations, ownership, invariants | 0.x foundation |
| 0002 | Sourcing: resolver/transport split, fetcher protocol | foundation |
| 0003 | Repo, gates, releases | foundation |
| 0004 | Rich IR + compiler passes + provenance | foundation |
| 0005 | Frontends + configuration | foundation |
| 0006 | Gradual migration | foundation |
| 0007 | Steps, resources, scheduling | foundation |
| 0008 | Canonicalization + satisfaction | foundation |
| 0009 | Diagnostics across boundaries | foundation |
| 0010/0011/0012 | Plugin provisioning → validation plugins → linters in core | 0.13.0–0.14.0 |
| 0013 | Constrained evaluation (sandboxed Deno, trust gate) | 0.15.x |
| 0014 | Content-addressed fetches | 0.16.x |
| 0015 | `grip adopt` | 0.16.x |
| 0016 | Platform facts, floating git, read-only store | 0.16.x |
| 0017 | Hardening/fuzzing playbook | process |
| 0018 | Critique response 0.17.14 | 0.17.14 |
| 0019 | Deploy journal (crash recovery) | 0.18.x |
| 0020 | Review response 0.18.1 | 0.19.x–0.20.0 |
| 0021 | cap-std fs hardening | 0.21.0 |
| 0022 | SBOM cargo-auditable | 0.21.x |
| 0023 | Case-fold everywhere | 0.26.0 |
| 0024 | Review response 0.21.0 | 0.21.x |
| 0025 | Transaction coverage | 0.22.0 |
| 0026 | Path-centric transactions | 0.23.0 |
| 0027 | Provable transactions | 0.24.0 |
| 0028 | Machine-checked model | (no release) |
| 0029 | Ownership lineage | 0.25.0 |
| 0030 | Canonical destinations | 0.26.0 |
| 0031 | Mode-aware identity | 0.27.0 |
| 0032 | Durable activation | 0.28.0 |
| 0033 | Review 0.27.0 | 0.29.0 |
| 0034 | Shared operation list | 0.30.0 |
| 0034+ | VM-level op harness | 0.31.0 |
| 0035 | Review fb11aaf | 0.32.0 |
| 0036 | Representation audit | (docs only) |
| 0037 | Rollback activation | 0.33.0 |
| 0039 | Build closures | core/TS 0.35.0 |
| 0038 | Journey harness | 0.34.0 |
| 0040 | Post-0.35 project sweep | review |
| 0041 | P1 contract fidelity and persistence | core/TS 0.36.0 |
| 0042 | Bounded acquisition, complete pins and stronger evidence | core/TS 0.37.0 |
| 0043 | Migration feedback and permission identity | core/TS 0.38.0 |
| 0044 | Complete surveys, explicit version spellings and bounded HTTP failures | core/SDK 0.39.0 |
| 0045 | Recovery admission hardening, mutation sessions, editor-reachable frontends | core/SDK 0.40.0 |
| 0046 | Verified decision kernels (Verus pilot + ownership + GC retention + op contracts) | core/SDK 0.41.0 |
| 0047 | Verified merge splice, graph closures and scheduler transitions | core/SDK 0.42.0 |
| 0048 | Unified review handoff with binding release and quality gates | 0.43–0.51 (partial) |
| 0049 | Handover bundle import (edition 5) — H0 reconciliation + A0 bottle selection | superseded |
| 0050 | E0 task/schedule scope and proof targets | partial (systemd) |
| 0051 | Linux BuildKit production and B-row qualification | retired (0055) |
| 0052 | Workspace graph, command, diagnostic and native-runtime contract | 0.44–0.51 |
| 0053 | CI and release-flow migration | Buildbot local qualification |
| 0054 | Workspace ergonomics and original handover completion | 0.48 |
| 0055 | One workspace surface | 0.50–0.51 |
| 0056 | Verified foundation: Lean-proved kernel, rings, trim | F0 + F1 (GO) in 0.52.0; F2 step 1 (`gripsack-host` Storage/RealFs/MemFs, resurrection rediscovered) in review |
| 0057 | Fast qualification: cache mounts, one dev profile, parallel e2e/mutants/package, theorem-backed test trim | landed in 0.52.0 (qualified in ~20 min warm, 36 min cold) |
| 0058 | Handover final reconciliation (supersedes 0056a): 25 landed, 18 obsolete, 13 pending, 3 rejected | landed (docs) |

## Settled rejections (all eras)

- **TOML/data-format frontend** — five times. TypeScript is the
  authoring surface.
- **cap-std piecemeal** — the migration was all-or-nothing (0020);
  0021 did it whole.
- **Sidecar-only SBOM** — the SBOM travels inside the binary (0022).
- **SQLite for the store** — can't transact scattered HOME files;
  the fs IS the store (0035).
- **Machine-local pin grant setting** — the trust gate is the
  boundary; validation happens at the grant (0033).
