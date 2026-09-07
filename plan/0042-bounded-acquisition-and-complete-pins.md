# 0042 — Bounded acquisition, complete pins, and stronger evidence

Status: **implemented for core/TS 0.37.0**; integrated release verification is
recorded below. IR v3 and existing persisted state remain readable.

The owner requested the P2s from 0040/current roadmap, the carried 0024
one-commit release-module item (including pixi), and tighter Rust/TLA+
models, with complete implementation and verified publication. This plan
records the contracts before source changes. 0041's completed module/test
splits stay; the remaining journey/contract/fuzz coverage is included.

## Scope and boundaries

Included: bounded acquisition and process supervision; HTTP/graph reuse;
causal logs; executable docs/schema/pack admission; persistent fuzz targets;
journey expansion; pinned toolchain/update automation; durable concurrent
self-update; complete update-time pins; stronger transaction/recovery models
and focused new publication/supervision models.

Not silently included: a new frontend, global step VM, arbitrary script
retries, signed update-channel design, new package ecosystems, hermetic
builds, an external audit, or changes to the ownership algebra. Existing
P2 breadth remains distinct from those product/trust decisions. IR v3 is
retained if only enforcing its already-shipped Rust acceptance surface;
a real wire/semantic break changes all three IR sides and bumps the version.
No public compatibility aliases or parallel legacy execution paths.

## A — Bounded, command-owned acquisition

Replace per-request HTTP construction and whole-archive RAM buffers with an
explicit `gripsack_fetch::FetchContext`. A context owns reusable direct/proxy
agents, the captured network policy, limits, and an acquisition semaphore.
It is shared across a command's module workers, never a process-global
client singleton. Trusted tool provisioning uses a context made BEFORE
repo build-env injection; artifact acquisition uses a fresh context AFTER
injection, so repo SSL/proxy settings cannot alter runtime selection or be
silently ignored by a stale client.

Default limits retain 512 MiB compressed and 4 GiB expanded per payload,
with explicit finite archive-entry/decoder-memory limits and a small
acquisition concurrency cap independent of `--jobs`. Settings are fallible,
positive, layered through the existing config merge. No silent clipping.
Measure peak memory on bounded local compressed fixtures, including pressure
from concurrent modules. Network/decoder memory does not scale with entire
payload size. Disk spools are private, unique and RAII-cleaned.

Download to a bounded seekable spool while hashing; verify the complete
transport digest BEFORE extraction. Stream gzip/xz/tar and bare content,
retain executable-bit semantics, and bound ZIP metadata and expanded bytes
as well as decoder working memory. Reject traversal/link escape, unsupported
entries, over-limit streams and malformed archives before publication.
Checks cover actual emitted bytes, not just untrusted size headers. No
partial payload becomes a published artifact. Preserve host-bound auth,
API asset endpoints, proxy bypass, system CA policy and redirect isolation.

The public fetch integration contract is:

- `FetchContext::new(FetchLimits) -> Self` (network agents lazy; no I/O merely
  constructing a context), shared as `Arc<FetchContext>` in execution.
  `FetchContext::artifacts(limits, provisioning)` captures artifact policy while
  retaining the earlier provisioning context for lazily installed trusted tools.
- `fetch(&self, spec: &FetchSpec, dest: &Path, locked: Option<&Value>)
  -> Result<FetchOutcome, FetchError>`.
- `payload_hash(&self, spec) -> Result<Option<FetchIdentity>, FetchError>`.
- Methods for existing `resolve_latest`, `resolve_brew`, `resolve_self_release`
  operations, with their existing arguments/results. Internal resolvers and
  provisioning use the same context explicitly. Git HEAD resolution may
  remain an explicit non-HTTP helper.
- `FetchOutcome { identity: FetchIdentity, url: Option<String>,
  version: Option<String> }`.
- `FetchIdentity::Download(DownloadHash)` versus `Tree(PayloadHash)`:
  transport SHA-256 is not canonical tree identity. `DownloadHash` has
  private representation, hashing/parsing producers and `as_str`; existing
  canonical store hash types are reused for trees. Wire lock hashes remain
  strings at the explicit boundary. No raw/canonical hash confusion.
- The plugin adapter takes `fetch(name, args, dest, locked, limits)` internally and
  returns `PluginFetch { tree: PayloadHash, url, version }`, already computed
  by core. No second tree walk just to recover a typed value.

## B — One bounded process mechanism, two protocol policies

Introduce `gripsack-process`, used by fetch, capabilities and lint hosts.
It owns a child process group, nonblocking stdin/stdout/stderr, bounded line
and total-output accounting, a fixed retained stderr tail and one deadline.
No unbounded MPSC queues, `read_line` allocations, detached reader threads,
or joins waiting for an inherited pipe. Kill the owned process group on
failure/completion as necessary, reap the leader, and close local pipe ends.
Guard against PID reuse and report cleanup failures honestly. Kernel
scheduling/SIGKILL progress is an explicit OS assumption, not a physical
or hostile-kernel guarantee.

