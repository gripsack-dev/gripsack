# 0004 — Rich IR, source mapping, and compiler passes

- Status: draft
- Date: 2026-08-22
- Amends: 0001 §3.2 (provenance), 0003 §8 (IR compatibility)

## 1. Why the IR must be rich

Every error gripsack ever produces should point at the user's module
source, not at generated JSON. That only works if source location is
*payload* — captured at node-creation time, threaded through every pass,
never recomputed. Three precedents:

- **React / `@babel/plugin-transform-react-jsx-source`**: injects
  `__source={fileName, lineNumber, columnNumber}` into every JSX element
  in dev builds. React's component error stacks point at your JSX, not
  the compiled output. Our `provenance` is exactly `__source`.
- **Babel / swc**: every AST node carries a span (byte offsets into a
  shared source map); transforms preserve spans so errors from any later
  pass still map back. Lesson: spans flow through the pipeline untouched.
- **rustc / miette**: diagnostics are structured data — code, severity,
  labeled spans, help text — collected across passes and rendered at the
  end. Compilers fail *slow*: report everything wrong, not the first
  thing.

We don't need VLQ source maps: those exist to map *generated text* back
to source. The IR is generated *data*; spans travel inline. Simpler and
lossless.

## 2. Span model

Every IR node type (module, entry, dependency, intent, source) carries an
optional `span`:

```json
"span": { "file": "modules/helix.py", "line": 14, "col": 1 }
```

- **Optional in the schema, mandatory in practice**: built-in frontends
  MUST emit spans on at least module level; node-level where the host
  language makes it cheap. Cores MUST NOT error on missing spans — a
  spanless node renders diagnostics with the module name as context.
- **Capture mechanism is frontend-specific**: Python uses `inspect`
  (frame file/line); TypeScript parses V8 stack frames (file:line:col).
  Both are eval-time, zero-cost-when-unused.
- Spans never participate in store-path identity (0003 — metadata, not
  build input).

## 3. Diagnostics are data

```rust
pub struct Diagnostic {
    pub code: &'static str,      // stable, greppable: "E101"
    pub severity: Severity,      // error | warning
    pub message: String,
    pub labels: Vec<Label>,      // span + per-span note
    pub help: Option<String>,
}
```

- **Stable codes** (`E1xx` structural, `E2xx` resolution, `E3xx` store,
  `E4xx` activation) — scripts, CI, and the future LSP match on codes,
  never on message text.
- Passes **collect** diagnostics and keep going where semantics allow
  (one bad module doesn't hide three others).
- The CLI renders human output by default and `--format json` for
  tooling. Same data, two renderers.

## 4. The pass pipeline

The core is a compiler over IR. Passes, each a pure-ish function with a
diagnostic sink:

```
IR JSON
  → 1 parse      syntax: JSON → ir::Ir            (E1xx: malformed, version)
  → 2 validate   structural sema                 (E1xx: unknown dep, bad dest)
  → 3 resolve    lockfile merge, pinning         (E2xx: drift, unpinned)
  → 4 lower      IR → exec graph, topo order     (E1xx: cycle, named)
  → 5 plan       diff vs current generation      (what would change)
  → 6 execute    fetch/build/install             (E3xx)
  → 7 activate   intents, flip, record           (E4xx, degraded)
```

Rules:

- Each pass takes the previous pass's output + a `&mut Diagnostics`.
- A pass with errors halts the pipeline *between* passes, never inside —
  pass 2 reports every bad module, not just the first.
- Spans are read-only payload end to end.
- Passes are library API (`gripsack-ir`, `gripsack-exec`), not CLI
  internals — anything the CLI does, a tool can do.

## 5. Why this buys an LSP for (almost) free

A gripsack LSP's value is *cross-module* checks pyright/tsc can't see:
unknown dependency names, destination conflicts between modules, cycles,
ownership-mode misuse — squiggled in the editor at the exact module line.

Because passes 1–2 are library functions returning structured
span-labeled diagnostics, the LSP is a **protocol shim**:

```
editor saves helix.py
  → shim re-evals the frontend (or accepts pushed IR)
  → runs parse + validate
  → maps Diagnostic{labels} → LSP Diagnostic via span
```

No new analysis logic, no second source of truth. What the IR must
guarantee for this — spans on every node, stable codes, JSON diagnostics
— is specified above and tested in the core, so the LSP starts as a
thin, boring program (tower-lsp + gripsack-ir). Scheduled after `apply`.

## 6. Non-goals

- No incremental/lazy query system (rustc's salsa) — env graphs are
  hundreds of nodes; recompute is microseconds. Revisit only if profiled.
- No span-perfect byte ranges — `{file, line, col}` granularity is
  enough; byte spans are a babel-scale requirement we don't have.
