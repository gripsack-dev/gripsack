# 0039 - Build closures (handover plan)

Status: **handover — planned, not started**. A fresh agent executes
this in a separate session. Everything below is decided or
recommended; the marked brainstorm items are open for the
implementer to evaluate. Owner-approved shape.

The rules that govern this repo apply in full: plan-first for
behavior changes, docker gates green (`test` / `ts-test` / `e2e` /
`model`), high code quality (no monster modules — split at ~800
lines; no naked types where a mixup is possible — newtype or
document; see `plan/0036`), website updated for user-visible
changes, release cut at the end, `plan/STATUS.md` updated in the
same PR.

## The problem

A build dependency today is a DEPLOYED dependency: to build
`consumer` against `compiler`, gripsack links the compiler's
binaries into the user's `~/.local/bin` and retains them in the
generation. The compiler pollutes the user's PATH purely to serve a
compile; removing the consumer leaves the compiler behind. (This was
a third-party review finding — 0035 F4's related gap.)

## The design (owner-approved)

A dependency edge marked `for: "build"` is a build-only edge: the
dependency is fetched and published to the store as today, but plans
ZERO destination ops — nothing links into HOME, nothing activates,
nothing enters the manifest's `entries`. During the consumer's
`build`/`custom` steps, the step's environment gains the closure:

```
PATH = <closure bin dirs, graph order> : $PATH
GRIP_DEP_COMPILER = /home/u/.local/share/gripsack/store/<hash>-compiler
```

Prepended, never replaced. (A controlled minimal PATH is the stricter
later option — NOT this item.)

```ts
export default module("consumer", {
  depends: [dep("compiler", { for: "build" })],
  steps: [
    shellStep("mkdir -p out && cp $(which cc) out/built", "build"),
    installStep({ "out/built": symlink("~/.local/bin/built") }, "install", { needs: ["build"] }),
  ],
});
```

### Semantics, pinned

- **DSL**: `dep("x", { for: "build" })` — one dependency list, the
  split is a property of the edge. IR: `Dependency` gains
  `for: "build" | "runtime"` (serde default `runtime` — additive, no
  `ir_version` bump; the three-sides rule applies: `schema/ir/v1.json`,
  `crates/gripsack-ir`, `typescript/` in one PR).
- **GC**: the closure's store paths must survive while the consumer's
  generation lives: `ModuleState.build_closure: Vec<PathBuf>`
  (serde default; gc reachability reads it like `tree256` does).
- **Transitivity**: the closure is the transitive set of build edges.
  A build dep's RUNTIME deps are NOT in the closure (they would be
  deployed by the runtime graph — that divergence is a documented
  boundary; a build dep with runtime deps gets them deployed as
  today, by the runtime edges, not the closure).
- **Verification**: build-only deps get no destination verifies
  (nothing deploys). Artifact verify steps on the staged payload run
  with 0035 receipts, unchanged.
- **Rollback**: nothing to restore (nothing deployed). No rebuilds at
  rollback — rollback restores files, it never rebuilds.
- **Plan/preview**: the op list shows the closure line per build-only
  dep — a marker op ("fetch + stage for build, not deployed"), never
  a destination op. The preview and apply agree by construction
  (0034's one planner).

## Implementation map (as of 0.34.0 — verify, the tree moves)

- DSL: `typescript/src/deps.ts` (`dep()`), `typescript/src/module.ts`
  (known-fields list — 0035 F3's strict validation).
- IR: `crates/gripsack-ir/src/model.rs` (`Dependency`), sema for the
  `for` value (unknown values are an error, span-labeled).
- Expansion/scheduling: `crates/gripsack-exec/src/expand.rs`
  (`dep_edges`), `schedule.rs` (a build edge orders the same way).
- The phase machine: `crates/gripsack-exec/src/module.rs` —
  produce/publish run for build-only deps; their deploy phase emits
  no ops. The build-step env: `build_step`/`run_shell` get the
  closure's PATH + `GRIP_DEP_*` vars (named `BuildEnv` struct — the
  env map is data, not ad-hoc set env).
- Manifest: `crates/gripsack-store/src/generations.rs`
  (`ModuleState.build_closure`), gc reachability in `gc.rs`.
- Preview marker op: `crates/gripsack-exec/src/ops/` (a marker op
  kind or a note on the module's op group — implementer's call, keep
  `ops/` files small).

## Proving it stays sound (the owner's ask: no regression-chasing)

The closure adds NO new destination semantics — the proofs to land:

1. **Op harness** (`crates/gripsack-exec/src/ops/model.rs`): a new
   case class — a build-only dep in the graph produces ZERO
   destination ops; the consumer's ops are unchanged. (The harness
   drives the shipped planner over materialized states — extend its
   world with a build edge.)