Shared API contract: `run(&mut Command, input: &[u8], Limits,
FnMut(&[u8]) -> Control) -> io::Result<Outcome>`. `Control` is `Continue` or
`Response`. `Outcome` contains `status: Option<ExitStatus>`, typed stop reason
and bounded stderr bytes. `Limits` includes timeout, input/line/total stdout/
total stderr bounds and retained stderr size; defaults are finite. Protocol
hosts retain their existing deadlines (capabilities 30s, lint 120s,
fetch 600s). Input is one NDJSON request, closed when fully sent. Callback
processing is synchronous with bounded lines; total output and diagnostic
counts prevent a parsed-message accumulation loophole.

The mechanism never declares protocol success. Fetch still requires a
response, successful child exit and no error diagnostic, then computes the
tree itself. Lint keeps its diagnostic policy, including nonzero exits
with valid lint diagnostics, but deadline/output/pipe failures cannot be
hidden merely because a response arrived. Capabilities failure means no
capability declaration, not a permanent live child. Tests use real peers:
stdin pressure, long/no-newline stderr, flooding, crashes, no response,
response-then-linger and descendants retaining pipes.

## C — Complete update-time pins without deployment

One commit refers to the user's lockfile diff, not automatic git commits.
`update` already downloads many payloads just to hash them. It will resolve,
acquire, verify and finalize pins in one pass, writing the lock once after
all selected modules succeed. Failures preserve the prior lock byte-for-byte.
Unique staging replaces fixed `.update-<name>` scratch names.

For source-only modules, stage the SAME repo overlay as apply, compute the
merged `tree256` and `repo256`, and publish the verified content-addressed
artifact as a cache. No destinations, generations or activation hooks are
created by update. GC may evict that cache; a cold pinned apply reconstructs
it without changing the lock. Build recipes are NOT executed by update;
their pins describe source identity, not unknowable build output, and apply
must not erase/rewrite already-finalized source metadata.

Share source resolution, overlay staging and pin construction with apply;
remove the competing per-kind update pin builders. Cover file, tarball,
GitHub releases, brew bottles, git (fixed and floating refs), pixi and plugins.
Pinned declarations are respected, not upgraded behind the author's back.
Preview stays offline. Unlocked apply retains deliberate first-use pinning.

Pixi records the actual resolved primary-package version from its metadata,
uses that version on pinned reconstruction, and compares core-harvested tree
identity in the same domain in update and apply. Conda bookkeeping remains
excluded. Do not fabricate a version or treat archive SHA-256 as a harvested
payload hash. Real pixi/local-channel smoke complements isolated peer fixtures.

Acceptance: after update, the first warm AND cold apply leave lock bytes
unchanged; repeated update is stable; overlays, version substitutions,
metadata-only completion, dependency pins, failures and subset scope are
covered. Build-only modules remain undeployed and never run hooks here.

## D — Causal tracing and graph reuse

Capture the run span before spawning workers; enter the matching module
and step spans while their actual work executes. No cross-worker context
leakage. Failure-log selection follows the explicit latest pointer, with
validated fallback behavior for a missing/broken pointer, not same-second
lexical filename ordering. Prove ancestry using real concurrent worker logs.

Prepare/reuse graph projections and memoized dependency recipe identities
within a command; invalidate memoized pin-dependent results when a freshly
resolved dependency changes. Avoid repeating full IR clones/tree hashes for
every consumer while preserving provenance exclusion and transitive pin
invalidation. Do not introduce a stale global identity cache.

## E — Executable contracts and persistent fuzzing

Important published TypeScript examples are extracted from the website
source and evaluated against the actual packaged frontend/core with offline
fixtures. No manually-maintained shadow copy of examples. Core and website
CI use this check; partial illustrative fragments must be classified
explicitly rather than silently treated as successful executable examples.
Keep behavior-level coverage for supported fields and remove wiring-only,
incidental-default or wording-pinning tests encountered in the affected area.

Align current JSON Schema structural acceptance with the strict Rust parser;
sema remains responsible for graph/path/phase relationships. Tighten pack
admission: only declared free markers mean free-form, malformed `_rule`
content cannot degrade to permissive behavior, and valid tool-config free
areas keep their semantics. Use typed internal pack variants rather than
re-decoding arbitrary marker maps during every rule walk.

Persistent cargo-fuzz targets and seed corpora cover manifest admission,
merge parsing, bounded archives, journal/recovery and GC-facing metadata.
Deterministic CI replay/smoke has explicit limits; longer coverage-guided
runs are scheduled. Fuzz state and crash artifacts stay out of commits.
  Fuzz targets share the production workspace lockfile and exact compiler pin;
  a separate resolver must not silently select a different archive/parser library.
All mutation-capable targets pin targets inside their sandbox: arbitrary
fuzz bytes must never become a host destination. Add journey worlds for
explicit-step contracts, verification, source changes and invalid retained
history, with reproducible seeds and observable state oracles.

## F — Reproducible inputs and durable self-update

