# 0040 — Post-0.35 project sweep: contract fidelity before more breadth

Status: **review complete; P1 implementation approved and tracked by [0041](0041-p1-contract-fidelity.md).**

Requested by the owner after 0039 landed and core/TypeScript 0.35.0
published. This is a proposed maintenance/architecture program, not a
claim that the proposals below have shipped. Read the priority table
first; the decisions to discuss are at the end.

## Baseline and review boundary

Reviewed release: `core-v0.35.0`, commit
`cfb97f3284d4de68aad92aa31c347b9fc8b9d399`.

- Feature PR: [#133](https://github.com/gripsack-dev/gripsack/pull/133).
- [Release-commit CI](https://github.com/gripsack-dev/gripsack/actions/runs/34061794124)
  passed Rust fmt/clippy/tests, TypeScript tests, 153 flow tests,
  native macOS flow tests, audit and TLC.
- [Core publication](https://github.com/gripsack-dev/gripsack/actions/runs/34061815739)
  verified four platform artifacts, published the crate set and
  GitHub release; npm 0.35.0 was downloaded and exercised separately.
- The sweep did **not** modify the released implementation. Thirteen
  small, offline, throwaway probes ran the real binary/frontend in
  disposable container HOME directories. These probes are diagnostic
  experiments, not thirteen new permanent tests. Their observations
  are recorded below so they can become focused regressions if we
  approve the corresponding contracts.
- Probe results below are **Linux-container observations**. The
  release's macOS gate passed, but these additional probes were not
  repeated on macOS. No memory-exhaustion or power-loss stress run
  was performed.

Coverage was a project-wide boundary sweep, with deeper tracing at
suspect seams—not a line-by-line proof or an exhaustive security audit:

| Surface | Reviewed boundaries |
|---|---|
| TypeScript and IR | dependency/step DSL, wire/version validation, schema, semantic passes, spans |
| Execution | expansion, module scheduling, identity, publication, verification, preview, activation, rollback, GC |
| Store and filesystem | manifest validation, journal/recovery interfaces, capability writes, same-FS/EXDEV publication, locking |
| Fetch | HTTP/auth/proxy/root construction, archive extraction, download caps, plugin conversations/provisioning |
| Lint and configuration | pack loading/checks, config-path discovery, subprocess protocol, config errors and precedence |
| Trace and CLI | worker span propagation, log selection, GC configuration, bootstrap/self-update |
| Evidence and delivery | journey/model coverage, large flow-test files, website examples/roadmap, CI/toolchain/release contracts |

**Evidence labels:** “Reproduced” means an actual CLI experiment below;
“Source” means the code path was inspected; “Proposal” is a design
choice requiring discussion. A source-derived failure scenario is not
presented as an observed crash or exploit.

## Recommended priorities

P1 = correctness work before the next feature expansion. P2 = bounded
architecture/evidence improvements, after the P1 behavior is settled.
P3 = demand-driven breadth or low-risk housekeeping. No P0 production
incident was established by this sweep; that is not a guarantee that
none exists.

| Priority | Item | Evidence | Proposed disposition |
|---|---|---|---|
| P1 | GC accepts a nested module store root and collects the actual payload | Reproduced | tighten the persisted store-root boundary before deletion |
| P1 | GC ignores invalid configuration and falls back to another retention policy | Reproduced | propagate config errors, distinguish missing from unreadable |
| P1 | Explicit-step modules lose verification, staging and lint behavior | Reproduced, three paths | one normalized execution view, not three local patches |
| P1 | Cross-module step `needs` is accepted but does not order modules | Reproduced | settle the supported ordering contract; compile it or reject it |
| P1 | Shell outputs/retries/cache declarations do not match execution | Reproduced | enforce real output contracts; choose retry/cache policy explicitly |
| P1 | Warm owned-link preview reports a new deployment while apply is satisfied | Reproduced | share prepared source/destination inputs as well as decision functions |
| P1, carried | Persistence-fault evidence, including EXDEV permission ordering | Source, not crash-reproduced | keep 0025/0035's fault-matrix work ahead of ecosystem breadth |
| P2 | Acquisition and subprocess budgets are not compositional | Source | streaming/acquisition cap + narrow bounded process primitive |
| P2 | Worker events lack module/run span ancestry | Reproduced | propagate causal spans and select logs by the explicit latest pointer |
| P2 | Schema, docs and advertised API contracts drift independently | Source, with runtime counterexamples | executable examples and consumer-facing contract coverage |
| P2 | The lifecycle flow-test file has become a responsibility sink | Measured | split by behavior while preserving the model and assertions |
| P2, carried | Reproducible toolchains and self-update publication discipline | Source/proposal | pin update policy and reuse durable publication principles |

These are recommendations, not an approved roadmap reorder. Existing
reliability work is not displaced by another ecosystem expansion.

## Confirmed runtime findings

### A. Persisted store roots and GC do not agree

**Reproduced.** In an isolated HOME, apply one ordinary owned-file
module. Change only its manifest `store_path` from the published root
to `root/nested`—an invalid module root that is still lexically under
`$GRIPSACK_HOME/store`. `grip gc` exits **0**, collects the original
payload, and leaves the owned link present but dangling.

Observed summary:

```text
case: accepted-nested-store-root-gc
gc rc: 0
original_root_survived: false
owned_link_exists: true
owned_link_resolves: false
```

This is a **corrupt/local-metadata** case, not evidence of remote code
execution or an ordinary valid manifest losing a reference. It matters
because `read_manifest` is explicitly the fail-closed boundary for
long-lived metadata.

Source: `gripsack-store/src/generations.rs::validate` checks for parent
components and a store-root prefix; `gripsack-exec/src/gc.rs::gc`
compares each direct child of the store against exact referenced paths.
The accepted path language is broader than the collector's root
language. `build_closure` paths need the same root invariant.

**Proposal:** require a module/closure store root to be exactly a valid
direct child of the configured store root. Reject root-itself, nested,
relative and parent-traversing forms before planning deletions. Share
that validation with producers/readers. A small validated store-root
type may earn its cost here; do not mechanically newtype every path.

**Acceptance:** corrupted root/closure references fail GC before any
generation, prior or payload deletion. All producer-generated roots
remain accepted. Decide missing-but-well-shaped artifact handling
separately: `store-verify --repair` can intentionally remove an artifact
while its generation remains, so “every referenced path must exist”
is not a substitute for the root-shape rule.

### B. Retention configuration errors are silently discarded

**Reproduced.** Create two generations; set user retention to 1. Make
the repo's `keep_generations` invalid (`"many"` instead of a number).
`grip check` fails with **E400**, but `grip gc` exits **0** and prunes
generation 1 using the user-layer policy. The invalid repo layer was
silently treated as absent.

Source:

- `gripsack/src/commands/gc.rs::user_keep_generations` converts both
  repo/user configuration errors to `None` using `.ok()`.
- `gripsack-config/src/lib.rs::load_user` treats **every** file-read
  error as a missing config, not only `NotFound` (source observation;
  the additional permission-error case was not exercised).

**Proposal:** one fallible config-loading/merging path for commands
that consume these settings. Missing optional files are empty layers;
parse, permission and I/O failures are real errors. No new config
backend or machine-local override system.

**Acceptance:** malformed or unreadable applicable config yields E400
and zero GC mutations; a genuinely absent optional layer still works;
valid repo-over-user precedence remains unchanged.

### C. Explicit-step and declarative modules are not one execution view

Three **reproduced** differences:

1. An explicit-step module with module-level
   `verify: verifyShell("exit 17")` passes `check` and **successfully
   activates**. The declared verifier never runs.
2. `steps: [configStep({ payload: trackedCopy("~/.config/demo") })]`
   with a real repo `payload` passes `check`, then apply fails with
   E301: no payload at the empty-tree store root. The repo source was
   not staged from the step's entry view.
3. A malformed Helix `config.toml` yields A00 with declarative
   `config`, but **check exits 0** for the equivalent `configStep`
   module carrying `lint: "helix"`.

Source:

- `expand.rs::expand_all` passes explicit steps through, while
  `expand` alone synthesizes the module-level verifier.
- `module.rs::ModuleRun::publish` and the config-only branch of
  `identity.rs::resolve` still walk `module.install/config`.
- `gripsack-lint/src/lib.rs::run` discovers only `module.config` paths.
- `resolve.rs::module_input` recurses into dependencies using
  `expand(dep_module)`, not the explicit-aware `steps_of` view. This
  is additional source evidence of the same competing projections;
  a stale-cache case for that exact recursion was not reproduced.

**Proposal:** prepare one immutable, crate-internal normalized module
view after validation. It owns/borrows the effective ordered steps,
source entries and verification contracts; identity, staging, lint,
preview and execution consume projections of **that view**. Do not
populate shadow copies of declarative fields to make old walkers work.
Keep the public DSL's two authoring styles; lower them exactly once.

Module-level `verify` is currently accepted alongside explicit steps.
Recommended behavior is to honor it pre-flip in both styles; rejecting
it would be a deliberate alpha contract change, not a silent skip.

**Acceptance:** paired declarative/explicit fixtures stage identical
payloads, invoke the same lint/verify contracts and make equivalent
destination decisions. A failing declared verifier never commits;
explicit-step source edits and dependency source edits invalidate the
appropriate artifact key. Test effects, not serialization alone.

### D. Cross-module `needs` is a phantom ordering edge

**Reproduced**, without timing assumptions:

```ts
// a-consumer: lexically first, no module-level depends
shellStep('test -f "$HOME/ready"', "consume", {
  needs: ["z-producer:produce"],
})
// z-producer
shellStep('touch "$HOME/ready"', "produce")
```

`check` exits 0. `apply --jobs 1` runs the consumer first and fails
E301. Source: `sema/steps.rs` validates the named reference;
`expand.rs::order_by_needs` considers cross-module refs ready;
`dep_edges`, `build_order` and the scheduler derive module ordering
from `module.depends` only.

**Decision required:**

- Conservative: require a compatible declared module dependency for
  each cross-module step reference, reject unfulfillable phase refs,
  and document that ordering is module-granular.
- More capable: compile a real cross-module step scheduling graph,
  with cycle/phase checks and precise completion semantics.

Do **not** silently turn every step-order edge into a runtime
installation dependency: that would change 0039's deployment roles.
A scheduling constraint and a dependency's purpose are different facts.

**Acceptance:** a consumer cannot run before its referenced producer;
cycles and unsupported activation-phase references fail before
mutation; subset planning includes needed producers without inventing
new HOME effects. Use the Rust graph/model harness for decision logic;
only change protocol models if the transaction/activation protocol
actually changes.

### E. Output, retry and cache policy needs an explicit contract

**Reproduced:**

| Declaration | Observed result |
|---|---|
| `shellStep("true", "produce", { outputs: ["missing-artifact"] })` | check 0, apply 0 despite missing output |
| `runStep(["true"], "produce", { outputs: ["missing-artifact"] })` | control: apply 1 / E301 |
| no-output shell appending to a counter, two applies | one invocation; second apply satisfied |
| explicit `runStep(..., { retries: 2 })`, succeeds on second invocation | one attempt; apply fails |

Source: `module.rs::produce/build_step/run_step` checks structured-run
outputs but discards custom-shell outputs; the phase machine's presence
shortcut skips the entire produce sweep. `Step.retries` and
`Module.retries` roundtrip through types but are not consulted there.
`typescript/src/steps.ts` still says a shell step without outputs
always runs; 0007 §4b specifies retry hierarchy.

**Proposal:**

- Enforce declared output existence consistently for shell/run paths.
- Either implement bounded, explicitly requested retries with typed
  failure classes, or remove/reject unsupported retry declarations.
  Do not blanket-retry hash mismatches or arbitrary non-idempotent
  effects. Keep network retry policy separate from script exit policy.
- Settle caching as a product choice. 0039 legitimately relies on
  warm build-artifact reuse: changing every no-output shell step to
  “always run” is **not** an automatic safe fix. Prefer explicit
  artifact/effect semantics and accurate docs over a hidden heuristic.

**Acceptance:** every accepted output/retry/cache declaration has a
consumer-visible behavior; no field is merely transported. Keep the
cold/warm closure fixture, add missing-output and bounded-attempt
cases after the policy is approved, and retire tests that only prove
that ignored fields survive JSON roundtrips.

### F. The owned-link preview still has different inputs from apply

**Reproduced:** apply a local `payload → ~/.owned` symlink, then plan,
then apply again:

```text
plan:  + payload → ~/.owned (new)
apply: ~/.owned unchanged
       already satisfied (generation 1)
```

Source: `ops/preview.rs`'s owned-entry branch computes a store root
without consistently joining the entry source; `already` and the
planner's source are therefore not the actual deployment target.
Sharing `plan_entry_op` cannot compensate for different inputs.

**Proposal:** share the prepared source descriptor/path construction,
not merely the final mode decision. Cold/fetched unknowns stay explicit
markers; a warm known payload must point to the same concrete file as
apply, including `from` and placeholder expansion.

**Acceptance:** warm owned/config/fetched/multi-entry previews report
satisfaction where apply does, and updates where the produced source
really changes. Extend the materialized op world beyond the current
copy-dominated fixture; never assert arbitrary report wording.

## Source-derived risks and architecture proposals

### G. Persistence evidence remains a first-priority carried item

`gripsack-fs/src/lib.rs::copy_into_dir` writes bytes and calls
`file.sync_all()` **before** applying the copied permissions. The
same-FS publication path changes modes before recursively syncing.
The EXDEV path's post-fsync metadata ordering deserves a dedicated
fault case; no power-loss failure was reproduced in this sweep.

Keep the full kill-point/persistence matrix from 0025/0035 at P1.
Specify the durable primitive contract (bytes, mode, child directory
and parent rename), then instrument failures at each boundary. A
normal successful copy or process-kill test is not evidence for every
power-loss ordering. Reuse existing capability primitives and models;
do not reopen the ownership algebra or introduce a replacement store.

### H. Bounded acquisition and subprocess supervision

Source observations:

- `fetch/tarball.rs::read_body` buffers up to 512 MiB;
  `fetch/archive.rs::decompress` buffers up to 4 GiB. These are real
  per-payload caps, **not unlimited decompression**, but multiplying
  them by module-worker count is not an overall memory budget.
  ZIP extraction is delegated separately and needs its own budget
  accounting. No allocation stress test was run.
- `http.rs::get` constructs an agent and reloads TLS roots on each
  request. Per-run client reuse is a concrete improvement, but must
  preserve proxy/CA/env policy rather than introducing a stale global
  singleton.
- Fetch and lint exchanges duplicate capped-line readers, reader
  threads, deadline accounting and child handling. Their stdout
  queues are unbounded channels; stderr uses `read_line` and checks
  the 64 KiB retained buffer only **after** a line is allocated.
- `fetch/plugin.rs::fetch_exchange` joins stderr after child exit;
  `gripsack-lint/src/exchange.rs::run_exchange` joins it on the
  no-response path. A descendant retaining a pipe is a source-derived
  liveness risk outside the receive deadline. This was **not** tested
  with a long-lived descendant.

**Proposal:** retain the existing fetch-memory roadmap item, with
streaming extraction and acquisition concurrency/budget independent of
module workers. Factor only the repeated process mechanics into a
small bounded primitive if both protocol hosts use it; do not build a
new generic plugin framework. Return typed process outcomes so fetch
and lint retain their deliberately different diagnostic policies.

**Acceptance:** bounded queue/line/total-output behavior, deliberate
stdin/stderr pressure, peer crash, missing response and inherited-pipe
cases terminate within contract. Measure peak memory on bounded local
fixtures. Keep rustls, host-bound tokens and redirect isolation.

### I. Causal tracing is not propagated into scheduler workers

**Reproduced:** two fetched modules emitted two `fetched` JSONL events
with empty span chains and only `message`/`step` fields. The run log
cannot attribute those worker events to module a versus b.

Source: `schedule.rs` spawns worker closures without carrying the
current tracing span; `gripsack-trace` records current span ancestry,
which is thread-local. Grouping CLI reports afterward does not
retroactively annotate events.

**Proposal:** capture the run span at dispatch and enter a module/step
span at execution. Verify real concurrent logs, not just a manually
nested span in the trace crate's unit test. For failure-log selection,
`e2e/conftest.py` should prefer `runs/latest`: names use a seconds
prefix plus random suffix, so lexical order is not execution order
within a second (source observation).

**Acceptance:** each worker event carries correct run/module context;
parallel modules never inherit one another's span; a failing test
prints its failing run's log rather than a same-second predecessor.

### J. Contract docs and schemas need executable checks

Source examples in the live website at release time:

- `doc/modules.md`'s explicit-step example mixes `fetch` with `steps`,
  which E103 rejects.
- Its factory example is missing the `return module(name, {` line.
- The ownership section claims multiple modules may merge the same
  destination, while E111/E119 reject sharing until aggregation lands.
- The site roadmap still lists journey property testing and rollback
  adapters under Next, despite 0038/0037 being shipped in STATUS.
- Current schema v2 still inherits permissive/unknown-field language
  from the original contract while the Rust parser is intentionally
  strict; package/readme descriptions also retain parts of removed
  authoring styles. `griplint`'s loader header still says the checker
  “lands next,” although the checker is shipped.

**Proposal:** make important docs examples evaluable against the real
frontend/core and give each documented field a behavior, not just a
wire-roundtrip check. Align schema structural constraints with the
strict parser, documenting which additional rules belong to sema.
Do not generate a new language or rewrite serde types just to remove
manual files. Keep changelog/STATUS/roadmap roles explicit: history,
landed/deferred decisions, and priority order respectively.

Pack validation can follow the same principle: `RuleValue::Free(String)`
accepts any string, and invalid `_rule` decoding can become free-form
in `checks.rs::section_kind`. No shipped pack counterexample was
established; validate pack-authoring data strictly before treating a
rule as permissive. Preserve intentional tool-config free-form areas.

### K. Test organization is the clearest “monster module” problem

Measured in the released tree:

```text
e2e/test_apply_lifecycle.py                  2307 lines
crates/gripsack-store/src/journal/mod.rs      857 lines
crates/gripsack-fs/src/lib.rs                 705 lines
crates/gripsack-exec/src/module.rs            700 lines
crates/gripsack-fetch/src/fetch/plugin.rs     654 lines
```

The journal total includes a large test section; the filesystem and
module files are below the approximate 800-line convention. Do not
split them blindly by line count. The 2307-line flow file mixes
subsetting, source identity, publication, recovery and regressions and
is the first clear responsibility split.

**Proposal:** split flow tests by observable lifecycle contract, keep
small fixture helpers, and extend the journey world with explicit
step metadata, verification, source-view changes and invalid history.
Keep the existing lineage/TLC models: their value is real, but they
prove behavior for supplied inputs and cannot discover a field that
never reaches the modeled decision function.

### L. Release and self-update improvements, without another rewrite

The 0.35 release pipeline successfully built/verified four platforms,
published crates before GitHub, shipped npm JS/types, and produced
attestations. Preserve those properties.

Two proposals remain grounded in current code:

- Pin a reproducible Rust/Deno/base-image policy with deliberate
  update automation. `rust:alpine` and the native stable toolchain are
  moving inputs despite the Docker-first contract. This folds into
  the existing reproducible-build/external-audit roadmap item; do not
  claim byte-identical releases until an experiment proves it.
- `commands/self_update.rs` uses fixed staging names, copy, a
  best-effort chmod and plain rename, without the durable publication
  discipline used by the store. Specify concurrent-update handling,
  unique staging, permission failure and fsync behavior, then reuse
  appropriate filesystem primitives. No interrupted/concurrent
  self-update failure was reproduced in this sweep.

Automatic provenance verification at install/update time remains the
existing signed-channel roadmap item; a checksum from the same release
channel is not an independent signature. The Node-20 action deprecation
warning in CI is lower-priority housekeeping with an existing
Dependabot PR, not a reason for a broad dependency-upgrade sweep.

## Implementation shape to discuss

Recommended order after approval:

1. **Fail-closed boundaries first:** GC root/config admission, and
   accepted verification/output declarations that currently disappear.
   Preserve minimal reproductions as regressions only when they test
   an agreed observable contract.
2. **One prepared module/graph view:** normalize the two authoring
   shapes once; route identity, sources, lint, verify, preview and
   scheduling through explicit projections. Named inputs and typed
   producers (0036), not more positional arguments or shadow fields.
3. **Settle ambiguous policies:** cross-module step precision,
   artifact-vs-effect caching and retry failure classes. Implement or
   reject unsupported declarations; never silently transport them.
4. **Strengthen evidence:** materialized op worlds, richer seeded
   journeys, durable-boundary fault injection, executable docs and
   real worker-span checks. No new TLA+ model unless a protocol moves.
5. **Then reduce resource/maintenance cost:** acquisition budgets,
   bounded process mechanics, focused test-file splits, reproducible
   toolchains and self-update publication.

This need not be one large PR. Prefer small end-to-end changes that
remove an old path when the replacement is proven. Do not add
compatibility shims to preserve misleading alpha behavior. An IR
version change is acceptable when truly needed and planned first;
most of the normalization above can remain core-internal.

### Keep / do not relitigate

- TypeScript → typed IR → Rust execution, no second frontend.
- Cap-std capabilities and the existing transaction/activation models.
- The shared operation planner; fix its input construction rather
  than creating a separate preview implementation.
- Content-addressed store and generation history; no SQLite or engine
  rewrite proposed.
- rustls-only builds and the two release namespaces.
- 0036's restraint: no universal newtyping campaign; GenerationId's
  recorded revisit trigger has not been established by this sweep.
- 0039's deferred lib/include exports, controlled PATH and general
  env inheritance retain their documented triggers/priorities.

## Questions for the morning

1. Approve the P1 ordering above, or put persistence-fault evidence
   ahead of the explicit-step contract work?
2. Should cross-module `needs` stay module-granular and be validated
   accordingly, or is a true global step DAG worth its added surface?
3. Should shell/run declarations be artifact recipes by default,
   effects by default, or require an explicit cache/effect policy?
   Keep the warm-build use case intact whichever rule we choose.
4. Implement explicit retries now, or reject/remove unsupported fields
   until a complete failure-class/backoff policy is agreed?
5. Prefer a narrow shared process primitive now for the two hosts, or
   first fix their bounds independently and extract after measurement?

This review preserved the 0.35.0 baseline. The owner subsequently
approved all P1s, implemented under [0041](0041-p1-contract-fidelity.md)
with the same end-to-end evidence discipline. P2 proposals remain
separate, prioritized roadmap work rather than implied shipped features.