2. **Lineage explorer** (`lineage_model.rs`): unchanged — nothing
   about the ownership algebra moves. If you find yourself editing
   it, stop and re-read this plan: the algebra is closed.
3. **TLA+**: no new spec. Nothing protocol-level changes (the
   transaction and activation specs don't know what a build edge is,
   and shouldn't). Document this decision in the PR — the rule stands:
   TLA+ for protocols, the Rust harness for decision functions.
4. **e2e**: the reviewer's fixture as a journey — compiler build-dep,
   consumer builds, update the compiler pin → consumer rebuilds
   (0035 F4's keying must hold across the closure), remove the
   consumer → `~/.local/bin` has no compiler, `gc` keeps the closure
   while the generation lives and collects it after. Plus a
   journey-harness seed (`e2e/test_journey.py`) with a build edge in
   the world.
5. **Gates**: all four docker gates + macOS CI green; `grip plan`
   shows the closure line.

## Acceptance

- The reviewer's F4 fixture with `for: "build"`: the compiler is
  never in HOME, the consumer builds against it, the pin bump
  rebuilds, removal leaves nothing, gc retains-while-live.
- `grip check` rejects an unknown `for:` value with a span-labeled
  diagnostic.
- plan/STATUS.md row for 0039 in the same PR; the roadmap's "Build
  closures" item moves to Shipped (site repo, docs PR);
  `specs/` unchanged (with the PR noting why).
- Release: `core-v0.35.0` (or the next free minor).

## Brainstorm for the implementer (evaluate, don't blindly build)

Owner-blessed expansions to consider during implementation — report
the verdict in the PR (adopt / defer-with-roadmap-entry / reject-with-reason):

1. **Edge-type lattice**: today `runtime` (default) vs `build`. Is
   there a real third? Candidates: `for: "verify"` (tools needed only
   to verify — but linters are in-core now, so maybe nobody), and
   toolchain-style edges (a dep that PROVISIONS a runtime, like our
   own deno/pixi provisioning — could `for: "build"` toolchains fold
   provisioning into the graph?). Evaluate against real fixtures;
   a type nobody needs is a rejected type.
2. **What a dep provides**: binaries today (`bin/` on PATH). Libraries
   and headers would want `LIBRARY_PATH`/`CPATH`-style vars
   (`provides: ["bin", "lib", "include"]`?). Only adopt with a real
   consumer fixture — speculative env vars are clutter.
3. **Controlled PATH** (the strict option): replace ambient PATH with
   system tools + closure only. Closer to hermetic; breaks builds
   that silently use host tools (that breaking is arguably the
   point). Evaluate: how many existing fixtures would break? If
   adopted, it's a separate flag per edge or a global setting —
   decide which and why.
4. **Env-exporting build deps**: a build dep whose `env` exports flow
   into the consumer's build steps (the DAG already has env
   inheritance for runtime deps at build time — check
   `module env inheritance for dependents` on the roadmap; closures
   may fold it in or stay separate).
5. **The op-level surface**: should "prepare the closure" be an Op
   kind of its own (visible in `grip plan` as a first-class step),
   or a phase note? The ISA is young; prefer the smallest extension
   that makes the preview honest.

Historical note for context: the owner shipped a primitive
"ephemeral dependencies" version once (deploy-and-clean-after) — the
closure approach replaces that idea entirely: nothing deploys, so
nothing needs cleaning.
