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
2. **`custom_shell`** — script *content* in the IR (or a file from the
   env repo, deployed and hash-verified like any other payload).
   Declared, flagged in `plan`, and it busts fine-grained caching —
   honestly, visibly.
3. **Builder/sourcerer plugin** — reusable logic becomes a new
   primitive out-of-tree (0002 §4).

What we will NOT do: `python_module("...")` or any code-by-reference
step action. The moment executable code rides inside the IR, the core
is evaluating again — provenance dies (a failure points at generated
code, not your module), caching becomes unsound (the hash can't see the
function's closure), and we've reinvented nix-lang's worst property
with extra steps. If a primitive is missing, the eval-time escape hatch
already exists: compute in your frontend and emit a primitive. If it
must run at execute time, that's `custom_shell`. If it's reusable,
that's a plugin. Three doors, all honest.

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
- Unknown resource names are a *warning* (`W201`), not an error —
  resources are an open namespace; a typo degrades to "no mutual
  exclusion", which is worth surfacing but not blocking.

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

## 7. Library structure (both frontends)

Single-file packages stop here. Both frontends become real libraries:

```
python/gripsack/                typescript/src/
  sources.py    — fetchers        sources.ts
  entries.py    — Dest/ownership  entries.ts
  deps.py       — Dependency      deps.ts
  intents.py    — activation      intents.ts
  steps.py      — Step + helpers  steps.ts
  facts.py      — host facts/tags facts.ts
  module.py     — module()        module.ts
  graph.py      — registry, emit  graph.ts
```

`__init__.py` / `index.ts` re-export the public API — existing module
files keep working unchanged.
