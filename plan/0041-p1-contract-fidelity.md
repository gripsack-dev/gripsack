# 0041 — Land the P1 contract and persistence fixes

Status: **implemented for core and TypeScript 0.36.0 (IR v3)**, following
the owner's approval of the P1 findings in 0040.

This plan is recorded before code changes. It implements all seven P1
rows in 0040, then reviews placement/types and restructures where that
removes a real competing representation or oversized responsibility.
The reproduced 0040 failures are the starting evidence, not baseline
checks to rerun merely to confirm the owner's report.

## Decisions

### A/B — GC admission is fallible and fail-closed

A module store root, and every build-closure root, must be exactly a
normal direct child of the configured store directory. Reject the
store directory itself, nested roots, traversal and unrelated paths
before any collection. Share the root rule at construction/reading
boundaries; do not require artifacts to exist because repair can
intentionally remove a referenced artifact before it is rebuilt.

Retention policy loading must distinguish an absent optional file from
parse, permission and I/O failures. Invalid applicable configuration
fails with a diagnostic before GC acquires deletion authority. No
silent `.ok()` fallback to a different retention policy.
The review also reproduced a dangling user-config symlink being treated
as absence. Only a genuinely absent path supplies an empty layer; a
present but unreadable link fails with E400 before collection.

### C — One prepared module view

Introduce a shared IR-owned prepared-module projection: the effective
ordered steps, deployment/source entries, fetch and verification
contracts. Identity, staging, lint, preview and execution consume this
projection instead of rediscovering `module.config/install` versus
explicit steps. Module-level verification runs pre-flip in both forms.
Do not fill shadow declarative fields to keep old walkers working.
Keep the raw IR as the wire/declaration boundary; prepare once per
command where possible and borrow its data downstream.

The initial integration API is `gripsack_ir::prepared::PreparedModule`,
with `new(&Module) -> Result<Self, Diagnostic>`, `steps()`, `entries()`,
`config_entries()`, `fetch()` and `checks()` projections. Execution's
`expand_all` becomes fallible and returns prepared modules. Construction
must reject cycles, not append an unexecutable remainder.

### D — Cross-module needs order complete modules, not deployments

Keep the existing module-granular executor rather than invent a global
step VM. Compile cross-module `needs` into **scheduling-only** module
edges, used consistently by order, waves, subsets and readiness. They
do not change runtime/build purposes or add PATH exports.

Validate target steps and reject unfulfillable activation references,
self-qualified references (use a sibling id), and union-graph cycles
before mutation. `module:done` means that module's pre-activation work
is complete; activation remains behind the single generation flip.
This is stronger ordering than a fine-grained step DAG, explicitly
stated rather than silently ignored.

### E — Artifact cache policy and removal of inert retries

Build, custom-shell and structured run steps are **artifact recipes**.
Their successful store result is cached by the prepared recipe, source
inputs and dependency pins. Outputs, when declared, are postconditions
and must exist for both shell and run actions. No-output does not mean
"always run". Effects that should happen at activation belong in
activation hooks. This preserves the legitimate warm-build closure
contract without adding a speculative cache-policy field.

Remove the nonfunctional module/step `retries` fields rather than add a
partially classified retry engine for arbitrary effects. Typed callers
must update; untyped/IR callers receive an error, never an ignored
count. A complete transport retry/failure-class design remains a
separate future decision.

This is a clean **IR v3** cutover: retain historical v1/v2 schemas,
emit/accept v3 only, remove retry fields on all three sides, regenerate
the golden corpus, and update pinned-frontend guidance. No aliases or
fallback readers. Lockfile and generation formats remain readable.

### F — Preview shares source construction with execution

Use the same prepared source descriptor and placeholder substitution
for deployment and preview. A known warm artifact uses its actual store
root plus the entry's relative source; a cold or unknowable artifact
remains deferred. Cover owned/config/fetched/multi-entry cases in the
materialized op harness and real CLI journeys. The shared decision
functions remain; remove duplicate input derivation.

### G — Persistence boundaries: real faults plus ordering evidence

Correct EXDEV publication so final permission changes precede the file
fsync that makes them durable. Specify bytes, mode, child-directory and
parent-publication ordering for each filesystem primitive.