Pin an exact Rust toolchain for containers/native jobs, Deno, multi-platform
base-image digests and release tooling. Reuse existing update automation
patterns and propose deliberate pin-update PRs, not silent floating upgrades.
Record an actual two-clean-build comparison on a release target; state
exactly which inputs were held fixed and whether binaries matched. An
external audit and an unconditional cross-time/cross-platform reproducibility
claim are not implied by pinning a compiler.

Self-update uses a per-executable coordination lock, unique RAII staging,
strict payload selection, checked permissions and capability-relative atomic
publication. Re-check the installed version under the lock so a process
started before another update cannot downgrade the executable. Delegated
brew/cargo/mise channels and check-only behavior remain. No auto rollback
following a committed namespace swap; report post-publication durability
errors as such, never pretend the old executable is still installed.

Shared filesystem extension for a streamed executable write:
`atomic_copy_with_mode(dir: &Dir, name: &Path, source: &mut impl Read,
mode: u32) -> io::Result<()>`, following write → mode → file fsync → rename
→ directory fsync. Its actual operation trace feeds the ordering model.
No whole-binary buffer is required. Automatic signed-channel verification
remains its separate roadmap trust decision.

## G — Tighter models, calibrated against real failures

Strengthen the Rust explorers and TLA+ together: complete type invariants,
explicit durability assumptions, multiple independently journaled destinations,
partial cleanup, crashes during recovery, preserved edits and bounded fair
recovery progress. Keep shipped decision functions as the Rust oracle seam.
The TLA+ model remains an independent expression, not generated Rust tests.

Add focused bounded-supervisor and concurrent-publication models where the
new protocol is not covered by existing transaction/primitive models. State
time/OS fairness assumptions. Avoid proving a vacuous liveness property by
making the failure transition impossible. Calibrate with executable negative
configs/mutants: premature publication/cleanup, lost pending activation,
ignored deadlines/inherited pipes and unsafe overwrite. A negative model
must fail for the intended invariant/temporal property, not parser errors.

The model runner is `scripts/check_models.sh <tla2tools.jar>`; Docker's model
gate invokes it, including positive and deliberately failing configs. Fuzz
and toolchain owners keep their additions compatible with that runner.
No committed TLC state directories or counterexample debris.

## Ownership during implementation

Main integrates command contexts, complete pins, graph identity caching,
versions, docs/publication and final gates. Independent owners may edit:
fetch acquisition/context internals (except plugin host); process crate and
plugin/lint adapters; tracing/journeys; schema/pack admission; models/runner;
toolchain/self-update/atomic copy; fuzz infrastructure; executable docs.
Root Cargo dependencies and compose integration belong to Main. Existing
Cargo manifests are coordinated, not simultaneously edited. Production phase
signatures are kept stable by tracing; Main edits their acquisition calls
only after that owner hands them back. Every parallel owner skips all
validation/formatting; one integrated final verification follows.

## Delivery evidence

Real CLI and packaged SDK smoke; adverse local transport/process fixtures;
peak-memory measurement; one-commit warm/cold/failed update flows; concurrent
self-update and fault paths; executable docs; corpus replay; positive and
negative model checks. All four compose gates, new bounded fuzz/repro gates,
native macOS CI and the external environment canary must pass. Update STATUS,
changelog, website contracts and priorities. Publish and verify core and TS
artifacts, registry packages, checksums and provenance before declaring done.

### Recorded integration evidence

- The Linux debug CLI was measured with 16 MiB and 256 MiB expanded payloads,
  then four 128 MiB modules at acquisition concurrency two. Sampled core
  `VmHWM` was 20,940 / 20,804 / 21,620 KiB; Linux `wait4` maximum-process RSS
  (including waited descendants, not their sum) stayed around 45 MiB. The
  loopback server observed exactly two concurrent requests in the four-module
  run. These are measurements, not a universal OS/native-tool memory guarantee.
- Real provisioned Pixi 0.77.1 consumed an isolated local conda channel at
  versions 1.0.0 and 2.0.0. Update, first warm apply, scoped cold reconstruction
  and repeated update kept lock bytes stable for each version. The recorded
  primary version and core-harvested digest agreed; no mock Pixi was used.
- The candidate npm tarball passed eight extracted executable website examples
  through both the packaged frontend and real CLI. An installed-package canary
  proves the package is selected, and missing-return factory mutations fail.
  One genuinely partial linter illustration is explicitly classified.
- The canonical schema is carried into the `gripsack-ir` package via its
  source symlink; `cargo package` verified the package and its schema bytes
  match the canonical file. Acceptance uses the standard JSON Schema validator,
  including pattern assertions, not a parallel subset interpreter.
- TLC checked 28 configurations: 14 positive protocols and 14 deliberately
  failing, named counterexamples. Repeated activation models physical generation
  records rather than assuming ghost invocation IDs exist on disk.
- Integration retained two safety distinctions: GC cannot evict generation-rooted
  artifacts merely to simulate a cold store, and merge removal preserves internal
  unowned separator lines. Over-strong prototype oracles were corrected rather
  than weakening either production contract.
