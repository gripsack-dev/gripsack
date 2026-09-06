# STATUS — the ledger: what landed, what's deferred, what's rejected

One row per plan. "Deferred" items live on the
[roadmap](https://gripsack.dev/docs/roadmap.html); "rejected" is
settled (don't relitigate without new evidence). Update this file in
the same PR as the plan it tracks (convention since 0036).

## Foundations (0001–0019)

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

Deferred from this era (on the roadmap): resolver executables (0013
D8), rollback adapters, module env inheritance for dependents,
secrets model, more probe kinds, more reference fetchers; the LSP and
the fetcher registry stay the north star.

## The audit/review era (0020–0036)

| Plan | Title | Release | Landed | Deferred → roadmap | Rejected |
|---|---|---|---|---|---|
| 0020 | Review response 0.18.1 | 0.19.x–0.20.0 | journal run marker, quarantine, durable cleanup, plan reversibility labels, macOS CI + attestations | SBOM (0022), cap-std (0021), signed channel manifest | cap-std piecemeal, most product proposals |
| 0021 | cap-std fs hardening | 0.21.0 | all five phases (cap-std Dir capabilities, fd-relative writes) | non-UTF-8 symlink targets end-to-end | — |
| 0022 | SBOM cargo-auditable | 0.21.x | in-binary SBOM, release verification | CycloneDX sidecar (when a user asks) | sidecar-only SBOM |
| 0023 | Case-fold everywhere | 0.26.0 | E111 case-folds on every host (stashed → landed with 0030) | — | — |
| 0024 | Review response 0.21.0 | 0.21.x | migration-report fixes | one-commit release modules (carried) | — |
| 0025 | Transaction coverage | 0.22.0 | rollback/prune/env-profile transactions, run-level compensation | the full kill-point matrix | — |
| 0026 | Path-centric transactions | 0.23.0 | generation identity, exact txn identity, intent recorded pre-mutation, mode-preserving writes | `--force` drift overwrite (owner decision pending), mode-in-identity (→ 0031) | — |
| 0027 | Provable transactions | 0.24.0 | postcondition verification, preflight, validated generations | persistent fuzz harnesses in-repo | — |
| 0028 | Machine-checked model | (no release) | Transaction.tla + TLC gate, Rust explorer driving shipped classify/decide, mutation calibration | — | — |
| 0029 | Ownership lineage | 0.25.0 | origin rides the epoch, observed ≠ authorized, lineage explorer | — | — |
| 0030 | Canonical destinations | 0.26.0 | E119 aliases, single-observation deploys, lineage retention on re-take-over, exact removal guard, legacy-marker refusal (later purged, 0035) | activation hooks (→ 0032), grip resolve, CAS displacement, merge aggregation | TOML frontend (5th time) |
| 0031 | Mode-aware identity | 0.27.0 | full mode in identity, chmod drift detection, exact mode restore, deterministic landing modes | — | — |
| 0032 | Durable activation | 0.28.0 | pre-flip pending record, resume/discard, Activation.tla + TLC, idempotency contract | rollback activation (fires intents on rollback) | — |
| 0033 | Review 0.27.0 | 0.29.0 | take-over keeps private modes, prior store 0600/0700, pin-grant validation, step needs ordering (E120), plan runs full gates, preserved-drift blocks mode switch, adopt codegen sanitizes | shared op list (→ 0034), fetch memory budget | machine-local pin setting; "plan predicts external state" |
| 0034 | Shared operation list | 0.30.0 | one planner for plan/apply/rollback; ops/{ISA, codegen, VM, preview}; preview is honest about drift | — | a new TLA+ spec for the op list (the protocol models already cover it) |
| 0034+ | VM-level op harness | 0.31.0 | 560 materialized cases: planner ≡ algebra, execution lands intent | — | — |
| 0035 | Review fb11aaf | 0.32.0 | canonical DestinationKey in manifests, verify receipts, strict TS fields, dep-pin build keys, closed txn boundary, vendored frontend (cargo install works), read-only accurate preview, normalized step graph, on_remove fires, reserved GRIPSACK_*, recursive fsync publish | build closures, rollback activation, streaming/memory budget, persistence-fault evidence | SQLite, rewrites, machine-local pin setting |
| 0036 | Representation audit | (docs only) | the typed/bare ledger + the convention (typed producers, plain wire, explicit seams) | GenerationId newtype (with the revisit trigger: a second u64 domain sharing a signature) | — |
| 0037 | Rollback activation | 0.33.0 | rollback runs the target generation's intents (durable, crash-resumable via 0032's record); rollback-undeclared modules fire on_remove | per-entry intent diffing (if anyone asks) | — |
| 0039 | Build closures | core/TS 0.35.0 | IR v2 `Dependency.for`; store-only build deps, graph-ordered PATH + GRIP_DEP_*, payload receipts, same-run dependency pins, generation-pinned GC, zero-destination op case + seeded journeys | library/header exports and controlled PATH (low priority, real-consumer/opt-in triggers); general env inheritance retains its existing roadmap priority | speculative verify/provisioning edge types; extra closure Op kind (marker suffices) |

| 0038 | Journey harness | 0.34.0 | seeded random journeys + expectation model + system oracles; caught the preserved-verify bug on day one | wider world (more module kinds, parallel-applies stay TLA+'s) | — |

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
