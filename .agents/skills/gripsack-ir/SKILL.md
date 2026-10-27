---
name: gripsack-ir
description: Evolve the gripsack IR safely — schema, Rust types, TypeScript emitter change together
---

# Evolving the IR

The IR is a three-party contract. Any change lands in **one PR** touching:

1. `schema/ir/v<N>.json` — the JSON Schema, the source of truth.
2. `crates/gripsack-ir` — serde types + validation, mirroring the schema.
3. `typescript/src/` — the workspace frontend, emitting declarations from
   ordinary TypeScript through the root `gripsack.ts` entrypoint.

## Rules

- The current reader is strict: `deny_unknown_fields` plus tagged admission.
  A structural field that an older reader rejects requires a version bump.
- **Breaking changes** bump `ir_version` and replace the current schema and
  reader together. The core and frontend speak exactly one version. Retire
  obsolete authoring models, exported APIs and callers; do not add shims.
  Historical schemas remain available in Git history.
- Persisted generations, receipts, locks and recovery journals are separate
  contracts. Authoring cleanup must not discard or reinterpret retained state.
- **Provenance is mandatory**: every new semantic declaration node has
  a required `span: {file, line, col?}` from the emitter; nested values
  with no independent declaration inherit their owner's span for
  diagnostics. The core preserves/surfaces provenance and never hashes it.
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