Add bounded, debug/test-only instrumentation at the shipped mutation
and durable boundaries, and an exhaustive reachable-boundary matrix
for apply/deploy, apply/prune, rollback/deploy, rollback/prune,
generation publication/flip and activation-record publication/cleanup.
Each scenario first records its boundary sequence; every reachable cut
is exercised with fresh isolated state, not a sampled hard-coded list.
The oracle is previous state, committed target, or explicitly preserved
user drift, followed by successful recovery with correct contents/modes.

Abrupt process termination and injected I/O failure prove those paths, not
all physical power losses. Separately drive an abstract persistence
ordering check from the shipped primitives' operation trace, including
permissions and fsync, and calibrate it with the old incorrect ordering.
Keep the existing ownership/transaction/activation models; add no new
protocol unless implementation actually changes a protocol. Document
what the real-fault matrix and ordering model each prove.

The process-loss cut uses SIGKILL, not SIGABRT: thousands of deliberate
crashes must not invoke a host's diagnostic core-dump service. The matrix
asserts the actual SIGKILL return status. This hardens the harness after a
Linux hosted runner lost communication during CI; that runner's missing
log does not establish the resource failure's cause. No cuts are removed.

### Persistence-matrix finding — record the mode actually written

The private-file matrix exposed an additional producer defect: takeover
wrote mode 0600 but recorded the nominal 0644 content/mode hash, so the
next update appeared to be user drift. Fix the produced identity, not
the lineage algebra. Distinguish source-executability policy from exact
rollback modes with a typed input. Record the source executable bit in
the entry (optional for old manifests), preserve acquired permission
bits on content-only updates, and adjust execute bits only when the
source's executable state actually changes. Hash the actual landed mode;
store verification must use that recorded mode as well.

Apply preserves template permissions; rollback honors the recorded exact
mode, including same-content/mode-only restoration (0031 §2/§5). Such
a restoration still requires intact written lineage, never a drift
observation. Satisfied receipts record the mode execution actually left.

## Modularity and representation review after P1 behavior works

Inspect all affected boundaries and the remaining project for duplicate
projections, misplaced helpers and ambiguous domains. Prefer cohesive
source/prepare/process/persistence modules; keep production modules
roughly below 800 lines without splitting by line count alone. The
2307-line lifecycle test file is a clear candidate for responsibility
splits. Reuse 0036's rule: typed producers, plain wire, explicit seams;
no indiscriminate newtype campaign or GenerationId without its trigger.

Repair documentation/examples and schema drift affected by these
contracts. Broader acquisition budgets, process-host consolidation,
installer provenance and ecosystem proposals remain their own roadmap
items unless a concrete restructuring prerequisite requires them.
Record any newly observed unresolved item with priority and evidence;
do not silently defer a P1 acceptance criterion.

### Review disposition

- Split lifecycle flows by destination lineage, execution contracts,
  activation, preview, transaction recovery and generation history.
  Preserve the test bodies and collection; only ownership/imports move.
- Move acquisition/build/run methods and pin construction into
  `module/produce.rs`, beside the existing `module/verify.rs`. The parent
  keeps lifecycle state, publication, deployment and outcome construction.
- Extract inline filesystem/journal unit suites into `tests.rs`. Keep
  the production journal and capability primitives together; they already
  have coherent protocol ownership, not a need for another storage layer.
- Keep raw IR admission separate from the owned prepared execution view.
  Keep module-granular scheduling, typed hash producers and `WritePermissions`;
  opaque wire pin strings and declaration spellings do not need ceremonial
  wrappers. No new public aliases or compatibility paths.
- Existing P2 acquisition/process bounds, causal worker spans, executable
  documentation coverage and reproducible update/toolchain policy remain
  explicit roadmap items. This review does not claim a security proof or
  physically simulated power loss.

## Evidence and shipment

- Keep the concrete P1 reproductions as behavioral regressions where
  they guard plausible failures; remove tests that only copy ignored
  fields or pin wording.
- Real CLI smoke: invalid GC inputs do not mutate; explicit and data
  modules agree; failing verification/outputs do not commit; cross
  needs order producers; warm plan agrees with satisfied apply.
- Full persistence boundary matrix plus ordering calibration.
- All four `docker compose run --build --rm` gates: test, ts-test, e2e,
  model. Native macOS CI and example-environment canary must pass.
- Update STATUS, changelog and the website's current contracts and
  correctly prioritized roadmap. Release core and TS together, verify
  crates.io/npm and all platform artifacts. Alpha breaks are documented
  and planned, not papered over with compatibility shims.
