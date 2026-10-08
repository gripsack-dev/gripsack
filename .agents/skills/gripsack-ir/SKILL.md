---
name: gripsack-ir
description: Evolve the gripsack IR safely — schema, Rust types, TypeScript emitter change together
---

# Evolving the IR

The IR is a three-party contract. Any change lands in **one PR** touching:

1. `schema/ir/v<N>.json` — the JSON Schema, the source of truth.
2. `crates/gripsack-ir` — serde types + validation, mirroring the schema.
3. `typescript/src/` — the one frontend, emitting module-compatible or
   workspace declarations from ordinary TypeScript. The Python emitter
   is gone (plan/0013 D1).

## Rules

- **Structural changes** are compatible within a version ONLY if the
  prior declared reader explicitly tolerates that extension point. The
  retained v3–v6 readers and current v7 reader are strict
  (`deny_unknown_fields` plus versioned tagged admission); even an optional
  structural field needs a new version or proven reader negotiation
  (plan/0003 §8). V6 keeps its original executing semantics and rejects v7
  authority; v5 retains native file profiles and v4 workspaces stay read-only.
- **Breaking changes** (rename, removal, meaning change, or a field an
  older strict reader rejects) bump `ir_version` and add
  `schema/ir/v<N+1>.json`; keep old schemas and versioned readers. The
  core accepts a declared range; the frontend emits exactly one version.
- **Provenance is mandatory**: every new semantic declaration node has
  a required `span: {file, line, col?}` from the emitter; nested values
  with no independent declaration inherit their owner's span for
  diagnostics. The core preserves/surfaces provenance and never hashes
  it. Retained v3 optional spans are historical compatibility, not a
  precedent for typed workspace nodes.
- Identity: producer recipe hashes include admitted semantic inputs,
  tools, platform and policy. Consumer selection and provenance do
  not enter producer identity. Changing a source/build field must
  invalidate affected work; metadata-only changes must not.
- The golden IR corpus (`e2e/fixtures/golden/`) snapshots the emitted
  envelope — an IR change regenerates it
  (`REGEN_GOLDEN=1 pytest e2e/test_golden.py`, see the gripsack-e2e
  skill) and the snapshot diff is part of the PR evidence.
  Under an explicit owner no-replay instruction, regenerate by direct SDK
  emission only and review the resulting semantic diff; do not invoke golden
  test functions or report emission as a passing corpus campaign.

## Checklist

- [ ] `schema/ir/` updated, version bumped if breaking
- [ ] `gripsack-ir` types + validation + unit tests updated
- [ ] `typescript/` emitter + tests updated
- [ ] `gripsack-store` hashing inputs reviewed (identity change intended?)
- [ ] compose gates green (`test`, `ts-test`, `e2e`)
- [ ] golden corpus regenerated and the snapshot diff reviewed
