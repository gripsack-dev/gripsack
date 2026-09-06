# 0039 — Build closures

Status: **implemented for core/TS 0.35.0**; release gates and publication follow.

## Execution amendment — alpha IR cutover

Owner approved an IR change rather than retaining compatibility solely
because the product previously shipped `edge`. Plan recorded before the
cutover:

1. **IR v2** replaces dependency `edge` with `for`, values `runtime`
   (default) and `build`. `schema/ir/v2.json`, Rust types, TypeScript
   output, and the golden corpus move together. Keep `v1.json` as
   historical documentation; core accepts v2 only. No aliases, shims,
   or fallback parser. A v1 document fails the version gate before
   field decoding, with advice to update a pinned frontend.
2. **DSL** is `dep(name, { for: "build" })`, emitting `for` directly.
   Reject old positional/options shapes, unknown purposes (E122 with
   a declaration span), and ambiguous `GRIP_DEP_*` aliases (E123).
   Rust's producer uses a typed dependency-purpose enum; invalid
   values never enter it.
3. **Deployment role** comes from the whole graph: any runtime
   incoming edge wins; build incoming edges alone make a module
   build-only; standalone modules deploy. Host membership supplies
   graph nodes, not a second root/deployment flag. Subset apply uses
   the same role decision as plan. Removing a consumer does not
   undeclare a tool still explicitly listed as a standalone module.
4. **Receipts and reachability**: retain build-only `ModuleState`
   with `build_only: true`, store identity and payload receipts, but
   empty `entries`, `intents`, and `env`. Compare store identity as
   well as checks before reusing a receipt. Consumers record
   `build_closure`; all retained generations pin these paths. GC
   releases them only once no retained generation references them.
5. **Execution**: order transitive build closure members using the
   existing validated dependency order, never a second scheduler.
   Freshly resolved dependency pins reach dependents in the same
   apply, so the next warm apply does not rebuild needlessly.
   Prepend closure bins to explicit step PATH or ambient PATH;
   preserve OS-string paths. No inherited module env exports.
6. **Proof and delivery**: exercise cold/warm applies, transitive pin
   changes, runtime/build transitions, subset apply, payload receipt
   failure/retry, rollback without builds, and GC with retained
   history. Extend the op harness and seeded journeys. Run all four
   compose gates and macOS CI. Update the site roadmap/docs and
   release both core and TypeScript with the IR v2 migration noted.

No destination algebra or transaction protocol changes: `lineage_model`
and `specs/` remain unchanged. The sections below retain the original
design motivation; this amendment governs where it refines them.

## Original handover requirements

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

- **DSL/IR**: `dep("x", { for: "build" })` — one dependency list,
  the split is a property of the edge. IR v2 `Dependency.for` has
  `build | runtime` with a runtime default. The version bump is
  required because this replaces the previously shipped `edge`
  field (see the execution amendment).
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

## Brainstorm verdicts

1. **Reject speculative edge types.** The fixtures need build/runtime
   only. In-core linters do not require a verify-tool edge; core Deno
   provisioning remains outside the environment DAG. Revisit a third
   purpose only with a concrete consumer, not a taxonomy exercise.
2. **Defer library/header exports, low priority.** No real lib/include
   consumer fixture justifies CPATH/LIBRARY_PATH/provides semantics.
   Recorded after reliability work on the site roadmap.
3. **Defer controlled PATH, low priority and opt-in.** The implemented
   shell/structured fixtures deliberately depend on system sh, mkdir,
   and cp. A strict mode needs an explicit system-tool baseline;
   simply removing ambient PATH would break them. Prefer a global
   build policy rather than conflicting per-edge PATH policies.
4. **Defer general env inheritance separately.** It was not already
   implemented: current module env is activation-profile data. Keep
   the existing roadmap priority; closure exports are PATH and typed
   dependency roots only, not implicit profile-env inheritance.
5. **Adopt a marker, not a new destination opcode.** The existing
   marker mechanism displays each build-only dependency while the
   shared graph projection excludes all its destination operations.

The implementation is split across IR graph decisions, BuildEnv,
scheduler input snapshots, the per-module lifecycle and its receipt
submodule. No new transaction or ownership algebra was introduced.
