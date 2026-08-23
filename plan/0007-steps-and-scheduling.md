# 0007 — Steps, resources, and scheduling

- Status: draft
- Date: 2026-08-22
- Amends: 0001 §3.1 (module shape), 0004 §4 (passes gain *expand*)

## 1. The model

A **step** is the phase building block inside a module: one node in the
execution DAG with a typed action, explicit dependencies, and resource
requirements. A **module** is a container of steps plus edges to other
modules.

The decisive choice: **steps exist, but nobody is forced to write
them.** Two authoring shapes, mutually exclusive per module:

- **Declarative** (default): the fields you know — `source`, `build`,
  `install`, `config`, `activate`. The core *expands* them into the
  conventional pipeline (`fetch → build → install → config → activate`)
  in a new pass, **expand**, between validate and resolve (0004 §4).
  Simple modules stay three lines long.
- **Explicit**: `steps = [...]` — full control. Declaring `steps`
  forbids the declarative fields on the same module (`E103`), because
  two sources of truth for one pipeline is how builds become
  undebuggable.

After expand, the scheduler only ever sees steps. Frontends stay thin
(serializers); behavior is identical across Python and TypeScript by
construction.

## 2. Step shape

```json
{
  "id": "patch",
  "action": {"kind": "custom_shell", "script": "…"},
  "needs": ["fetch", "rust:install"],
  "resources": ["cargo-lock"],
  "phase": "build"
}
```

- `id` is module-scoped; cross-module refs are `module:step`. Every
  synthesized module also gets a `done` barrier step, which is what a
  plain module-level dependency edge binds to — so `depends = ["rust"]`
  and `needs = ["rust:install"]` coexist, the latter being the finer
  tool (e.g. an ephemeral toolchain: you need its *install*, not its
  config deploy).
- `phase` is a reporting/ordering tag (`fetch | build | install |
  config | activate | custom`), not a scheduling barrier.
- Primitive action kinds: `fetch(source)`, `build(spec)`,
  `install(entries)`, `config_deploy(entries)`, `intent(action)`,
  `custom_shell(script)`.

## 3. The escape hatch — and the door we keep locked

Ladder, same philosophy as sourcing (0002 §2):

1. **Typed primitive** — the engine interprets it: cacheable,
   introspectable, shown by `plan`.
2. **`run`** — a *structured* action: `argv`, `env`, `cwd`, `outputs`
   as data. No shell interpretation, no quoting bugs; declared outputs
   keep it cacheable and satisfiable (0008 §4). This is the
   Pants/Bazel lesson: build logic should produce action *descriptions*
   (data), not shell text — and it covers most things people reach for
   shell for ("run this binary with these args").
3. **`custom_shell`** — script *content* in the IR (or a file from the
   env repo, deployed and hash-verified like any other payload).
   Declared, flagged in `plan`, cache-busting without declared
   `outputs`. The last rung, for genuinely shell-shaped work (pipes,
   redirects).
4. **Builder/fetcher plugin** — reusable logic becomes a new primitive
   out-of-tree (0002 §4).

What the field teaches:

- **Nix** embeds shell snippets in the DSL (phases as bash strings) —
  the cautionary tale: untyped, unquotable, undebuggable.
- **Guix** stages Scheme into the store via g-expressions — true
  single-language builds, but it works because Scheme is code-as-data;
  reproducing it means shipping a language runtime into every build.
  Not our trade.
- **Bazel/Buck2** have custom logic (Starlark) *register structured
  actions* (argv, inputs, outputs, env) that the engine executes and
  caches — logic in the language, execution as data.
- **Pants** is our closest architectural sibling: Python `@rule`s
  return `Process` *descriptions* to a Rust engine. Same split as our
  eval/execute halves.

Our answer sits at their intersection: all *decision logic* lives in
your typed Python/TS at eval time (single language for config AND
logic); everything that crosses into the core is data — primitives,
then `run`, then shell. Guix-style staging is the one model we
explicitly reject: the core never evaluates code.

