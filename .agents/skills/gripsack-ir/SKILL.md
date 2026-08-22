---
name: gripsack-ir
description: Evolve the gripsack IR safely — schema, Rust types, Python emitter change together
---

# Evolving the IR

The IR is a three-party contract. Any change lands in **one PR** touching:

1. `schema/ir/v<N>.json` — the JSON Schema, the source of truth.
2. `crates/gripsack-ir` — serde types + validation, mirroring the schema.
3. `python/gripsack/` — the emitter (`to_ir()` shapes).

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

## Checklist

- [ ] `schema/ir/` updated, version bumped if breaking
- [ ] `gripsack-ir` types + validation + unit tests updated
- [ ] `python/` emitter + tests updated
- [ ] `gripsack-store` hashing inputs reviewed (identity change intended?)
- [ ] compose gates green (`test`, `pytest`, `e2e`)
