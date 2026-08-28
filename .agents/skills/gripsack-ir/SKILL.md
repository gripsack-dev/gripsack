---
name: gripsack-ir
description: Evolve the gripsack IR safely — schema, Rust types, TypeScript emitter change together
---

# Evolving the IR

The IR is a three-party contract. Any change lands in **one PR** touching:

1. `schema/ir/v<N>.json` — the JSON Schema, the source of truth.
2. `crates/gripsack-ir` — serde types + validation, mirroring the schema.
3. `typescript/src/` — the emitter (`module()` → IR shapes). The Python
   emitter is gone (plan/0013 D1); TypeScript is the single frontend.

## Rules

- **Additive changes** (new optional field, new source/build/intent kind)
  do NOT bump `ir_version`. Readers tolerate unknown fields (plan/0003 §8);
  old cores ignore them. Do not abuse this for semantic changes.
- **Breaking changes** (rename, removal, meaning change) bump
  `ir_version` and add `schema/ir/v<N+1>.json`; keep the old schema file.
  The core accepts a declared range; the frontend emits exactly one
  version.
- **Provenance is mandatory**: new node types get
  `provenance: {file, line}` from the emitter. The core preserves and
  surfaces it, never interprets it.
- Store-path identity: the input hash covers the resolved module plan.
  Adding a field that affects what gets built/fetched MUST change the
  hash input; adding metadata (provenance, docs) MUST NOT.
- The golden IR corpus (`e2e/fixtures/golden/`) snapshots the emitted
  envelope — an IR change regenerates it
  (`REGEN_GOLDEN=1 pytest e2e/test_golden.py`, see the gripsack-e2e
  skill) and the snapshot diff is part of the PR evidence.

## Checklist

- [ ] `schema/ir/` updated, version bumped if breaking
- [ ] `gripsack-ir` types + validation + unit tests updated
- [ ] `typescript/` emitter + tests updated
- [ ] `gripsack-store` hashing inputs reviewed (identity change intended?)
- [ ] compose gates green (`test`, `ts-test`, `e2e`)
- [ ] golden corpus regenerated and the snapshot diff reviewed