What we will NOT do: `python_module("...")` or any code-by-reference
step action. The moment executable code rides inside the IR, the core
is evaluating again — provenance dies (a failure points at generated
code, not your module), caching becomes unsound (the hash can't see the
function's closure), and we've reinvented nix-lang's worst property
with extra steps. If a primitive is missing, the eval-time escape hatch
already exists: compute in your frontend and emit a primitive. If it
must run at execute time, that's `custom_shell`. If it's reusable,
that's a plugin. Three doors, all honest.

## 4a. Verify

A step or module MAY carry a `verify` — a smoke contract, not a test
framework:

- **Primitives verify themselves for free** (fetch checks sha256,
  install checks link targets). Explicit verify matters for
  `custom_shell` — an opaque action needs an explicit contract — and as
  a module-level smoke test (`hx --version`).
- Kinds: `binary_runs {path, args}`, `file_exists {path}`,
  `shell {script}`.
- Module-level verify is a synthesized terminal step:
  `fetch → build → install → config → verify → activate`. It runs
  **pre-flip** — a broken module never activates. Post-flip failure
  stays degraded-generation territory (0001 §3.8); verify never runs
  post-flip, and never on a no-op apply (0008 §4).
- Verify failures get their own diagnostic class (`E3xx`) so
  "step failed" and "verify failed" are greppable apart.

## 4b. Retries

- **Default retries apply only to fetch actions** (network transients:
  connect errors, 5xx, 429). Everything else defaults to 0 — a failing
  build is almost always deterministic, and a `custom_shell` may not be
  idempotent (fetch is, because the destination is content-addressed).
- **Failure classes, not blanket retry**: hash mismatch, 404, and
  validation errors are never retried — a lockfile hash mismatch is a
  tampering signal (0002 §4).
- **Hierarchy**: engine default < module `retries` < step `retries` — a
  bare count; backoff policy is engine-owned (exponential + full
  jitter). Attempts are reported (`attempt 2/4 after 1.3s: …`).

## 4c. Throttling

Concurrency caps (resources) don't solve rate. Throttle domains do:

- Token-bucket domains in the core; conservative built-in budget for
  `api.github.com`; primitives auto-attach to their domain. Custom
  domains in `env.toml`: `[throttle] "api.corp.com" = "5/s"`.
- 429 handling honors `Retry-After` (bounded) within the step's retry
  budget before failing.
- Fetcher plugins declare their budget in `capabilities` (0002 §4).
- Built-in resolution ("latest release") happens **in the core at
  lock/update time** (0002 §7) so built-in API traffic stays inside the
  throttle; eval-time resolvers are outside it by nature.

## 4. Resources

Named, host-global, capacity-1 by default (mutex). A step runs only when
every resource it declares is free.

- **Primitives auto-declare their contention domains**: the pixi
  fetcher requires `pixi-lock`, cargo builds require `cargo-lock`,
  network fetches share a bounded `network` pool (capacity = small N,
  the one non-1 default). Users only declare resources for their own
  `custom_shell` steps that touch shared state.
- **Cross-process safety**: named resources map to `flock` files under
  `$GRIPSACK_HOME/locks/`, so two concurrent `grip` runs serialize too
  — in-process semaphores alone wouldn't.
- **The namespace is closed by declaration.** The IR carries a
  top-level `resources` section (`[{"name": "pixi.lock"}]`); frontends
  expose `resource("pixi.lock")` markers. A step's `resources` must
  resolve to declared resources ∪ core built-ins (`network`,
  `pixi-lock`, `cargo-lock`) — anything else is a hard error (`E107`).
  Frontends also validate at eval time, so a typo fails in the user's
  editor run, before the core ever sees the IR.

## 5. Scheduling: no waves

The engine builds one **global step DAG** for the whole operation and
runs a ready-queue scheduler: a step becomes ready when its `needs`
have finished *and* its resources can be acquired; up to N run
concurrently (N = cores). There are no phase waves — materializing
"fetch everything, then build everything" as barriers only constrains
parallelism; the DAG already encodes what actually has to wait.

The single global barrier is the **generation flip** (0001 §9.2):
`post_link` intents run as each module's steps complete; the flip
happens once, atomically, after everything; `post_activate` intents run
after. Execution states for reporting: `pending → blocked(resource) →
running → done | failed`.

## 6. Semantic passes gained

- `E103` — module mixes `steps` with declarative fields.
- `E104` — `needs` references an unknown step (sibling or `module:step`).
- `E105` — step-level cycle (after expansion; names the cycle members —
  module-level cycles now surface here with step precision).
- `W201` — unknown resource name.

## 7. Authoring styles

Two ways to write a module, both producing the same IR:

- **Data style** — `module(name, fetch=..., config=...)`: declarative
  fields the core expands (§1). The default for simple and
  dotfiles-only modules.
- **Class style** — subclass `Module` and override phase methods
  (`fetch()`, `build()`, `install()`, `config()`, `verify()`,
  `activate()`), each returning a step or list of steps. Phase methods
  run **at eval time only** — they build data, never execute at build
  time. The pipeline chains steps: within a phase and across phase
  boundaries a step with empty `needs` needs the previous step;
  explicit `needs` always win. The chaining is compiled to explicit
  `needs` in the emitted IR — the IR carries no sugar, and the
  conformance test is that both frontends emit identical IR for the
  same logical module.

## 8. Library structure (both frontends)

Single-file packages stop here. Both frontends become real libraries:

```
python/gripsack/                typescript/src/
  fetch.py      — fetchers        fetch.ts
  entries.py    — Dest/ownership  entries.ts
  deps.py       — Dependency      deps.ts
  intents.py    — activation      intents.ts
  steps.py      — Step + helpers  steps.ts
  facts.py      — host facts/tags facts.ts
  resources.py  — declarations    resources.ts
  module.py     — module + Module module.ts
  graph.py      — registry, emit  graph.ts
```

`__init__.py` / `index.ts` re-export the public API — existing module
files keep working unchanged.
