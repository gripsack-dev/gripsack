# 0048 — Unified review response: boundary fixes and release assurance

Status: **implementation handoff — the changes proposed here have not
shipped.** This is the single backlog for the local 0.42.0 review and
the external fresh-eyes review dated 2026-09-10. Implement the combined
milestones in §9, using the original packets in §§1–8 and additions in
§13; do not execute the two reviews as separate projects.

- Baseline: commit `176eaec`, workspace/core/SDK 0.42.0, IR v3.
- **Evidence labels:** **Demonstrated** = observed on the real 0.42.0
  binary in a disposable, offline container (dummy tokens, throwaway
  HOMEs, no untrusted network). **Source** = confirmed by reading the
  code path end-to-end. **Docs** = text contradicts shipped behavior.
- Review method: seven parallel read-only slices (recovery, execution,
  boundary/security, formal verification, delivery/CI, contracts,
  ecosystem) plus owner-run probes. No production code was edited.
- Integration review: 2026-09-11, local HEAD
  `176eaecd85a95b4f29ed49293a889c7bf16cc8c8` (same baseline).
  The external review was supplied as pasted text; its linked 30-page
  DOCX was not available as a local attachment and was not inspected.
  Additional findings below are source assessments, not newly executed
  vulnerability probes. The baseline gate results are historical,
  inherited from the original local review, not rerun for this handoff.
- Additional labels: **Existing** = implementation/evidence already
  present; **Gap** = missing contract or coverage, not an exploit claim;
  **Proposal** = new product behavior; **External** = registry, hosting,
  signing, hardware, or owner action that code alone cannot establish.
- **Precedence:** §9 is the binding delivery/deferral/release contract;
  §10 is mandatory engineering acceptance; §6 defines required proof
  work; §13 resolves F-01–F-16. Original finding numbers remain intact.
- Owner follow-up: required core verification must land before release;
  only named later/exploratory/claim-gated work may wait. A blocked proof
  blocks release unless the owner explicitly changes that requirement.
  Readability, semantic types and cohesive modules/crates are deliverables,
  not optional cleanup. These directives supersede older loose deferrals.

## Baseline gates (all green at review time)

| Gate | Result |
|---|---|
| `test` (fmt + clippy -D warnings + cargo test + embed freshness) | pass |
| `ts-test` (deno test on frontend) | pass |
| `e2e` | 245 passed in 863s |
| `model` (TLC, positive + calibrated negatives, attribution-bound) | pass |
| `verify` (Verus: 56 obligations, 0 errors; 4 mutants rejected) | pass |

The gates being green and the findings below coexisting is the point of
this plan: the failures live at admission boundaries the current
evidence does not exercise (sandbox grant construction, repo-controlled
environment, archive link graphs, tampered persisted metadata), not in
the proved kernels.

## Priorities

P0 = correctness at trust/security boundaries; blocks the next release.
P1 = contract fidelity and evidence integrity. P2 = hardening, docs,
ecosystem cutover. The original demonstrated issues remain release
blockers: a public-source review reporting no confirmed critical flaw
does not negate a local reproduction. Threats include modified trusted
repositories, crafted upstream artifacts, and corrupt/tampered local
state. This is not a claim of an observed attack or compromise.

Release class is authoritative: a P2 item assigned NEXT still must land;
a theoretical/experimental label cannot demote a required §6 proof.

| # | Priority | Finding | Evidence |
|---|---|---|---|
| 1 | P0 | Deno `--allow-read` grant injection via comma in pinned frontend path | Demonstrated |
| 2 | P0 | `[eval.env]` is process-global before eval: repo controls facts/probe environment and credential audiences | Demonstrated (PATH, GH_HOST) |
| 3 | P0 | TAR and ZIP archives extract composed symlinks that escape the payload root | Demonstrated |
| 4 | P0 | `--host`/`default_host` are unvalidated; lockfile writes escape `locks/` (in-repo `..` and absolute forms) | Source (witnesses named) |
| 5 | P0 | A panic inside module execution deadlocks the apply and holds the lifecycle lock forever | Source |
| 6 | P0 | Manifest `prior` fields bypass `read_manifest` validation → tampered manifest turns rollback/prune into an arbitrary-read→destination-write gadget | Source |
| 7 | P0 | Release pipeline publishes out of dependency order (ir before policy) and pushes the Homebrew cask before the GitHub release it references exists | Source |
| 8 | P1 | `update` vs `update --check` compute "unchanged" differently; "unchanged" updates still rewrite lock bytes | Source |
| 9 | P1 | Verify-step actions escape E109/E115 path admission; E110 is vacuous for stepped modules | Source |
| 10 | P1 | Ledger says the merge splice wrapper "fails closed, never panics"; it `.expect()`s | Source/Docs |
| 11 | P1 | Verus mutant calibration accepts *any* "not satisfied" line, not the named contract; global obligation floor only | Source |
| 12 | P1 | `Transaction.tla` retains the removed op-ordering classify heuristic (unreachable, stale mirror) | Source |
| 13 | P1 | Plugin cache validates receipt tag but not source; throttle admits NaN/inf/<1 rates (NaN panics); adopt touches hosts file before the trust gate; eval output is unbounded | Source |
| 14 | P1 | `resources` on Verify/Intent steps are accepted but never acquired | Source |
| 15 | P2 | Schema/DSL parity gaps: `entry.from` minLength, inert `vars`/`marker` (E108 never emitted), env `append` unauthorable | Source |
| 16 | P2 | CI composition: only `test` required; no aggregator, top-level permissions, or concurrency; audit/fuzz failures leave no evidence; demo lane unpinned; npm publish lacks provenance | Source |
| 17 | P2 | Recovery ergonomics: staging residue never collected; journal/quarantine world-readable; unrestorable-entry wedge has no escape hatch | Source |
| 18 | P2 | Ecosystem: local tap at 0.2.1 without generated Cask, dotfiles pinned to 0.17.5, obsolete frontend metadata, duplicated websites with 0.39 example-test pins | Source (local snapshots, not live registry state) |

---

## 1. Evaluator boundary (P0) — demonstrated

These are the highest-severity findings. The Deno permission model is
the architecture (0013 D2); all three defects are in how the core
*constructs the grants and the process environment*, outside anything
the kernels or models cover.

### 1.1 Comma-in-path read-grant injection — Demonstrated

`crates/gripsack/src/commands/frontend.rs:51-55,69` builds
`--allow-read=<a>,<b>,…` by joining canonical paths with `,`. A
repo-created pin directory named `repo/pin,/outside/parent` containing a
`package.json` with `{"name":"@gripsack/core"}` passes the
`pin_is_gripsack_core` check (frontend.rs:83-89) and is pushed into the
grant list — Deno parses the comma as a separator, granting the outside
parent.

Probe (offline container, disposable HOME): control with no pin →
outside read denied (`NotCapable`). With the comma pin → `grip check`
exit 0 and the module reads the outside canary. Confirmed on 0.42.0.

**Fix:** forbid path-list separators in granted paths at the admission
seam: `pin_is_gripsack_core` (and the repo/inputs/frontend paths before
spawn) must reject any path whose display contains `,`. One validation
function shared by both. Fail closed with a diagnostic naming the pin.

**Acceptance:** the probe above fails with a named error post-fix;
`typescript/test/driver.test.ts` pin tests stay green; e2e gains a
regression using a comma pin.

### 1.2 `[eval.env]` is process-global before evaluation — Demonstrated

`crates/gripsack/src/commands/eval.rs:145-147` injects every accepted
`[eval.env]` entry into the gripsack process's own environment before
facts detection, Deno spawn, and artifact `FetchContext` capture.
`gripsack-config` rejects only `GRIPSACK_*` names. Consequences:

- **PATH** redirects the unqualified `ldd` exec in
  `crates/gripsack-exec/src/facts.rs:53-59` — with an operator
  `GRIPSACK_DENO` override (facts `LazyLock` not yet initialized), a
  repo-supplied `ldd` runs during `grip check`. Demonstrated: harmless
  shim wrote a marker outside the repo; check exited 0.
- **GH_HOST** retargets an inherited enterprise token:
  `crates/gripsack-fetch/src/http.rs:47-52` captures the host binding
  from env at artifact-context creation (after injection,
  `context.rs:35-39`). Demonstrated with a dummy token on loopback:
  operator `GH_HOST=trusted.example.invalid` → no Authorization sent;
  repo `GH_HOST=127.0.0.1` alone → fake `Bearer` transmitted.
- Loader variables (`LD_PRELOAD`, `DYLD_*`) affect a dynamically linked
  evaluator before Deno permissions apply (source-confirmed; not
  separately demonstrated).
- HTTP additionally matches bearer by host only
  (`http.rs:56-65`): an `http://` URL on a bound host receives the
  header in cleartext. The e2e loopback fixtures deliberately use HTTP;
  that is the test seam, not the production posture.

**Fix (split into named scopes; do not keep one global injection):**

1. Facts must be computed from an environment the repo cannot touch:
   qualify the detector (`/usr/bin/ldd` fallbacks or a direct
   `gnu_get_libc_version` call) and/or detect facts **before** any
   repo-configured injection.
2. Credential *audience* is operator-owned: `GH_HOST`/`GITHUB_HOST` and
   the token names must be read once at process start (or excluded from
   `[eval.env]`); a repo may never rebind an ambient token. Reject
   credential-audience variables in `[eval.env]` with a named
   diagnostic. Keep build-time variables (proxy, CA, SSL_CERT_FILE)
   working — scope them to build/fetch child processes via explicit
   `Command::env` maps instead of process-global `set_var`.
3. Bearer headers only over HTTPS; fail closed otherwise. Move the
   dummy-token HTTP fixtures to local TLS or an explicitly test-only
   transport seam. Do not ship a loopback exception that can transmit
   real credentials to a repo-selected local service.

**Acceptance:** all three demonstrated probes fail closed post-fix;
the existing `test_eval_env_reaches_build_steps` e2e stays green (build
steps still see declared env); the credential-routing TLA+ model gains
a real-code bridge for the audience rule.

### 1.3 Host identifiers are unvalidated — Source

`--host` and env.toml `[env] default_host` reach
`gripsack-exec/src/lockfile.rs:58` (`locks/{host}.lock` write),
`gripsack-lint/src/versions.rs:10`, `adopt/mod.rs:169-171`, and the
driver lookup with no shape check (only the auto-detected hostname is
sanitized, `crates/gripsack/src/commands/mod.rs:78`). Witnesses
(repo-controlled, behind the trust gate):

- `default_host = "../modules/foo"` + shipped `modules/foo.ts` → eval
  succeeds, lock write lands at `repo/modules/foo.lock`.
- `default_host = "/tmp/x"` + shipped `hosts/tmp/x.ts` → Node `join`
  keeps the base (in-grant read), Rust `Path::join` replaces it → an
  arbitrary absolute path is overwritten, always with the `.lock`
  suffix and lockfile-JSON content.

**Fix:** one admission function (`host_name_ok`, mirroring
`module_name_ok`/E116) applied at eval.rs:113-115 and adopt: reject
absolute hosts, empty, and any `/`, `\`, or `..` segment. New E-code.

**Acceptance:** both witnesses fail at admission with the new code;
normal role-named hosts unaffected.

### 1.4 Boundary supervision gaps — Source

- Eval reads child output with raw `.output()`
  (`crates/gripsack/src/commands/probe.rs:122`): a runaway or hostile
  pinned frontend produces unbounded stdout/stderr into memory. Route
  through `gripsack-process` bounded supervision (the same primitive
  0042 gave fetch plugins), or cap the buffers.
- Adopt reads/writes `hosts/testhost.ts` before the trust gate
  (`adopt/mod.rs:173-180` vs gate at ~198). Reorder: gate first, or
  document why adopt's file access is exempt.

## 2. Archive containment (P0) — demonstrated

### 2.1 Composed symlinks escape the payload root — Demonstrated

`crates/gripsack-fetch/src/fetch/archive/paths.rs:27-46` validates link
targets *lexically*; the composition of two archive links is never
resolved. Fixture: directory `d`; symlink `d/up → ..`; symlink
`leak → d/up/../sentinel`. Lexically the second target resolves to
`d/sentinel` (inside); the kernel resolves through `d/up` to the
sibling of the extraction root.

Probe (standalone caller linked against the current
`libgripsack_fetch`, offline): **both TAR and ZIP** return
`Ok(FetchOutcome …)`; `out/leak` resolves to the outside sentinel and
reads its contents. The checks at tar.rs:95-116 (ancestors vs symlink
set) guard *content paths*, not link-target resolution through
previously created links; `tree.rs:63-84` (`validate_tree`) accepts any
symlink; `deploy/mod.rs` reads sources with following semantics
(`fs::read`, preflight metadata), so the escaped link is consumable
downstream.

Scope honesty: this is a containment failure of the payload tree (the
store path can hold a link pointing anywhere), not a demonstrated
arbitrary-write primitive.

**Fix:** validate the complete admitted link graph, then materialize it
through root-pinned directory capabilities. Resolve relative targets
through other links with bounded traversal; reject escape, cycles and
unresolvable/ambiguous targets. Share the resolver between tar, zip,
`validate_tree` and `copy_tree_filtered`. Validity must not depend on
archive member order. A canonicalize-at-creation check is insufficient
for forward links and later replacement; do not use it as the fix.

**Acceptance:** the demonstrated TAR and ZIP fixtures fail with
`UnsafeArchive` post-fix; existing archive fuzz corpus stays green;
a regression fixture for both formats lands in the fuzz corpus.

### 2.2 Plugin identity and throttle admission — Source

- `plugins.rs:95-99`: cache hit checks `receipt.tag` but ignores
  `receipt.source` — the same alias/tag from a different origin runs
  the previously provisioned plugin. Compare the full ref.
- `throttle.rs:25-40`: rates accept NaN/inf and capacities < 1; NaN
  panics at `Duration::from_secs_f64` (~:69); a 0.5/s rate can never
  accumulate one token. Validate with `is_finite()` and a minimum
  quantum at parse; add a panic regression.
- `plugins.rs` `acquire_declared` (~:282) waits without a deadline
  before the process timeout starts — a stalled plugin parks the
  acquisition forever. Bound the wait with the supervision deadline.

## 3. Scheduler and update semantics (P0/P1) — source

### 3.1 Panic in a worker deadlocks the apply — Source, P0

`crates/gripsack-exec/src/schedule.rs:124-238`: the worker body runs
`run_one` outside any `catch_unwind`; the completion bookkeeping
(`running.remove`, `kernel.finish_*`, `condvar.notify_all`) executes
only on normal return. A panic unwinds the scoped thread; every other
worker parks on the condvar with `st.running` never empty;
`thread::scope` blocks joining them; the lifecycle flock is held by the
hung process. The proved latch never engages because the bridge never
calls it — this is precisely the unproved bridge the ledger names, and
the one failure mode that converts a bug into a hang instead of an
error. No `panic = "abort"` profile is set. Today's in-path `expect`s
appear infallible (latent trigger), but store/fs panics or future
fetcher kinds convert directly into a deadlock.

**Fix:** wrap the run in `std::panic::catch_unwind(AssertUnwindSafe(...))`;
on panic treat as module failure — `running.remove`,
`kernel.finish_fail(idx)`, record a typed "worker panicked" error,
`notify_all`. This honors the kernel's total-entry contract (every
start answered by exactly one finish).

**Acceptance:** a temporary debug-gated panic seam (mirroring
`crash_hook`) makes a two-module `--jobs 2` apply return an error
instead of timing out; journeys/e2e unchanged.

### 3.2 Update verdict parity — Source, P1

`crates/gripsack-exec/src/update.rs:89-93,122-128`: check mode compares
full `LockEntry` equality; publish mode uses `same_source` (sha +
repo256 + version, asymmetric version arm) and unconditionally rewrites
the entry. Divergence: a cosmetic spec/metadata change → check exits 1
"would bump" while update prints "unchanged" yet silently rewrites lock
bytes. 0043 F3's check contract ("exit 0 when current; exit 1 when a
pin would move") is not a predictor of update.

**Fix:** one predicate — full entry equality in both modes; skip the
rewrite when unchanged; delete `same_source` (clean cutover).

**Acceptance:** offline probe (file-fetch module, hand-edit a cosmetic
field into the lock) — both modes agree, "unchanged" leaves lock bytes
untouched; `test_complete_pins`/`test_migration_feedback` stay green.

### 3.3 Resources on Verify/Intent steps are never acquired — Source, P1

sema validates declaration (`sema/resources.rs:12-34`); acquisition
exists for produce and deploy steps only; `module/verify.rs:31-78`
runs verifies with no resource lock, Intent steps only log. 0007 §4's
serialization promise silently does nothing for exactly the steps that
run arbitrary user scripts.

**Decision:** reject `resources` on Verify-action and Intent steps in
sema with a help line (throttling belongs on produce steps). Do not add
a second resource-acquisition path to the checks projection.

**Acceptance:** new E-code + sema test + one diagnostic e2e; two
modules sharing a declared resource on verify steps fail at check.

## 4. Recovery and store integrity (P0/P2) — source

### 4.1 Manifest `prior` bypasses the strict persisted boundary — P0

`read_manifest`/`validate` (`generations.rs:207-262`) validates
`entry.from`, `entry.hash` (64-hex), modes, store paths — nothing on
`entry.prior`. `prior.hash` flows into plain-path joins, not capability
reads: `deploy/restore.rs:70,122` read `home/prior/<hash>` with `..`
escaping `$GRIPSACK_HOME`, then write the bytes to the user destination
with the (unvalidated) recorded mode; `gc.rs:120` joins the same way
into the root set. Requires tampered/corrupt `$GRIPSACK_HOME` state
(trusted local), but contradicts the crate's fail-closed doctrine.

**Fix:** in `validate()`, require `prior.hash` 64-ascii-hex and
`prior.mode ≤ 0o7777`. Use the existing 0036 typed-producer convention
for admitted prior hashes and one capability-backed accessor; migrate
all restore and GC consumers, not only the first plain-path join.

**Acceptance:** tampered-prior repro (edit a manifest's prior hash to
`../`-bearing text; undeclare the module) fails at read_manifest before
any restore; pre-fix it writes planted bytes to the destination.

### 4.2 Duplicate-destination validation keys on the spelling — P2

`generations.rs:248` lowercases `entry.to` instead of using
`DeployedEntry::key()` (the canonical key 0030/0035 built). Aliased
spellings of one physical file pass validation; rollback's
`by_destination` map then silently drops one transition. Unreachable
from apply (expand-time check catches it) — this is the
hand-edit/old-manifest boundary the validator exists for. One-line fix
+ aliased-manifest test.

### 4.3 Staging residue is never collected — P2

`publish_generation` stages at `generations/.staging-<N>-<pid>` and
removes only its own pid's dir; after a crash the next apply uses a new
pid; `generations::list` skips non-numeric names; GC never sees them.
`generations.rs:616` asserts a never-used name (`.staging-1` without
pid) — the assertion is vacuous.

**Fix:** under `LifecycleSession`, reap only recognized staging
directories owned by the generation publisher; do not follow links or
delete arbitrary dot-directories. The lifecycle lock must exclude a
live publisher before cleanup. Replace the vacuous assertion with
enumeration of actual staged names and exercise recovery through the
existing persistence-matrix fault harness.

### 4.4 Journal and quarantine permissions — P2

`prior/` is 0700/0600 (0033 R2, "the listing itself leaks what files
were ever adopted"); journal entries — which contain destinations and
prior hashes — are written 0644&umask, and `journal/quarantine/`
persists rejected entries indefinitely at default perms. Tighten via
the same helper pattern.

### 4.5 `create_temp` unlinks on AlreadyExists — P2

`gripsack-fs/src/lib.rs:117-129` removes a leftover temp and retries;
`streamed.rs` deliberately refuses ("a collision must not unlink a file
belonging to another writer or a reused PID"). Adopt streamed's
create_new loop in lib.rs — safe direction, one convention.

### 4.6 Unrestorable-entry wedge needs an escape hatch — P2

Crash window with `Prior::Absent`; a *directory* appears at the
destination → restore's `remove_file` errors → reconcile fails → every
later apply/rollback aborts and GC refuses. Deliberate conservatism,
but the only unwedge is hand-deleting journal files.

**Decision:** retain fail-closed recovery; do not automatically discard
or quarantine an unrestorable intent and continue. Extend `doctor`
with read-only inspection of the blocked entry, observed destination,
required prior and actionable restoration instructions. Document the
manual repair procedure: preserve the unexpected object outside the
destination, restore the required precondition, rerun recovery, verify
the result. Acceptance: the directory-at-an-absent-destination case is
diagnosable and recoverable without editing/deleting journal records;
GC remains blocked until recovery completes. An arbitrary journal-drop
command is not part of this plan.

### 4.7 Destination observation is unbounded — P2, ledger-only

Each destination is read fully 3–4× (observe, precondition, capture,
postcondition). 0042 bounded fetch memory; destination observation was
never bounded. Record the constraint in the guarantee ledger; do not
redesign the drift guard.

## 5. Contract and frontend fidelity (P1/P2) — source

- **Verify-step actions escape E109/E115 (P1):**
  `sema/verify_paths.rs:14-19` and `sema/paths.rs:95-98` collect
  verifies from `module.verify` and `step.verify` only;
  `StepAction::Verify { verify }` is never admitted-checked —
  `sema/placeholders.rs:118-125` *does* include it, and
  `prepared.rs::checks()` collects all three. An absolute or `..` path
  inside a verify action passes `grip check` and fails/reads mid-apply.
  Fix: mirror the `checks()` collection in both passes. Acceptance:
  both payloads rejected with module span; no IR change.
- **E110 is vacuous for stepped modules (P1):**
  `commands/eval.rs::validate_sources` walks declarative
  install/config fields only; E103 empties them for steps-style
  modules, so a stepped fetch-less module with a typo'd repo source
  passes check and fails mid-deploy. Fix: walk the `PreparedModule`
  install+config_deploy entries, gated on "no fetch step".
- **Inert fields; E108 never emitted (P2):** `Entry.vars` (template
  only) and `marker` (merge only) are silently dropped on other modes;
  `codes::UNSUPPORTED_MODE` has zero emit sites. Fix: warning-level
  E108 sema pass first; promotion to error is a v-next decision.
- **Schema v3 rejects the whole-payload form (P2):** `$defs.entry.from`
  `minLength: 1` contradicts `sema/paths.rs:72-77` (empty `from` = the
  payload root, deliberately legal, TS-authorable). Preserve shipped
  behavior: remove `minLength`, document the whole-payload form and add
  a schema_acceptance corpus case. This is a schema correction, not an
  IR-version bump.
- **Env `append` exists on the wire but not the DSL (P2):** IR/schema
  support `op: "append"`; TS lowers only set/prepend. Minimal fix:
  accept `{ name, op, value }` objects alongside the `Record` sugar.
  IR unchanged.
- **Frontend resource registry is construction-order-sensitive (P2):**
  `typescript/src/resources.ts` validates refs at step construction; a
  host that constructs a step before calling `resource()` throws.
  Validate collected refs at `emitIr` instead — order-free, same error.
- **Lint silence for payload-sourced config (P2):** `lint` on a module
  whose config is payload-relative matches zero repo files and says
  nothing. Warn when `lint` is set and zero files matched.
- **Linter plugin protocol has no versioned schema (P2):** author
  `schema/lint/v1.json` (request/diagnostic/response) with a parity
  test over the exchange fixtures — the IR side got exactly this in
  0042 §E.
- **Docs sweep (P2):** `typescript/README.md` API table lists
  nonexistent `define`/`Module` exports; `crates/gripsack-ir/src/
  step.rs:21-22` KNOWN_RESOURCES comment describes a W201 open
  namespace that is now E107-closed; `plan/0003` §8's "IR readers MUST
  tolerate unknown fields" contradicts the settled strict contract
  (footnote superseding it); template `env.toml` linter example
  (`package = "griplint-helix"`) is not a valid `owner/repo@tag` ref;
  `AGENTS.md` names `schema/ir/v2.json` as THE current contract — it is
  v3.

## 6. Verification evidence integrity (P1) + the proof roadmap

The stack is genuinely strong — real biconditional contracts, one
implementation shared by production/explorers/verifier, calibrated
negatives with named attribution, honest exclusions. The gaps are in
the calibration harness and one false ledger sentence.

### Fixes (P1)

1. **Ledger/code mismatch:** `managed_blocks/mod.rs:156` ends in
   `String::from_utf8(bytes).expect(…)`; guarantees.md's
   MERGE-SPLICE-001 claims "fails closed, never panics." Fix the code
   (return a typed error, matching `ops/plan.rs:452` and
   `ops/execute.rs`) — do not downgrade the sentence. Not CLI-reachable
   today (the parser is line-aligned by construction); it is the
   failure mode the deferred Layer-2 proof would rule out, so the
   interim control must be honest.
2. **Mutant attribution (`scripts/check_verus.sh:86`):** calibration
   accepts any "not satisfied" line. Parse the verifier's error
   location and require the failing function/file to be the mutated
   one — the pattern check_models.sh already uses for TLC negatives.
   Acceptance: a scratch mutant breaking an *unrelated* lemma must fail
   the gate.
3. **Obligation floor:** `MIN_OBLIGATIONS=50` is global. Add per-module
   floors (or a named-postcondition presence check per kernel family)
   so a refactor that drops one contract cannot hide behind added
   lemma obligations.
4. **Seed the missing mutants:** OWNERSHIP-001 (drift-promotion:
   swap an Update/Preserve biconditional side) and GC-RECOVERY-001
   (roots-in-deletion). Their ledger rows currently describe mutants in
   conditional mood while the other families' are real.
5. **Stale spec mirror:** `specs/Transaction.tla`'s `Classify` retains
   the 0028-removed op-ordering heuristic arms for `prev=NONE`
   (unreachable in shipped cfgs; MultiDestination.tla removed them with
   a comment). Delete the arms, add the comment, add `ASSUME PREV #
   TARGET` — also required for the TLAPS pilot below.

### Binding verification programme — release work, not a research escape hatch

The owner explicitly requires the feasible, concrete verification
programme below **before the next public release**. M-V1–M-V7 are
`NEXT` deliverables; only M-V8 is `EXPLORATORY`. This supersedes the
previous placement of all proof work in optional M5 research.
“Feasible” is not an agent-controlled waiver: the targets use existing
specifications, production paths and established tools, but no new
proof is claimed feasible or discharged merely by writing this plan.
If a required obligation cannot be discharged, release stays blocked
until the code/specification is corrected or the owner explicitly
changes that obligation. Do not silently replace a theorem with tests.

The programme has four distinct evidence boundaries:

```text
reviewed safety requirement
  -> abstract protocol theorem (TLA+/TLC/TLAPS)
  -> shipped decision/transition implementation (Verus)
  -> production observer/executor correspondence (refinement campaigns)
  -> qualified filesystem/process behavior (native/fault/platform evidence)
```

An unproved arrow stays labelled unproved; having tools on both sides
is not a composition theorem. Preserve the current single executable
implementation shared by production, proofs and explorers. No new
proof engine replaces working Verus contracts or TLC counterexamples.

| ID | Required artifact and observable acceptance | Release class |
|---|---|---|
| M-V1 | TLAPS inductive-safety pilot over the corrected `specs/Transaction.tla`, companion `specs/TransactionProofs.tla`, pinned `tlaps` compose/CI lane and attributed barrier-order mutant. Discharge Init => Inv, Inv /\ Next => Inv', and Inv => stated safety; record all assumptions and remaining model bounds. A checked unchanged small model is the pilot, not the generalized result. | NEXT / M1 |
| M-V2 | Loom coverage of the real scheduler coordination seam: shared production transitions/coordination under instrumented synchronization, not a second hand-copied worker algorithm. Exercise panic, failure latch, notification/lock interleavings, exactly one completion per start and no lost-wakeup deadlock within the declared model. A broken completion/notification mutant must fail the named check. Label Loom results bounded systematic concurrency testing, not a universal proof. | NEXT / M1 |
| M-V3 | Merge-scanner Layer-2 implementation proof: accepted scan output has sorted, disjoint, in-bounds and UTF-8-aligned spans and satisfies the splice kernel's actual admission requirements. Keep malformed/ambiguous grammar and foreign-byte integration cases through the real scanner/executor. Property testing supplements this proof; it is not the former automatic fallback that marks the proof done. Fix §6's fallible wrapper regardless. Prefer the current Verus toolchain; an unsupported construct is an explicit blocker, not permission to axiomatize the scanner. | NEXT / M1 |
| M-V4 | Byte-level journal/marker admission properties over the real parser: absent versus explicit null, wrong types, duplicate/missing identity fields, unknown format versions, truncation and integer boundaries. Corrupt inputs never construct a valid recovery fact or cause effects; stable admitted data round-trips. Calibrate a missing-field/default-to-fresh-state defect. This is property-tested parser evidence, not a proof of serde. | NEXT / M1 |
| M-V5 | Production GC-root/build-closure integration property with an independent fixture/history oracle: retained manifests, transitive build inputs, priors and unfinished recovery protect the required objects; current is never pruned. A dropped-root or roots-in-deletion mutant fails. Keep the typed retention proof and exact-prefix pruning obligation from R2; the integration property does not claim to prove arbitrary filesystem inventory completeness. | NEXT / M1 |
| M-V6 | Generalized transaction/recovery safety theorem and concrete bridge, detailed below: arbitrary finite destination sets, no artificial two-crash cap, journal cleanup ordering and lifecycle/generation composition under explicit storage assumptions. Named cleanup/identity/publish-order mutants fail, and the real repeated-recovery campaign exercises the modeled boundaries. Do not label the small M-V1 theorem as this deliverable. | NEXT / M1 |
| M-V7 | Extend Verus into selected shipped transition/admission logic for journal ordering and the existing process/update/acquisition protocols, with explicit proof-to-effect boundaries. Prove the stated legal transitions, resource-budget arithmetic and error precedence; run hostile protocol/real-adapter tests and attributable seam mutants. The detailed obligations below are acceptance, not “add a model” placeholders. | NEXT / M1 |
| M-V8 | Isolated Aeneas-to-Lean experiment on an unverified safe-Rust parser/algorithm only after the required stack is protected. Actual-source extraction, theorem/axiom audit, pinned regeneration and a meaningful failing mutant are required to call the experiment successful. It does not replace M-V1–M-V7 or authorize a core toolchain migration. | EXPLORATORY / M5 |

#### Generalized recovery theorem: M-V6

`MultiDestination.tla` currently hard-codes `Destinations == {1, 2}`
and `crashes \in 0..2`. Merely running TLAPS on those definitions
retains both restrictions. Replace the proof model's hard-coded set
with an arbitrary finite destination set and allow crashes during
recovery without the two-crash cap; keep finite TLC instances for
counterexample discovery. Prove the relevant interference/composition
conditions rather than asserting “one destination is enough.”

Cover publication/current-pointer flip, entry/prior durability,
restoration, two-barrier cleanup and successive lifecycle operations:
new generations respect high-water/freshness rules; rollback may
reactivate an older generation but has distinct transaction/activation
identity. Connect GC admission and hook pending-state obligations to
those lifecycle transitions without claiming exactly-once hook effects.
Required safety statements are:

- Mutation cannot precede durable required intent/prior evidence.
- The published current pointer refers to an admitted, durably
  published generation under the named rename/sync assumptions.
- Exact transaction identity decides commit status; numeric order and
  Apply/Rollback labels cannot manufacture commit authority.
- Cleanup cannot destroy the evidence needed after another crash;
  restoration is durable before its recovery entry is removed.
- Ambiguous/corrupt state fails closed and retains actionable evidence;
  externally edited state is handled by the ownership contract, never
  silently overwritten by a recovery default.
- Recovery/retention composition preserves every admitted recovery root;
  repeated interrupted activation preserves its intent identity.

Strengthen the inductive invariant as necessary; do not weaken the
observable safety property to make the solver succeed. Explicitly
separate the abstract storage contract from actual OS durability.
TLC explores behaviors of its finite instantiated model, not arbitrary
parameter sizes. TLAPS claims only the generalized definitions and
assumptions actually proved. Any safety counterexample is a finding to
fix, not a model state to exclude without a justified domain invariant.

Liveness is a separate, **claim-gated** theorem: eventual successful
recovery requires that crashes cease, required I/O eventually succeeds,
work is scheduled and the state is recoverable. Persistent disk failure
or irreconcilable external drift cannot be wished away. Do not advertise
“always recovers” or “eventually succeeds” from a safety theorem.

#### Implementation and protocol correspondence: M-V7

Use the shipped journal/recovery, `gripsack-process`, update-summary and
acquisition-budget code as the targets; name exact extracted symbols
and claim IDs in the implementation ledger before editing. Minimum
obligations, in addition to existing kernel contracts:

1. Journal mutation/cleanup control flow can obtain effect authority
   only after the required admitted/durable predecessor states. Typed
   states or Verus tracked state must constrain the real executor;
   an uncalled ghost model or trusted wrapper around an unverified body
   does not satisfy this requirement.
2. Protocol frames/counters and resource-budget arithmetic cannot
   overflow into extra authority; oversized/invalid/partial inputs are
   rejected according to the admitted protocol. Input/stdout/stderr
   and outstanding-work bounds remain distinct, not interchangeable
   integers or a single “limit” used for different units.
3. Admission waits/retries consume one operation budget; a transition
   cannot reset/extend the deadline or report successful completion
   after a terminal failure. Real clocks, syscalls and descendant
   behavior remain explicit assumptions tested by the actual supervisor.
4. A complete update survey accounts for every selected module and
   failure dominates “changes available”; check/publish share one
   decision. Source layout evidence never implies recipes/hooks/verifiers
   ran. Reuse the existing model and production report types.
5. Production observers, action selection, syscall error classification,
   prior capture, effect execution and CLI/JSON results run through R2's
   independent concrete oracles. The proof of a pure transition is not
   presented as a proof that a filesystem observation supplied it the
   right facts. Include dropped-sync mutants at the persistence tier.

Use a small explicit specification for unavoidable external effects,
with every assumed postcondition and trusted body registered in the
assurance ledger. Do not give an opaque `write_durably` function the
whole property being claimed and count that assumption as a proof.
Where formal model-to-Rust correspondence is not itself proved, say
exactly which transition properties are proved and which mapping is
calibrated/tested. Do not silently promote the latter to “verified.”

#### Tool choice and exploratory limits

Keep Verus for actual Rust contracts, TLC for counterexamples, TLAPS
for inductive protocol safety, Loom for the coordination seam and
native/fault campaigns for the runtime boundary. Pin each tool/backend
and retain raw attributable results in R4's release evidence.

Lean/Aeneas is a separately scoped experiment, not the prerequisite
for this release. Aeneas' documented safe-Rust subset, translation and
external models remain part of the trust boundary; its current unsafe/
concurrency work is not a basis for replacing the native runtime proof
programme. Choose a remaining unproved safe-Rust admission/parser
property and extract its actual production implementation. If M-V3 has
already closed the selected scanner property, a second proof is an
explicit tool-diversity experiment, not additional behavior coverage;
do not substitute a handwritten Lean copy of the Rust implementation.
Acceptance: no `sorry` or axiom that assumes the target property;
audited external models/theorem assumptions; meaningful mutant failure;
reproducible extraction and proof after a source change. Unsupported
extraction is a valid recorded experiment outcome, not a passing core
verification gate. See the [Aeneas scope](https://github.com/AeneasVerif/aeneas#targeted-subset-and-current-limitations),
[TLAPS](https://github.com/tlaplus/tlapm), and
[Verus transition systems](https://verus-lang.github.io/verus/state_machines/).

## 7. Delivery and CI (P0/P2) — exact edits

Local files reviewed; live GitHub settings (branch protection, secrets,
environments, trusted publishing) are EXTERNAL and named as such.

**P0 — release correctness:**

1. `release-core.yml`: the publish loop order puts `gripsack-ir`
   (line 164) before `gripsack-policy`, but `crates/gripsack-ir/
   Cargo.toml:10` depends on policy at the same workspace version —
   on a fresh version bump the ir publish stalls its 12 attempts while
   nothing can publish policy, then the release fails. Correct order:
   `gripsack-policy gripsack-ir gripsack-fs gripsack-store
   gripsack-process gripsack-fetch gripsack-config griplint
   gripsack-lint gripsack-trace gripsack-exec gripsack`. Derive and
   assert this order from workspace dependency metadata so the next
   dependency does not recreate the bug.
2. `release-core.yml`: finalize and verify the GitHub release **before**
   updating the Homebrew formula or dispatching the website update.
   Keep registry publication before public release announcement.
   Publication across registries is not atomic: stage, record partial
   completion and resume safely; never claim a rollback of a published
   crate. §13 R4 defines the release-manifest commit point.
3. Recompute and compare every named artifact digest after each
   cross-job handoff and before attestation; reject missing, extra or
   mismatched expected subjects. Resolve checksum paths relative to
   one dist root, not a duplicated `dist/dist` shell path.

**P1 — gate composition:**

4. `ci.yml`: add top-level `permissions: contents: read`; add
   `concurrency: { group: ci-${{ github.workflow }}-${{ github.ref }},
   cancel-in-progress: true }` (ci/examples only — releases must not
   cancel in progress).
5. Add an aggregator `gate` depending on every required lane (today
   `test`, `e2e-macos`, `fuzz`, `docs`, `audit`; include new evidence
   lanes when added). Use `if: always()` and explicitly require each
   `needs.<job>.result == 'success'`; failure, cancellation or an
   unexpected skip must fail. A dependency-only `run: true` stub is
   not sufficient. Require this check in branch protection
   [EXTERNAL], and update AGENTS.md in the same cutover.
6. `release-typescript.yml`: add `id-token: write`,
   `npm publish --access public --provenance`, and a non-canceling
   concurrency group. Consider crates.io/npm trusted publishing to
   retire long-lived tokens [EXTERNAL registry-side config].
7. `audit.yml`: the granted `issues: write` is never used — add an
   `if: failure()` idempotent issue step (mirror check_upstream.py's
   pattern), or drop the permission. Decide the `--deny yanked
   --deny unsound` policy deliberately (currently warn-only).
8. `fuzz.yml`: mount a crash-out directory and `if: failure()`
   upload-artifact (retention 90d) — a weekly crash currently leaves no
   reproducing input.
9. `demo.yml`: pin `ghcr.io/charmbracelet/vhs` by digest and replace
   `curl | sh` Deno with the sha256-checked release zip (same block as
   ci.yml:63-68); register both in check_pins.py as a new unit.
10. `Dockerfile` verify stage: pin rustup-init by sha256 like
    cargo-auditable is (the only unchecksummed installer left).
11. `.github/dependabot.yml`: add the `npm` ecosystem for
    `/typescript` (Docker is intentionally owned by check_pins.py).
12. `install.sh`: exclude prerelease refs from stable selection. Replace
    optional provenance verification with §13 R4's fail-closed verified
    path; lack of a verifier must not silently become checksum-only
    success. Keep any weaker convenience path explicitly separate.
13. `ci.yml` audit job: drop the dead `checks: write` (plain
    `cargo audit` publishes no Check run).

## 8. Ecosystem cutover (P2)

Local snapshots; public production state not fetched — per-repo claims
are about these checkouts.

| Packet | Action | Evidence |
|---|---|---|
| E1 | homebrew-tap: bump formula 0.2.1 → current; the release workflow bump exists but the tap shows it never landed locally; add the release-driven tripwire (formula version == latest core tag) | tap Formula vs core Cargo.toml |
| E2 | Reconcile documented Homebrew channels with generated artifacts: `release-core.yml` creates BOTH a source Formula and a macOS binary Cask. `--cask` is valid once that Cask is published; do not blindly replace it everywhere. Verify advertised channels against the finalized release and repair the stale local tap snapshot. | release-core.yml:203–244 vs local Formula-only checkout |
| E3 | tknawara-dotfiles: update the obsolete frontend pin to the release manifest's tested compatible SDK; regenerate lock and run `npx tsc` + `grip check` in a disposable environment. Doctor must compare supported IR/SDK compatibility, not numeric “major behind” (all 0.x versions share major 0). | package.json/lock vs core; §13 R4 |
| E4 | Add migration tombstones to example-env-python (`frontend = "python"` is a hard migration error) and griplint-py (checker packs are in core). Request owner archival separately; do not delete/yank registry artifacts or archive live repositories as a side effect of implementation. | config error path, examples.yml |
| E5 | gripfetch-apt: README leads with the deleted python frontend; frontend-ts docstring uses a nonexistent `module({…})` call shape; env.toml example pins @0.1.0 vs Cargo 0.1.1 | README/frontend-ts vs DSL |
| E6 | griplint-conformance: `crash_is_contained_and_a_warning` never induces a crash — wire the choker fixture; update plan citations (0011 → 0012 core-driven) | suite source vs core exchange.rs |
| E7 | Use gripsack-dev.github.io as canonical per core AGENTS.md; verify domain ownership externally before disabling/archiving the older gripsack-site deployment. Consume the verified release manifest instead of manually bumping `GRIPSACK_VERSION`; add the shipped 0045–0047 and pending 0048 states to the roadmap. | pages.yml pins, roadmap; §13 R4/R7 |
| E8 | Correct org-profile's “Python or TypeScript” tagline. Mark obsolete linters-py metadata as historical; owner decides archival. No unrelated local-directory deletion. | profile README |
| E9 | Reverse conformance gate: core CI runs the published gripfetch-/griplint-conformance suites against a reference fixture plugin, so protocol drift fails core CI instead of a plugin author's report | both suites exist and match the core hosts today |

## 9. Binding delivery scope, release gates and permitted deferrals

**Owner directive:** splitting implementation across releases is allowed;
silently dropping work or its verification is not. The release classes
below supersede earlier loose milestone/research wording. P0/P1/P2 is
severity, not permission to defer. §10 quality is mandatory for every
landed packet, regardless of class or release.

For an assignment to implement **one release**, the default deliverable
is every `NEXT` item and prerequisite, fully integrated and verified.
`LATER` work may remain queued only as explicitly authorized below.
For an assignment to implement **the whole plan**, `LATER` remains
required scope; completing M0–M2 is not “plan 0048 implemented.” Do not
infer permission to stop at an arbitrary subset of a required packet.

### Release classes and authority

| Class | Meaning | Who may change it? |
|---|---|---|
| NEXT | Must land, meet every acceptance criterion and pass all applicable verification before the next public release, including an alpha/prerelease claiming these guarantees. Missing code, proof, test evidence or quality work blocks release. | Owner only; an agent may not downgrade it. |
| LATER | Still required work, explicitly allowed to miss the next release and assigned to M3/M4. The first release that includes it must also include its complete verification/docs/migration. | Owner decides cancellation or further schedule change; current scheduling permission is not deletion permission. |
| CLAIM-GATED | Evidence must exist before the corresponding capability/platform/assurance is advertised; required pre-1.0 obligations remain required. A release that does not make that claim may carry the explicit exclusion. | Owner controls supported scope; an agent cannot withdraw an existing claim silently to evade verification. |
| EXPLORATORY | A genuine optional experiment, not a missing implementation of a required feature. May be deferred without blocking a release; no passing claim is published until actual success. | May be queued as listed; expanding it or substituting it for required work needs owner approval. |

**External action is a status, not a fifth optional class.** A required
signing, branch-protection, reporting, domain or platform-owner action
stays required even if the implementing agent lacks permission. Finish
reachable implementation and verification, record the exact action and
keep release blocked. Do not mark “enabled” based on a proposed YAML
file or assume that adding documentation changed a live setting.

### Integrated milestones

M0–M2 form **one next-release gate**, not permission for three partial
public releases. Code may merge in complete internal packets; the owner
chooses version/tag only after the release gate passes. M3–M4 are the
pre-authorized later product windows. M5 is platform/claim evidence and
explicit exploration, no longer a bucket for skipping core proofs.

| Milestone | Dependencies | Work to land | Observable exit |
|---|---|---|---|
| M0 — stop boundary defects | none | §1.1, §1.2, §1.3, §1.4; §2.1, §2.2; §3.1; §4.1 and §4.4; §7 items 1–3; R1; the R5 controls required by evaluator/native calls used in this release | Original reproductions fail safely or terminate; changed source cannot execute under an unchanged approval; corrupt priors are rejected before effects; evaluator output is bounded; release order dry-run passes. |
| M1 — verified core decisions, transitions and replay | M0 for concrete effects; proofs/types may start earlier | R2 and R3; §3.2, §3.3; §5 Verify-action/E110 fixes; §6 fixes 1–5 and every M-V1–M-V7 deliverable | Typed contracts, generalized protocol theorem, Layer-2 proof, Loom and concrete/persistence/protocol campaigns pass with attributable negatives. Hook IDs/outcomes and check/publish decisions satisfy the real state contract. |
| M2 — truthful verifiable release | M0–M1 and required owner actions | §7 items 4–13; R4; R6 applicability/identity contracts; R7 security/current-doc/canonical-site cutover; §8 E1–E3 and E7–E8 public-contract work; §10 quality gate | Complete candidate evidence binds the exact artifacts; required CI aggregation is enforced; verified installation fails closed; actual site/docs/compatibility match the signed tuple; no required verification is deferred. |
| M3 — remaining runtime and contract features | Complete M0–M2; shared receipt contracts | Remaining R5 native/trace breadth not already required by shipped behavior; R8 JSON survey; §5 remaining IR/DSL/lint parity and capability inventory; §4.2, §4.3, §4.5, §4.7 | Full corresponding runtime/contract behavior and its verification land together; unchanged earlier proof obligations remain green. |
| M4 — ecosystem and recovery usability | R4 tuple; M3 lint protocol where consumed | §4.6 diagnostic/manual repair; §8 E4–E6 and E9; remaining R7 public examples and site regression automation | Recovery can be repaired without journal surgery; ecosystem/conformance/examples target the actual compatible release; newly shipped surfaces are verified, not merely documented. |
| M5 — qualified platform evidence and exploration | Stable production implementations; owner-provided facilities | R6 VM/mount qualification; remaining claim-gated pre-1.0 fixtures/byte-binding evidence; M-V8 optional Lean/Aeneas and separately scoped liveness research | Claimed platforms/capabilities have actual evidence. Unrun experiments remain explicit; M-V1–M-V7 are already required before release, never deferred here. |

### Scope register — exactly what may wait

These entries are exhaustive groupings of §§1–8 and R1–R8, not samples
of the “important” work. Each referenced packet includes **all** its
changes and acceptance cases unless an exact split is stated here.
Every new behavior carries its own tests/proofs/docs in the same release.

| Scope | NEXT commitment | Explicitly authorized later portion |
|---|---|---|
| §1.1–§1.4 evaluator/host/adopt | All fixes, every caller and regression. No exception for a trusted-only fixture or alternate CLI entrypoint. | None. |
| §2.1–§2.2 archive/plugin/throttle | All admission, containment, identity and bounded-wait fixes and regressions. | None. |
| §3.1–§3.3 scheduler/update/resources | All behavior changes, sema diagnostics and actual lifecycle regressions; M-V2/M-V7 cover their verification seams. | None. |
| §4.1 / §4.4 persisted priors/privacy | Strict prior admission, capability access and private authoritative state; privacy of every new receipt is included now. | None. |
| §4.2 / §4.3 / §4.5 / §4.7 | Any portion needed to satisfy a NEXT invariant or failing required campaign is promoted into this release. | Otherwise M3: aliased-manifest key fix, recognized staging cleanup, collision-safe temporary-file handling and the explicit destination-read applicability limit. |
| §4.6 recovery usability | Preserve fail-closed recovery and actionable existing errors; no automatic journal discard. | M4: richer doctor inspection and the documented manual-repair flow, with its scenario verification. |
| §5 contract fidelity | Verify-action E109/E115 and stepped-source E110 fixes; correct every current public contract/example made false by this release and the enumerated stale API/IR descriptions. | M3: remaining schema/DSL parity, lint protocol schema, zero-match diagnostics, resource construction order and inventory feature. Broad additional API examples may follow in M4, not documentation required to use a changed API. |
| §6 verification | Fixes 1–5 and M-V1–M-V7 in full. No automatic proptest-for-proof substitution and no “research” relabeling. | M-V8 only is EXPLORATORY; conditional liveness/hardware claims follow their explicit claim gates. |
| §7 delivery/CI | All 13 items, required live aggregation/signing/registry actions, staged negative release/installer verification. Resolve each offered alternative explicitly; do not omit the item. | Optional provider/tool migrations only if the selected secure published path is already complete; no optional-verifier downgrade. |
| §8 E1–E3 / E7–E8 | Compatible active examples/pins, actual Formula/Cask channels, current frontend metadata and canonical manifest-driven site. | Owner-directed archival/domain cleanup cannot be simulated; obsolete-repo archival itself is not required if an honest migration surface is published and domain safety is established. |
| §8 E4–E6 / E9 | Do not advertise obsolete Python frontend support or unexecuted conformance as current evidence. | M4: remaining historical-repo tombstones, transport-plugin docs, choker fixture and reverse conformance integration. |
| R1 source-bound approval | Entire packet, explicit legacy trust migration and every adversarial source case. | None; mutable-tree hash/recheck is not a smaller equivalent delivery. |
| R2 typed policies/refinement | Entire packet and all concrete oracles/mutants/receipts, plus §10 domain-type migration. Required proof work follows M-V1–M-V7, not a later research carveout. | Optional richer pruning prose/UI only; the canonical decision and machine evidence may not be deferred. |
| R3 hook identity/outcomes | Entire packet: durable per-intent identity/results, old-state handling, inspectability, safe fixture simulations, examples and actual crash/replay evidence. | No generic external compensation/retry product is required or authorized. |
| R4 release/assurance/installer | Entire packet, including fail-closed consumers, actual provenance policy, invalid-input cases and owner-run security actions. | Additional signing providers or new SBOM formats are optional; the selected path cannot ship incomplete. |
| R5 process/trace policy | Bounds, identity/grant/env/FD handling, reserved hook IDs, escaping and private bounded receipts for every caller introduced or changed by NEXT work; run required hostile campaigns across currently shipped supported behavior. The existing documented containment tier must remain honest. | M3: remaining caller migration and full completed-log lifecycle/retention. Additional complete-tree containment tiers are CLAIM-GATED; no silent fallback to group-only under a stronger advertised claim. |
| R6 filesystem/identity | Changes 1–5 as applicability/identity contracts, every named software fault/process-crash case applicable to the shipped paths, and correct evidence status. A required case exposing a real safety failure promotes its fix to NEXT. | Native naming support extensions in M3; actual VM/mount campaigns in M5 only where no current power-loss certification is advertised. Required pre-1.0 platform evidence stays required. |
| R7 public/security/site | Truthful active frontend/lint/API contracts, usable private reporting/support/revocation policy, manifest-driven canonical site and actual browser/example/header checks on surfaces changed now. | M3/M4: inventory convenience, broader additional examples, historical-repo migration and expanded repeatable accessibility/performance automation. Required verification of this release's UI/docs is not deferred with that automation. |
| R8 structured surveys | Preserve and verify the current source-layout/survey/error-precedence contract and §3.2 decision parity now. | M3: new JSON/coverage output and its real CLI scenarios. Arbitrary native-effect `--preflight` remains an unaccepted separate product proposal, not a deferred stub. |

**Prerequisite-promotion rule:** a LATER item becomes NEXT when a NEXT
implementation depends on it, a required campaign exposes its safety
failure on a supported path, or this release advertises its guarantee.
Do not keep a known failing dependency on the later list while marking
the dependent feature complete. Conversely, an unshipped new convenience
command does not need a fake test against an inert placeholder: leave
both implementation and verification explicitly queued.

**Verification attaches to the release, not to leftover capacity.** All
M-V1–M-V7 and existing applicable gates run before the next release.
Every other in-scope proof/test/campaign must run before the first
release containing its related behavior. Failed, skipped, cancelled,
timed-out, unsupported or unrun required evidence is **not success**.
A new negative test failing for unrelated syntax/compile errors is not
a successfully killed behavioral mutant. Fixes discovered by required
verification are part of the release, not optional follow-up work.

### No unilateral reduction or evidence substitution

- The agent may not drop a leaf requirement, weaken its acceptance,
  remove a relevant negative, change a required proof into a smoke
  test, lower a guarantee to make a failure disappear, introduce a
  permissive fallback, or mark partial scaffolding complete.
- Preserve safe code-quality refactors, schema/caller migration and
  public contracts as part of the feature. “Refactor later” is not an
  authorized way to satisfy §10 with unreadable code today.
- If required work cannot be completed, record the concrete technical
  blocker, attempted approaches, retained evidence and remaining
  obligations. The state is **release blocked**, not “done with follow-ups.”
  Only the owner's explicit decision may waive or reschedule NEXT.
- An owner-approved exception must identify exact requirement IDs,
  release scope, risk, temporary guarantee wording, replacement evidence
  if any and the future target. Silence, a failed tool, an earlier alpha
  label or generic instruction to “make progress” is not approval.
- An emergency/security hotfix release with reduced scope requires
  that same explicit owner decision; the implementing agent cannot
  invent the exception. This plan itself authorizes no publication.

### Required implementation and handoff record

Before coding, expand this scope register into a **leaf-level execution
record in this plan**, with stable packet/step identifiers such as
`R1.1` and the existing section IDs. Include every change and acceptance
criterion from the referenced packets; do not summarize away substeps.
Keep one record, not an out-of-date second roadmap. Each leaf records:

- release class and target milestone;
- prerequisites and implementation owner;
- status: pending / implementing / implemented-unverified / verified /
  blocked (including external blocker) / deferred-authorized;
- changed symbols/modules, compatible wire/schema migration and PR/SHA;
- actual command/proof/case, result and evidence artifact;
- every unmet acceptance criterion;
- deferral authority (this register for LATER, or the owner's specific
  decision for a NEXT exception), risk and resumption target.

Only **verified** counts as landed for release. Deferred-authorized is
open work, never checked off as implemented. Historical baseline results
cannot verify changed code. At each handoff report the exact completed,
blocked and queued IDs, next executable step and failing/unrun evidence;
never “everything important is done.” Do not end a whole-plan assignment
merely because one internal milestone merged.

### Parallel ownership and merge contracts

The integration owner owns shared wire/schema contracts, evaluation
inputs, `crates/gripsack/src/main.rs`, journal format, required CI
aggregation and final release evidence. Assign file/module ownership
before concurrent edits; serialize only shared mutation boundaries.

- M0 lanes: evaluator/env/source approval; archive/plugin admission;
  scheduler panic handling; persisted-prior/privacy; release-order fix.
- M1 lanes: typed API/Verus and callers together; hook records/replay;
  update/IR admission; TLAPS generalization; Loom/protocol/refinement
  campaigns. Synchronization and schema consumers use agreed contracts,
  not sibling implementations with coincidentally similar names.
- M2 lanes: evidence producer, verified installer/self-update consumer,
  canonical site/compatibility consumer, security/platform governance.
  One manifest schema and verification policy serve every consumer.

Focused regressions run after a coherent lane is integrated. Run full
container gates once against the integrated candidate, not against
half-merged siblings. A later code change affecting a proved/tested
path invalidates the affected evidence and requires rerunning it.

### Next-release checklist — all items required

- [ ] Every NEXT leaf and promoted prerequisite is verified; every
  permitted later item remains individually recorded as open work.
- [ ] Original bug regressions and R1/R2/R3 source/refinement/replay
  campaigns exercise the real shipped code with attributable negatives.
- [ ] M-V1–M-V7 all meet their specific acceptance criteria; no proof
  contains an admitted target theorem, silently reduced domain or
  replacement “test-only” result. Evidence clearly distinguishes
  theorem proofs from property/concurrency/fault testing.
- [ ] `docker compose run --build --rm test`, `ts-test`, `e2e`,
  `model`, `verify` and newly registered pinned TLAPS/Loom/protocol
  lanes pass on the exact final candidate. Verus remains amd64-only;
  supported native macOS behavior has its own required evidence.
- [ ] Applicable software-fault, protocol and native behavior matrices
  pass. A missing VM claim is explicitly excluded, not represented by
  SIGKILL or a simulated persistence result labelled hardware evidence.
- [ ] §10's mandatory readability/type/module/crate review is complete;
  no quality debt was deferred merely to reduce the implementation diff.
- [ ] CI aggregation rejects failed/skipped/cancelled required jobs;
  manifests/digests bind the candidate, code, configurations and results.
- [ ] Non-publishing release/install/self-update dry-runs include bad
  digests/identities, missing evidence, wrong compatible SDK, partial
  publication and verifier failure; old binaries remain usable.
- [ ] Actual affected CLI/site/example paths are exercised; required
  migrations, docs, STATUS/changelog and release instructions agree.
- [ ] Required owner signing/protection/reporting/domain/revocation
  actions have observed evidence. Test staged deployment first; no
  production publish is authorized merely to discover if it works.

### Pre-1.0 and claim-gated obligations

- [ ] M-V1–M-V7 stay required and green across subsequent changes.
  Additional liveness claims state fairness/termination assumptions.
- [ ] Historical lockfile, generation, journal, trust and IR fixtures
  migrate explicitly or refuse before effects. A format changed by
  the next release requires those fixtures **now**, not only at 1.0.
- [ ] Source bundles, selected executable bytes and actual grants are
  receipt-bound; unsupported spawn-race/containment guarantees are
  not advertised. Adding a stronger tier requires its actual tests.
- [ ] Each advertised crash-durability filesystem combination has both
  process-crash and qualified VM-power-loss evidence before 1.0; absent
  hardware/OS facilities block that claim, not manufacture a pass.
- [ ] Stable hook IDs, durable outcomes and ambiguous-state recovery,
  signed manifests, verified payloads, existing in-binary SBOM and
  owner-tested revocation remain complete, not pre-1.0 placeholders.
- [ ] Public docs and current compatibility/evidence describe downloaded
  bytes. No universal correctness or universal filesystem slogan.

LSP/fetcher registry, a native Windows port, arbitrary-effect preflight
and a Lean/Aeneas migration are not secretly added release requirements.
Only the explicit M-V8 experiment may wait as optional proof research;
the concrete core verification programme is release-blocking.

## 10. Non-negotiable code-quality acceptance contract

**Owner directive:** optimize for correctness, explicit invariants,
readability and maintainability by the next engineer—not writing speed,
token efficiency, shortest code, smallest diff or fewest files. Efficient
compiled behavior still matters: clarity is not permission for needless
allocation/copying. This section is part of every packet's acceptance;
it cannot be deferred to a later “cleanup release.”

Use the established typed-producer/plain-wire and cohesive-module
patterns, strengthened here. This supersedes 0036's blanket reluctance
to type generation identifiers and its stringly-kernel exception at the
boundaries changed by this plan. Do not cite that historical decision
as permission to leave new semantic values interchangeable.

### Domain types, units and naming

1. **Semantic values have semantic types.** Use existing identity types
   and introduce the necessary missing domains: `GenerationId`,
   `ActivationIntentId`, `HostName`, `FileMode`, retention policy/count,
   executable identity, byte counts/limits and attempt counts. Use
   `Duration`/`Instant` for time, not unitless integers. Distinguish an
   identifier from a count/index and a limit from the value it bounds.
   Prefer a name such as `ActivationIntentId` to `Aid` or `Id`; types
   should explain their role without reading their implementation.
2. **Role errors must not compile.** R2 desired/live/prior identities
   remain distinct; takeover, drift, admission/commit/outcome and
   protocol phases use enums or named state structures, not positional
   Booleans, integers or tuple slots. An enum with impossible field
   combinations is not sufficient; constructors/variants encode valid
   states. `type GenerationId = u64` is an alias, not type safety.
3. **Validate once at an owned boundary.** Private fields and fallible
   constructors establish range/ordering/encoding invariants. Serde DTOs
   may remain wire strings/numbers where compatibility requires, but
   must convert into admitted domain types before policy/effects.
   Never deserialize unvalidated data directly into a supposedly valid
   domain object. Keep serialization shapes stable unless the approved
   migration changes them; no silent default supplies missing authority.
4. **Clean type migration is required.** A changed shared lifecycle API
   uses `GenerationId` all the way through its callers; no raw-u64 alias,
   duplicate overload or `as_raw()` detour to avoid updating consumers.
   The primitive representation may remain inside the newtype, numeric
   algorithm or explicitly admitted wire boundary. Use checked conversion
   and checked arithmetic for boundaries, not truncating casts/sentinel
   values. Bounded proof indices may use mathematical integers without
   confusing them with executable generation/byte/time domains.
5. **No magic policy numbers.** Retry limits, deadlines, quotas, protocol
   versions, permission masks and status mappings are named typed values
   or constants with units and a documented policy source. Share one
   definition across related consumers; do not duplicate `16 * 1024 *
   1024` or arbitrary `3`/`2` rules in callers. Literal zero/one for an
   obvious local index, wire examples and boundary fixtures are fine;
   this is not a demand to newtype every loop counter or hide ordinary
   arithmetic behind a constants module.
6. **Names favor understanding.** New public and domain names describe
   the value/action (`GenerationInventory`, `DestinationObservation`,
   `ExecutableIdentity`, `RecoveryOutcome`), not `Data`, `Val`, `Ctx2`,
   `Op2`, `Aux`, `tmp` or unexplained abbreviations. Established `IR`,
   `JSON`, `CLI`, local iterator indices and existing narrow APIs may
   retain conventional names where unambiguous. Do not create terse new
   APIs to imitate an old one; improve misleading names when touching
   their ownership boundary. Use LSP renames/references when available
   and migrate every caller, example and proof.

### Cohesive modules and purposeful crate boundaries

**Module decomposition is mandatory when the packet materially touches
a mixed-responsibility module; it is not optional cosmetic cleanup.**
Before implementing, record a small responsibility/dependency map in the
packet's execution record. Preserve one owner for each invariant and
operation; split along meaningful responsibilities, not by arbitrary
line chunks or a “types/helpers/utils” dumping ground.

Concrete existing pressure points to split as they are changed:

| Current area | Required ownership separation | Constraints |
|---|---|---|
| `crates/gripsack-store/src/journal/mod.rs` | Wire/admission, durable record storage, recovery coordination and public facade must be separately reviewable. | Reuse existing `journal/marker.rs` and `journal/recover.rs`; do not create second marker/recovery implementations. Keep the facade small and durability ordering discoverable. |
| `crates/gripsack-exec/src/ops/plan.rs` | Copy/link/merge/removal planning and common admitted observation inputs have explicit homes. | Keep one policy dispatch and the existing `ops/execute.rs`, `preview.rs` and model consumers. Mode-specific modules may share a narrow admission abstraction, not duplicate classification/report logic. |
| `crates/gripsack-store/src/generations.rs` | Manifest types/admission, inventory, generation publication and current-pointer operations have explicit ownership. | The published-generation invariant crosses these boundaries deliberately; do not spread plain path joins or introduce competing validators. |
| Evaluation/trust and activation/process packets | Source bundle preparation, approval, process launch and replay receipt persistence are separate responsibilities with explicit handoffs. | Extend existing core/store/process owners first. Do not replace one oversized file with another generic runtime module. |

Aim for roughly 400 or fewer non-test production lines per cohesive
file. This is a **review trigger, not a line-count game**: no compressed
formatting, giant expressions, generated source or unnecessary fragments
to satisfy the number. A larger non-generated file needs a concrete
cohesion justification in the review; mixed responsibilities still must
be split. Proof modules may be larger when theorem/algorithm locality
helps review, but each keeps one named kernel family and readable
lemmas. Move substantial tests/proof support into cohesive companion
modules when they obscure production control flow; keep useful local
invariant documentation with the implementation.

**New crates are permitted, and required if a real invariant/dependency
boundary cannot be expressed cleanly inside existing crates.** Prefer a
module split when it is sufficient. Before adding a crate, record:

- its single responsibility and public contract;
- why an existing module/crate is not the right owner;
- actual consumers or a concrete proof/security/dependency-isolation
  need (not speculative future reuse);
- the resulting acyclic dependency direction and who owns shared types;
- expected public API, verification and release/versioning effects.

Do not create a `common`, `shared`, `core2`, “framework” or generic
receipts crate to avoid choosing ownership. Do not add traits/generics,
callbacks or dependency injection without a real substitutable behavior
or proof boundary. The existing policy crate stays independent of
filesystem/process/runtime code. Keep visibility private/pub(super)/
pub(crate) unless external consumers need the API; no blanket `pub` or
re-export chains that disguise coupling.

A necessary new crate lands completely: workspace/lock metadata,
acyclic imports, publish topological order, applicable container/musl
builds, proof/test/fuzz/docs coverage, release manifests and examples.
No empty crate, half-migrated facade, compatibility alias, duplicate
algorithm or disabled feature path counts as a finished split.

### Readable control flow, errors and efficiency

- Prefer explicit phases and exhaustive matches at dangerous decisions.
  A reviewer must be able to follow observe -> admit -> plan -> record ->
  effect -> verify -> cleanup without deciphering nested iterator chains,
  closure capture or Boolean algebra. Small named helpers are useful
  when they express one invariant; one-line indirection without meaning
  is not. Preserve deterministic order and visible error precedence.
- Use typed errors with operation/phase/cause and safe context. Preserve
  NotFound versus PermissionDenied/corruption/timeout distinctions. No
  `.ok()`, broad default, success-looking empty result, arbitrary retry
  or swallowed error to make a failing path pass. Repo/process/wire input
  cannot reach an `unwrap`/`expect` panic in place of admission. Narrow
  internal assertions need a locally understandable established invariant;
  the scheduler's explicit panic-to-failure containment remains required.
- Borrow identities and read sets where possible; no string clones,
  rehashing, repeated sorting or allocations merely to appease the type
  checker. Newtypes/enums should be zero-cost unless ownership is real.
  Do not trade clear phase boundaries for micro-optimizations without
  measured need. No hand-rolled unsafe filesystem/serialization code when
  the existing capability and typed admission patterns suffice.
- Bound untrusted I/O and allocation before consuming it; a ledger
  exclusion is not permission to leave a NEXT budget requirement undone.
  Preserve R6's explicit, owner-approved platform limitations instead of
  manufacturing runtime guarantees. No state hidden in process-global
  environment, magic numeric sentinel or unrelated generic context bag.
- Contracts explain why: accepted/rejected states, ownership, ordering,
  durability point, units, failure semantics and proof assumptions.
  Public rustdoc and examples are part of a changed API. Plan numbers
  supplement these contracts, never replace them. Use descriptive proof
  lemmas/intermediate facts instead of opaque solver incantations; any
  trusted body/axiom or verifier workaround is visible and justified.

### Quality and verification sign-off — mandatory for every landed packet

- [ ] Responsibility map matches the resulting modules/crates; required
  splits happened and no new catch-all module or dependency cycle exists.
- [ ] Domain/role/unit types and policy enums prevent plausible misuse;
  no naked semantic numbers, tuple flags or aliases bypass admission.
- [ ] Public/error/control-flow names are readable without author context;
  non-obvious constants, phases and proof assumptions are explained.
- [ ] Every caller, schema, proof, explorer, fixture and affected example
  uses the final contract; obsolete implementations/shims are removed.
- [ ] Proofs and regression/campaign checks defend observable invariants,
  boundaries, errors and recovery—not source text, wiring echoes,
  incidental wording or a coverage percentage. Broken-property mutants
  fail the named obligation. No changed-code evidence is inherited from
  an old SHA and no required check is disabled to get green.
- [ ] Deterministic isolated tests exercise actual implementations. Any
  discovered in-scope implementation/wording-only test is removed rather
  than mechanically re-pinned. Tests are readable artifacts, not padding.
- [ ] Container format/lint/build/tests and all applicable proof/native/
  effect-surface checks pass after integration. Any introduced `unsafe`
  or trusted verifier escape has a reviewed necessity/invariant record.
- [ ] Review findings are resolved before “verified.” Record the reviewer
  (or an explicit self-review where independent review is unavailable),
  evidence and decisions; do not claim an independent review that did
  not occur. The author's claim that code is “clean” is not acceptance.

A packet that works but violates this contract is **unfinished**.
Splitting a release may move an authorized feature; it may not move the
quality work needed to make already-shipped code maintainable.

## 11. Sound — retain, do not re-litigate

One planner for plan/apply/rollback with precondition+postcondition
per op; tagged/versioned journal wire with fail-closed quarantine;
exact-equality commit classifier with required `previous_generation`;
two-barrier cleanup ordering; LifecycleSession typing the lock+home;
capability-pinned destination mutations; high-water monotonic
generations; shared production/explorer/verifier kernels with
proof erasure on plain cargo; check_models.sh's attribution-bound
negative calibration; the journey harness and persistence matrix as
the process-crash tier (at this baseline, power loss is covered by
ordering assumptions/models, not hardware tests; R6 adds separately
qualified VM evidence); TypeScript-only frontend;
filesystem store over SQLite; in-binary SBOM; pinned, not hermetic.

## 12. Non-goals and settled

No universal correctness claims from finite TLC instances; no verified
wrappers over the threaded bridge; no Lean in the core repo; no second
frontend; no TOML frontend; no automatic rollback on hook failure; no
global step VM or partial retry engine. The external-writer exclusion
(cooperating-process lock only) stays visible in the ledger verbatim.

## 13. External review disposition and integrated work packets

The pasted review is useful boundary-oriented prioritization, not an
independent execution audit. Agree with narrowing claims, typed policy
admission, source-byte trust, hook idempotency support and release
coherence. Do not adopt its grades as test evidence or its proposals as
confirmed vulnerabilities. The three scout source slices and Main's
public/current-doc reads were read-only; no new binary/security campaign
was run in this planning pass.

### Finding-by-finding disposition

| Finding | Disposition and pushback | Unified packet / overlap |
|---|---|---|
| F-01 | Accept release-scoped evidence binding. Existing guarantee ledger already states properties/tools/exclusions; repair its known false sentence and dangling references rather than start from zero. | R4; §6, §7 |
| F-02 | Accept borrowed identity roles, explicit lineage/takeover and validated sorted inventory. Do not replace existing hash types or allocate strings; a fields-only struct with raw Booleans is insufficient. | R2; §6 calibration, §10/0036 convention |
| F-03 | Partly existing. Concrete op, lineage, recovery, mode and persistence harnesses ship; extend uncovered lifts and independent seam calibration, not a second framework. | R2; §6 M-V2/M-V5 |
| F-04 | Correct reviewer premise: trust is path-only; remote/HEAD are informational, capabilities are not receipt-bound. Adopt source-bundle approval as an explicit stronger product contract, not a falsely described existing guarantee. | R1; §1/R5 |
| F-05 | Accept stable IDs, durable per-intent results and fixture simulation. Pending activation already exists; keep warning/no-rollback/no-automatic-retry policy and acknowledge remote-success ambiguity. | R3; §4/R5 |
| F-06 | Accept explicit applicability and VM campaign. Current docs already exclude hardware proof; no blanket filesystem/Windows guarantee or generic fsync certification. | R6; §4/§6 persistence |
| F-07 | Accept canonical tuple/lifecycle cutover. Independent core/TS versions are intentional; metadata must describe compatibility rather than force every version number equal. | R4/R7; §7/§8 |
| F-08 | Stale-version finding, not missing checker: 0.42 documentation and implementation ship it; 0.23 says “lands next.” Fix current discoverability and real lint protocol/conformance gaps. | R7; §5/E6/E9 |
| F-09 | Accept verify-before-execute. Sidecars alone do not authenticate a publisher. Also remove the site's mutable-main installer fetch and distinguish script bootstrap trust. | R4; §7.12/§8 |
| F-10 | Partly existing bounded supervision; real gaps are bypassing callers, identity/env/FD binding and escaped descendants. Native capability declarations are not OS containment. | R5; §1.2/§1.4/§2.2 |
| F-11 | Accept structured coverage; reject blanket “validation_performed:false” and temp-directory “isolated preflight.” Source layout/sema already run; arbitrary-effect preflight is a separate proposal. | R8; §3.2, 0044 |
| F-12 | Accept private bounded logs, safe cleanup/redaction/rendering. JSONL escaping and atomic latest replacement already exist; recovery receipts are not disposable logs. | R5; §4.4 |
| F-13 | Accept consolidated metadata/alias contract. Destination identity already includes full mode; store tree identity intentionally normalizes executability. Do not silently change hash domains. | R6; §4.1/§4.2, 0031/0036 |
| F-14 | Accept public examples/invariants and named stale-doc fixes; do not add a generated-API documentation framework or universal coverage quota. | R7/R2; §5 |
| F-15 | Accept actual support/reporting/revocation policy. Private reporting, signers and release roles require owner verification; no fictional email/SLA or automatic disclosure. | R7/R4; §7 |
| F-16 | Accept canonical assurance index and actual site checks. Hosting headers/duplicate domain control need deployed evidence; an old workflow pin is not proof the live homepage is that version. | R7/R4/R6; E7 |

### Cross-packet receipt and schema contract

Use existing owners rather than a generic new receipts service:
evaluation owns source approval; Op/journal owns mutation intent and
postconditions; activation owns replay state; process owns launch
identity/enforcement; trace is a bounded diagnostic projection; release
owns evidence publication. Each wire record has a schema/version,
stable operation/intent ID, named facts and outcomes, and references
other records by ID/digest. Unknown versions fail closed at authoritative
admission; missing evidence never manufactures success. Diagnostic
tracing may be incomplete without deleting durable recovery state.

Schemas for the generated release/assurance output and new authoritative
receipts land with their consumers and fixtures. Wire types use the
repo's admitted producer pattern. Evidence must not include raw file
contents, credentials, secret environment values or unredacted remotes.
Receipts describe what was observed/enforced; they are not additional
proofs and cannot eliminate external-writer or OS/runtime assumptions.

### R1 — Approve and evaluate the same source bundle

**Evidence / disposition (F-04).** The review's premise is incorrect:
`gripsack-store/src/trust.rs:35–45,78–83` keys approval on canonical
path only. Remote and HEAD are audit fields, not keys, and the prompt's
capability summary is a fixed string. `GRIPSACK_TRUST_ALL=1` bypasses
the entire gate. This is documented design, not a newly reproduced
exploit; the external claim of commit/capability-bound trust must not
be repeated. `commands/frontend.rs` and `probe.rs` evaluate the mutable
worktree. Content identities computed later for deploy inputs do not
bind frontend evaluation.

**Changes.**

1. Deliberately supersede 0013/trust.rs's *path-only* convenience rule:
   approval covers canonical repo identity, a source-bundle digest and
   actual evaluator/native-action grant policy. Keep HEAD/remote as
   provenance, not an assertion that those bytes were evaluated. Dirty
   work is allowed **only after approving its captured bytes**. This is
   a user-visible trust-contract change: edits can require reapproval;
   it is not a transparent bug fix or a new per-commit prompt policy.
   Document it in trust help, migration notes and the security page.
2. Introduce one `PreparedEvaluation` owned by the CLI evaluation path.
   Prepare data without evaluating repo code or provisioning/running
   repo-selected native plugins. Canonicalize/admit roots and create a
   private, content-addressed read-only bundle before prompting. Hash
   the bytes that were actually copied, not a first pass over mutable
   originals. Reuse audited root-capability/copy/hash primitives; the
   source-overlay machinery is a useful precedent, **not** an already
   safe evaluator-bundle implementation.
3. The bundle must cover the complete *admitted read set*, a conservative
   superset of imports: env.toml, selected host and modules, arbitrary
   repo-local files Deno may read, untracked/ignored imported files,
   import maps/config, the resolved pinned frontend and every allowed
   dependency root. `hosts/` + `modules/` alone is insufficient; static
   import-graph enumeration misses dynamic imports and filesystem reads.
   Publish an inventory of path/type/size/digest entries and aggregate
   digest, with bounded count/size/traversal and no raw content in logs.
   A deliberately excluded path must be unavailable to eval, not silently
   read from the worktree. Do not treat `.gitignore` as trust admission.
4. Copy files rather than hardlinking to mutable originals. Resolve
   in-root symlinks into admitted bundle objects and reject cycles,
   escapes, special files and unsupported imports; an external pinned
   frontend is an explicitly admitted separate root copied into the
   bundle. Dirty submodule content is captured as bytes, not merely a
   gitlink OID. Rebind imports, current_dir and Deno read grants to the
   bundle; omit the live repo and undeclared outside roots. Validate
   grant encoding (§1.1) on these actual paths. Do not fetch packages or
   fall back to ambient node_modules during evaluation. Keep source-map
   diagnostics in logical repo paths, not internal scratch paths.
5. All fixpoint rounds use this same bundle. Host/probe facts are a
   separate core-generated immutable input per round with their own
   digest and declared capabilities; do not silently fold volatile facts
   into the source approval. Config and pinned-frontend selection used
   after approval come from the captured bundle too. Inventory all eval
   callers (check/plan/apply/update/adopt/doctor and repo clones) and
   remove their ability to reconstruct an unapproved live frontend.
   Bound executable runtime identity via R5. Private bundle immutability
   protects against repo edits and sandboxed repo code, not an arbitrary
   privileged/same-UID attacker modifying trusted runtime storage; keep
   that host-integrity assumption explicit and revalidate cached bundles.
6. Version `trust.toml` admission and stored receipts; old path-only
   entries require explicit renewed approval, never get an invented
   digest/default capability grant. Add non-evaluating inspect/add/list
   operations that show bundle digest, inventory changes and actual
   grants; non-TTY add requires an explicit expected bundle digest.
   Changing source or adding grants invalidates approval. Relocation and
   re-clone at the same path have tested behavior: different bytes need
   approval; identical bytes/grants need not depend on branch spelling.
7. Remove the ambient blanket bypass: `GRIPSACK_TRUST_ALL=1` produces a
   migration diagnostic, not automatic approval. Migrate e2e helpers,
   demo/examples workflows and documentation to explicit digest-bound
   approval of their disposable reviewed fixtures. Do not infer trust
   from `CI=true`, a runner name or a checkout path; these are forgeable.
   Keep gate tests outside fixture auto-approval helpers. No compatibility
   shim that preserves path-only approval under the old env variable.
8. Emit a private versioned evaluation receipt containing bundle digest,
   root identity, informational Git metadata, frontend/runtime identities,
   actual grant policy, input-envelope digests and outcome. Never log
   secrets, raw source or credential-bearing remotes. This is not proof
   of source intent; it makes the approved/evaluated-byte boundary
   inspectable and allows R4 to describe the actual guarantee.

**Acceptance.** Extend `e2e/test_eval_gate.py` with real Git/worktree
fixtures and the real Deno entrypoint, not a mock approval echo:
staged edit, unstaged edit, untracked import, ignored import, symlink
retarget, dirty submodule with unchanged parent pointer, alternate
worktree, branch switch, re-clone at the same path, outside/generated
import, changed pin and added capability. Before renewed approval no
new bytes execute; after it, receipt and actual evaluated output agree.
Use a controlled pause after approval and between fixpoint rounds:
mutate the original repo/pin/symlink and prove eval uses the captured
bundle (or rejects before execution), never a mixed later worktree.
PermissionDenied/EIO during capture fails, not “empty/missing source.”
Non-TTY old trust/bypass fails with a migration path; explicit CI fixture
approval works. No test reads real HOME or sends real credentials.

**Dependencies / delivery.** M0; one owner with §1 env/grant/adopt fixes
and R5 evaluator supervision. R2/R3 consume the resulting receipt/grant
contract. Hash/recheck of a mutable tree, “clean git status,” and static
imports-only hashing are rejected substitutes, not fallback designs.


### R2 — Typed policy admission and extensions to the real refinement harness

**Evidence / disposition (F-02, F-03).** Accept the role/order risks.
`ownership.rs::plan_copy` takes three string roles, a positional drift
Boolean and takeover Boolean; `plan_link` takes four Booleans.
`retention.rs::plan_prune` relies on sorted input by comment; the current
producer sorts, so this is an API admission gap, not evidence of
wrong current production pruning. Inputs are already borrowed: no
owning-string refactor is warranted. Store already has `FileIdentity`,
`ManifestHash`, `BytesHash` and `PayloadHash`; reuse those producers.

Refinement is **not absent**: `ops/model.rs` materializes filesystem
states and drives the real planner/executor; `lineage_model.rs`,
`mode_model.rs`, journal explorers and persistence/journey e2e add
independent coverage. `ops/model.rs` also asks the real kernel for its
expected decision, which checks agreement but cannot alone expose
matching observer/oracle errors. Extend this infrastructure, do not
create a second model or test VM.

**Changes.**

1. In `gripsack-policy`, define zero-copy borrowed role types
   `DesiredIdentity`, `LiveIdentity`, `PriorManagedIdentity` and named
   `CopyFacts`/`LinkFacts`. Replace takeover with
   `TakeoverPolicy::{Deny, Allow}`. Encode lineage as distinct variants
   (last managed write versus preserved observed drift), not a raw
   `(identity, bool)` or a “managed” identity carrying preserved drift.
   Use a link-observation enum (absent/owned/foreign) plus explicit
   lineage authority instead of independent booleans that permit
   `absent && ours`. Keep the exact existing authority truth tables;
   do not silently turn a recorded-but-replaced link into refusal.
   A named struct whose fields are still interchangeable `&str`/`bool`
   does **not** meet the requested compile-time criterion.
2. Keep the policy crate independent of store (no dependency cycle).
   Adapters project existing typed producers into the role wrappers;
   arbitrary abstract identity strings remain usable in model cases.
   Types prohibit accidental argument/Boolean swaps, not a malicious
   caller deliberately labelling wrong bytes; observer correctness is
   still a separate obligation. This supersedes 0036's stringly-kernel
   exception; §10 also strengthens semantic identifier/unit types in
   touched lifecycle APIs. Plain display text/local indices remain plain.
3. Add a borrowed `GenerationInventory` over admitted `GenerationId`
   values, with private representation and a constructor checking
   **strictly** ascending order (therefore uniqueness). Use it in
   retention admission/pruning. The producer returns the typed inventory;
   validate once and borrow thereafter. Do not sort/copy per policy call
   or silently deduplicate corrupt input. If a real caller needs an
   owned constructor, sort its owned Vec in place. Migrate shared
   generation API consumers together; wire numbers remain compatible,
   with explicit admission/serialization rather than a raw-u64 bypass.
4. Preserve current keep/current semantics: pruning examines the oldest
   excess prefix, never removes current, and may therefore keep one
   extra. Cover no limit, zero keep, empty inventory, absent/missing
   current and integer boundaries. Existing Verus pruning contracts
   prove subset/current protection, not the entire “oldest exact set”
   rule; specify/prove that exact prefix result under the admitted
   sorted invariant before advertising it as verified retention policy.
   Reject invalid constructor inputs in both runtime and verified view.
5. Migrate ownership/retention kernels, `ops/plan.rs`, `gc.rs`,
   `deploy/mod.rs` re-exports, `lineage_model.rs`, `ops/model.rs`, direct
   policy tests and all callers in one change. Re-state biconditionals
   over typed fields, prove constructor invariants, and recalibrate
   `scripts/check_verus.sh` in the same PR. Do not leave a raw public
   alternate entrypoint, test-only algorithm, trusted-body shortcut or
   erased-wrapper proof bypass. Retain runtime behavior with proof
   erasure and the pinned Verus toolchain.
6. Extend `ops/model.rs` to owned/foreign/dangling links, journal lineage
   reconstruction, permission/missing/error classification, merge
   malformed/mode/foreign-byte paths, apply/rollback/prune and aliases.
   Generate concrete fixtures, run the production observer, capture
   typed facts and kernel result, run the real operation adapter,
   re-observe, and compare bytes/type/mode/authority/prior preservation
   to an oracle derived independently from fixture/edit history.
   Reuse journal `repeated_model.rs`'s independent-oracle precedent.
   For GC extend existing roots/build-closure properties; for scheduler
   use the existing journey/panic seam and §6's Loom bridge work rather
   than pretending ownership tests prove thread synchronization.
7. Add attributable seam mutants: wrong producer assigned to desired/
   live role; preserved drift promoted to managed lineage; foreign link
   labelled owned; PermissionDenied treated as NotFound; prior captured
   after a write; refusal executed as takeover; intended bytes/mode
   not landed; report built from stale decision. Raw argument swaps
   should be killed by compilation; deliberately mislabelled producers
   must fail the concrete oracle. Omitted parent sync belongs to the
   persistence/order and R6 campaign, not an ordinary process test that
   cannot observe power-loss durability. Every negative must fail at its
   named assertion/contract; unrelated compilation/runtime failure is
   not successful calibration. No source-text “calls this function” test.
8. Extend existing Op/journal/report records with one versioned policy
   receipt: operation/run IDs, canonical destination identity, admitted
   observation/error classification, named kernel facts, decision,
   authority and intended effect, actual completion/error and resulting
   identity. Generate human/JSON explanations from that semantic result.
   Refuse/preserve has a receipt too, without claiming an effect occurred.
   Persist necessary mutation intent before effect; record completion
   only after postcondition. No raw file contents or secrets; R5 privacy
   and bounds apply. Optional diagnostic trace loss is not durable
   receipt success. A richer pruning explanation is a projection of the
   one decision, not a new policy engine or second persisted plan.

**Acceptance.** Compile-fail doctests demonstrate that desired/live
roles cannot be exchanged, raw takeover Booleans are rejected, a
preserved observation cannot stand in for a managed lineage variant,
and a raw/unsorted slice cannot directly call pruning. Construction
rejects duplicates/unsorted IDs; a valid inventory retains exactly the
current policy behavior. `verify` discharges typed contracts and the
new sorted/exact-prune obligation; calibrated ownership/retention/seam
mutants fail by name. Existing lineage/mode/journal/journey tests plus
extended concrete harness agree on actual filesystem results and
receipts, including denied/error paths. No per-call identity allocation.

**Dependencies / delivery.** NEXT / M1 after M0 effect fixes; proof/API
and complete caller migration are one owner. R5 supplies privacy/process
policy, R6 persistence evidence and R4 release attribution. M-V1–M-V7,
including Loom/parser/root obligations, are mandatory before release;
§9 permits no automatic research or property-test fallback for them.


### R3 — Stable hook intent IDs, durable outcomes and safe duplicate simulation

**Evidence / disposition (F-05).** Accept ergonomic/evidence improvements,
not exactly-once claims. `store/activation.rs::PendingActivation` already
persists the generation and actions before the flip. `exec/activate.rs`
resumes the saved actions, never re-reads changed repo source. There are
no per-intent IDs/completion receipts; a mid-loop crash can repeat
completed effects. Normal adapter failures are warnings and the pending
record is cleared; this is **not** a guaranteed-success retry queue.
The run journal's exact transaction classifier must remain authoritative.

**Changes.**

1. Version the pending-activation format. Allocate a collision-resistant
   activation instance ID once and persist it before any hook effect;
   a target generation alone is not unique (rollback can activate the
   same generation multiple times). Derive per-intent IDs from that
   instance plus the canonical effective action/trigger/occurrence.
   Identical duplicate custom hooks remain distinguishable; built-in
   font/desktop coalescing creates one effective intent with explicit
   contributing modules. Persist the resolved order/digests, not a
   fresh index derived from whatever repo happens to exist on replay.
2. Pass reserved `GRIPSACK_ACTIVATION_INTENT_ID` and an attempt counter
   into each hook's explicit child environment. Attempt count may
   change; the intent ID never changes for a replay. Record command/
   script identity, trigger and R5 process receipt. User/repo env cannot
   override the reserved ID. Cover custom activation, service/cache,
   rollback activation and on_remove paths; preserve their existing
   timing and document which are recovery-replayed. Do not claim
   on_remove replay safety merely because activation has a record.
3. Persist per-intent states and outcomes through the existing atomic
   FS/journal primitives: pending, started/ambiguous, succeeded, failed,
   or superseded. Flush intent before launch; flush the recorded result
   after return. On crash, only a durable succeeded/terminal outcome
   suppresses replay; `started` is ambiguous. A crash after remote
   success but before local success durability can still repeat the
   effect with the same token. Do not record success before running.
4. Keep settled failure policy: known hook failure is a warning, never
   automatic generation rollback or a new unbounded retry loop. Record
   failure durably and expose it, rather than silently losing evidence.
   Resume interrupted work under the existing lifecycle/commit guard;
   superseded/uncommitted actions do not run. Do not change warn-and-no-
   retry into retry-on-every-command as a side effect of adding tokens.
   Describe the contract as `delivery: at_least_once` for replayable
   interrupted hooks, `failure_policy: warn_no_retry`, with no promise
   of eventual success. Supersession/cancellation is explicit.
5. Archive completed outcomes before clearing the pending record, using
   a small private versioned receipt under the existing activation
   state area; pending recovery evidence is not ordinary log retention.
   Add `grip hooks list --json` to inspect intent, attempts and terminal/
   ambiguous result without rerunning anything. Legacy pending records
   have no trustworthy instance ID: explicitly migrate and persist a
   new ID before first replay, recording `legacy_identity_unavailable`;
   never claim that token deduplicates effects that predate migration.
   Corrupt/unknown versions fail closed with an actionable diagnostic.
6. Add `grip hooks test --duplicate` and `--crash-after-start` as
   **fixture-only simulations**, reusing the production intent runner
   and existing crash seams. Default fixtures use throwaway HOME/state,
   known harmless scripts and loopback services; they never select the
   user's live hooks. Include a non-idempotent append/notification
   fixture to show the duplicate and a receiver that atomically commits
   token+effect to show correct receiver-side deduplication. Explain
   that a temp directory does not sandbox arbitrary native scripts;
   arbitrary user-hook execution is not a “safe test” offered here.
7. Provide runnable idempotency examples: atomic local replace, a
   cooperating local keyed operation with crash-safe state, and an
   external request passing the token to a receiver with atomic
   deduplication. No generic “write a done file after shell” wrapper
   claims exactly-once remote effects. Separate local activation and
   external notifications in examples and receipts; document operator
   reconciliation/compensation for ambiguous outcomes. Do not invent
   automatic compensating actions for arbitrary remote services.
8. Extend `specs/Activation.tla`, its calibrated negatives and existing
   activation/persistence e2e to model per-intent durable outcomes,
   stable replay identity and known-failure behavior. Retain the exact
   transaction classifier and two-barrier ordering. An ID alone is not
   evidence of receipt durability or receiver idempotency.

**Acceptance.** Exercise crash before launch, during hook execution,
after an observable remote success but before local receipt, during
receipt durability, between two hooks and after all receipts before
pending cleanup. Repeated recovery keeps the same ID, skips durably
completed intents and never runs superseded work. A second activation
of the same rollback target gets a different ID. Known hook failure
warns without rollback or automatic retry. Fixture append duplicates;
atomic receiver-side dedup produces one committed effect. Receipt
inspection remains correct after trace pruning and legacy migration.
Use `e2e/test_activation_lifecycle.py`, persistence matrix and the real
CLI simulations; do not merely assert that an env field was forwarded.

**Dependencies / delivery.** M1 with R5's reserved-env/supervision
contract and M0 private durable storage. No IR bump for a core-injected
token; if action syntax actually changes, migrate Rust/schema/TS
atomically. This packet does not authorize running real user hooks.


### R4 — Release-scoped assurance, coherent versions and verified installation

**Evidence / disposition (F-01, F-07, F-09).** Accept the missing
release-evidence binding, not a claim that verification is absent.
`verification/guarantees.md` already names properties, pinned tools and
excluded boundaries. `release-core.yml` attests platform tarballs and
checks the shipped in-binary SBOM; `repro.yml` keeps separate run
artifacts. They are not one release-scoped, machine-readable record.
Core and TS versions are independent by policy, currently both 0.42.0;
IR v3 is compatibility, not a requirement that every version equal the
CLI. `lockfile.rs::Lockfile` currently has no format-version field: do
not label it v3 merely because the IR is v3.

Additional source gap: the canonical site's
`website/build.py:390–395` fetches `main/install.sh`; `latest_release`
fetches live API state with a 0.8.0 fallback. Thus installer bytes,
banner and the workflow's 0.39.0 example-test pin can describe different
products. Remove these independent selectors, not just their strings.

**Changes.**

1. Extend the existing verification ledger, not a parallel database.
   Add versioned schemas for generated `assurance.json` and
   `release.json` under `schema/verification/` and `schema/release/`.
   Keep narrative claims/assumptions in one reviewed source and have
   the runner emit measured fields. Generate the public matrix from
   that data; check ledger IDs against emitted entries. Proposed rows
   are permitted in development docs, never as passing release proof.
2. Each assurance entry must include: stable property ID and precise
   claim; `claim_type`; source symbol/model and full source SHA;
   source/config/workflow digests; actual tool/compiler/solver versions
   and image/lock digests; bounds, fairness/admission/OS assumptions;
   excluded boundaries; job/run/attempt; result and coverage inventory;
   raw evidence artifact name/digest; calibration mutants with expected
   failing property, observed attribution, disposition and reason.
   Distinguish `model_checked`, `contract_verified`, `property_tested`,
   `refinement_tested`, `fault_injected`, `end_to_end_tested`, and
   `artifact_attested`. A retained counterexample is not a surviving
   valid implementation; unexplained surviving mutants block the
   affected claim. No global test/obligation count substitutes for
   named properties. “Not run” is not “passed.”
3. Extend `check_verus.sh`, `check_models.sh`, existing fault/fuzz/journey
   runners, `check_reproducible.sh` and CI artifact upload paths to emit
   their own bounded raw/summary evidence. Fix §6 calibration first.
   Attach evidence for the exact candidate SHA, including relevant
   runtime bridge cases; an old successful main run is inadmissible.
   Upload failure artifacts too, without credentials or real HOME data.
4. Derive release metadata from existing authoritative Cargo, npm,
   schema and embedded-frontend inputs; add only an explicit tested
   compatibility table where no machine derivation is possible. Record
   product/core tag and CLI version, every crate version, embedded SDK
   revision/digest, published npm version/integrity if applicable,
   supported IR versions, lock/journal/trust format identities and
   migration/refusal policy, platform targets and artifact digests.
   Record an unversioned legacy format honestly until a separately
   tested versioned migration lands. Fix doctor/release-skill advice to
   recommend a *published compatible* SDK, not `core.version` by guess.
   Validate package metadata before publication; do not try to generate
   pre-build Cargo metadata from a post-build signed manifest.
5. Use this non-circular artifact graph:

   ```text
   tagged source + pinned build/verification inputs
     -> tested binaries/packages + raw evidence + in-binary SBOM
     -> assurance.json (digests of evidence, not itself)
     -> release.json (digests of artifacts and assurance.json)
     -> detached signed attestations over the exact manifest bytes
     -> finalized GitHub release -> tap/site/channel consumers
   ```

   Reuse GitHub/Sigstore build-attestation infrastructure rather than a
   bespoke signature system. Verification must pin expected repository,
   issuer, release workflow identity/ref and subject digest; merely
   finding *an* attestation from GitHub is insufficient. Signature
   authenticates provenance, not correctness. Keep signatures outside
   their subjects; neither manifest contains its own digest.
6. Apply §7's topological publish order and fail-closed aggregator.
   Stage artifacts, validate all gates, publish packages idempotently,
   verify registry identities, then finalize the release. Consumers
   must see the complete verified manifest before updating. Resumption
   checks already-published package version/integrity, not an arbitrary
   “already exists” substring. A failed npm/tap/site step is recorded;
   never forge cross-registry atomicity or republish different bytes
   under an existing version. Attach reproducibility results and
   verifier logs to immutable release assets, not expiring CI links
   alone. Scheduled campaigns remain separately dated evidence.
7. Lead docs with a versioned download of manifest, detached bundle and
   payload, verified by an independently installed trusted verifier
   before extraction/execution. The initial supported verifier can be
   the existing `gh attestation verify` tool, with exact identity and
   ref checks enforced by the shared policy. If a required check is
   unavailable in the selected `gh` version, reject that version; do
   not weaken policy. Do not download an unverified verifier as a
   bootstrap shortcut. Reuse this policy in `install.sh` and
   `commands/self_update/`; missing verifier, unavailable evidence,
   failed identity/digest or a revoked release fails before replacing
   the binary or running its `--version`. This dependency and offline
   verification limits must be explicit.
8. Publish the versioned installer as a hashed/attested release subject.
   The site copies those verified bytes, never raw main. Curl-to-shell
   may remain separately labelled convenience: the bootstrap script
   itself is already trusted/executing even if it verifies its payload.
   HTTPS plus a checksum from the same origin is not publisher
   authentication. Stable selection excludes prereleases and withdrawn
   releases. Consume the last explicitly selected verified manifest
   offline or fail; never synthesize a fallback version on fetch error.
9. Coordinate R7's owner-run signer/release-revocation procedure. A
   locally valid historical signature cannot prove a release has not
   since been withdrawn. Online channel selection checks authenticated
   revocation state; offline instructions name their freshness limit.
   A signer-compromise drill includes an out-of-band trust-anchor
   replacement, not a revocation file authenticated only by the stolen
   authority. No custom PKI or new SBOM format is required.

**Acceptance.** Produce a non-publishing candidate release bundle from
the real build outputs. The consumer rejects wrong SHA/ref/workflow,
missing/corrupt evidence, a mismatched SDK, a tampered installer/tarball,
a wrong-identity valid signature, unexplained mutant survival and a
revoked version. A valid fixture installs and self-updates; each failing
fixture leaves the old binary usable and executes no unverified bytes.
Interrupt/resume each publication stage against disposable fixture
registries/storage; the site/tap cannot advance to an incomplete tuple.
Run final compose/native gates, package dry-runs and actual disposable
installer/self-update smoke flows. No production publishing in tests.

**Dependencies / delivery.** M2; consumes M0/M1 evidence and R5's bounded
process invocation. Main release owner controls schemas/signing policy;
installer/site owners consume them. GitHub protections, environment
approval, registry trusted publishing, token retirement and revocation
are **External** owner tasks with recorded evidence, not script claims.


### R5 — One native-process policy and a bounded private trace contract

**Evidence / disposition (F-10, F-12).** Extend existing infrastructure.
`gripsack-process` already supplies deadlines, process-group cleanup,
bounded input/frames/stdout/stderr and a retained stderr tail; its docs
explicitly exclude descendants that leave the group. Eval's
`probe.rs` uses raw `.output()` and hooks use `verify.rs::run_shell`
without that supervision. Plugin receipts ignore source on cache hits
(§2.2), and provisioned Deno/plugin executables are not rehashed before
use. `gripsack-trace` uses JSON escaping and atomic `latest` replacement,
but logs use default permissions and have no size/retention budget;
console child-output passthrough is not escaped. HTTP diagnostics
already contain useful URL-redaction logic. None of this proves a
Windows problem: native Windows is not a supported implementation here.

**Changes.**

1. Inventory real native entrypoints: evaluator, explicit probes,
   fetchers/linters, build recipes, verifies, activation/removal hooks,
   fact detectors and self-update tooling. Reuse `gripsack-process`;
   add the minimal bounded raw-output mode needed by shells/builds,
   rather than forcing their output into the plugin JSON protocol.
   Deadline includes resource admission, spawn, I/O and reaping. Apply
   §2.2's bounded acquisition wait. Preserve current protocol limits
   unless a measured legitimate consumer needs an explicit allowance;
   never turn output overflow into truncated successful JSON.
2. Introduce an admitted process description with absolute executable
   path, selected bytes digest, package/ref/provenance if known,
   working directory, explicit env-key policy, protocol version,
   deadline/output limits and enforcement level. Resolve PATH once
   using operator-owned inputs before repo env is considered; no
   implicit current-directory search. A user-selected PATH executable
   can remain an explicit lower-provenance choice, not a fabricated
   attested package. Shell invocation receipts bind interpreter plus
   script digest; spawned subcommands/dynamic libraries are outside
   that digest unless separately bound. Preserve “pinned, not hermetic.”
3. On cache hit check source+tag+digest, and rehash selected executable
   bytes before use; replacement/tampering fails, never silently refreshes
   under prior approval. Hashing a pathname immediately before
   `Command::spawn` is still a race. Pin a validated executable handle
   or execute from private immutable materialization with a supported
   OS identity-bound spawn mechanism; describe its host-integrity
   assumptions. Record which platforms truly bind launched bytes and
   which only revalidate. Do not claim the pre-1.0 exact-byte gate for
   an unresolved hash/reopen window. Reuse one adapter, not per-caller
   “secure spawn” variations. Include Deno overrides and native plugins.
4. Clear inherited environment, then construct per-role maps. Preserve
   required DENO_DIR/runtime, explicit operator HOME/PATH/locale/temp,
   proxy and CA behavior; build declarations reach build children as
   §1.2 requires, not the parent or credential-routing context. Ambient
   credentials/loader variables are denied unless a declared, operator-
   approved role explicitly needs them. R3 IDs override any hook env.
   Receipts list grant names/policy and redacted metadata, never values
   or secret-bearing argv. Native code still has user filesystem/network
   authority unless an actual OS sandbox restricts it: call that out;
   protocol `capabilities` metadata is not an enforced sandbox grant.
5. Close inherited descriptors except explicit stdio/protocol handles;
   verify with an intentionally non-CLOEXEC canary. Keep process-group
   kill/reap for ordinary Unix children. Add a Linux complete-tree tier
   only with a real delegated cgroup/containment facility and resource
   limits that contain setsid/double-fork descendants. If unavailable,
   reject a request for complete-tree containment or report the existing
   group-only tier; do not silently claim full descendant cleanup on
   macOS/unsupported hosts. A Windows job object belongs to a future
   Windows port, not this change. Bound parser nesting/frames, invalid
   UTF-8, partial-frame waits and both output streams. Keep cleanup
   deadlines even when the leader exits while a grandchild holds pipes.
6. Consolidate terminal escaping at the untrusted-output render boundary,
   reusing the trust prompt's control handling. Escape ANSI/OSC/BEL and
   other terminal controls from child stderr, paths and diagnostics;
   retain intentional renderer colors. Keep normal serde JSON escaping;
   do not invent a second JSON codec. Apply deterministic field-aware
   redaction based on the existing HTTP precedent. Never promise a
   regex can discover arbitrary secrets embedded in arbitrary prose.
7. Make `runs/` 0700 and logs 0600 regardless of umask, capability-rooted
   no-follow creation with create-new collision handling. `latest` may
   only reference a generated run basename under that root; cleanup
   must neither follow a hostile link nor remove unknown files. Reuse
   the existing atomic symlink helper, add admission around its inputs.
8. Establish operator-controlled diagnostic budgets: initial defaults
   64 KiB per event, 16 MiB per run, 256 MiB total, at most 100 completed
   runs and at most 30 days. Do not let repo config silently raise them.
   Bound before allocating/formatting huge fields; mark truncated
   diagnostic content explicitly. If the per-run limit is reached,
   terminate diagnostic logging with a bounded warning/incomplete marker,
   never corrupt a JSONL record. Prune only completed recognized runs
   under safe concurrency; preserve active runs and all pending
   journal/hook evidence. If active runs consume the quota, stop new
   diagnostic logging, not recovery evidence. Document when optional
   tracing failure is nonfatal versus mandatory intent/receipt failure
   preventing an effect. No automatic export/telemetry.

**Acceptance.** Extend `gripsack-process/tests/lifecycle.rs` and
`pressure.rs` plus real eval/hook/plugin CLI flows: oversized/partial
JSON, deep nesting, invalid UTF-8, stdout/stderr flood, ignored signals,
leader exit with inherited pipes, bounded fork pressure, setsid/double-
fork (only within a disposable constrained VM/container), inherited
secret/FD canaries, cached executable replacement and PATH/CWD shadowing.
Assert actual survivor/exit/state behavior and the recorded enforcement
tier, not just receipt field copies. Full-tree tests cannot pass by
ignoring escaped descendants. Evaluate real command output containing
OSC/ANSI and inspect terminal/JSONL bytes; private permissions hold
under permissive umask. Trace-cap/retention/latest-symlink attacks leave
external canaries, active logs and recovery receipts untouched.

**Dependencies / delivery.** §9's explicit split controls: NEXT includes
all R5 controls needed by R1/R2/R3/R4 and every required campaign over
shipped supported behavior; remaining caller/trace-lifecycle breadth is
M3. A failing required case promotes its prerequisite fix to NEXT.
Exact-byte/complete-tree claims remain platform-specific and gated on
actual evidence, never a silent weaker fallback or an agent waiver.


### R6 — Filesystem applicability, identity semantics and real power-cut evidence

**Evidence / disposition (F-06, F-13).** Accept a discoverable matrix
and missing metadata/alias qualifications. Current ledger/safety docs
already distinguish process kill, modeled persistence and physical
power loss. `gripsack-fs/src/fault.rs` kills a process; it does not power
cut a machine. `store/hash.rs` deliberately has two mode domains:
store/tree hashing tracks executability, while destination/journal
`canonical_bytes_identity` includes full permission bits (0o7777).
“Only executable mode is identified” is wrong for deployed files.
Existing types and tagged identities must not be replaced gratuitously.

**Changes.**

1. Consolidate the actual identity contract in the existing guarantee
   ledger and safety docs: file-byte versus full file identity, link
   target bytes, tree/overlay identities, type tags, mode masks and
   restore/fallback rules from `ops/plan.rs`/`verify_store.rs`. Explicitly
   distinguish identified, preserved and unsupported metadata. Ownership,
   timestamps, ACLs/xattrs/resource forks/quarantine are not guaranteed
   round-trip metadata unless code+evidence says otherwise; document
   Deno provisioning's deliberate quarantine handling separately.
   Atomic file replacement can sever a hardlink relationship; inode/link
   topology is not preserved. No new xattr/ACL hashing or wire/hash
   format migration merely to satisfy a broad wording request.
2. Document lexical case handling accurately: Unicode lowercasing is
   not full Unicode case folding or filesystem normalization. NFC/NFD,
   case aliases and hardlinked destinations need native observations;
   do not claim an APFS overwrite defect from source search alone.
   Extend the existing destination-key admission (§4.2) with concrete
   native alias fixtures. If two admitted destinations resolve to one
   physical object, reject before effects using the existing canonical
   admission seam, not a second spelling pass. Preserve on-disk spelling;
   do not globally normalize user filenames or silently retarget them.
   Unsupported ambiguous naming must be rejected or explicitly excluded
   from the support claim until tested. Keep the no-portable-CAS external
   writer window visible; canonical names do not close it.
3. Resolve dangling guarantee references such as
   `RECOVERY-IDENTITY-001` and the intended `LOCK-SESSION-001`: add their
   actual claim/evidence/status rows or replace references with existing
   rows. A named ID is not evidence. R4's ledger/schema validation must
   reject dangling published property IDs.
4. Add a machine-readable applicability matrix consumed by R4/R7:
   OS/kernel/runtime, filesystem/version/mount options, backing storage,
   local/network/synced/overlay classification; rename/replacement,
   cross-device behavior, file sync, parent-directory sync, locking,
   identity semantics, process-loss/VM-power-loss evidence with digests,
   date/revision and known exclusions. Separate `tested`, `assumed`,
   `experimental`, `unsupported`, `not_run` dimensions; runner OS is not
   proof of its filesystem or physical flush semantics.
5. Initial documented envelope: Linux local ext4 and macOS APFS are
   candidates for qualification, not automatically power-loss certified.
   Linux inside WSL on its own ext4-backed disk is distinct from
   `/mnt/c`/DrvFS/NTFS. NFS, SMB/CIFS, FUSE, overlay, synced/network homes,
   XFS and Btrfs receive only the evidence actually collected; no broad
   support inferred from POSIX-looking APIs. Runtime mount detection
   may report an unknown/experimental result, never “durable” from one
   successful fsync. `doctor` can expose the detected applicability and
   its limits; do not reject previously working mounts merely to invent
   a new product requirement. Native Windows remains unsupported.
6. Extend the recorded-boundary fault/persistence campaign, not a new
   transaction engine. Exercise ENOSPC, EDQUOT, EIO, ESTALE, read-only
   remount, lock contention/stale lock state, parent replacement and
   EXDEV across generation publish/current flip, journal/prior/receipt
   durability, rollback/prune and executable publication. Record the
   exact syscall/phase and expected old/new/ambiguous durable state.
   Failed persistence must not become successful commit/cleanup.
7. Provision disposable VM images on an owner-approved dedicated runner.
   Drive the real binary to a named durable transition using an external
   control channel, abruptly terminate VM power without a guest shutdown,
   restart the same disk and run real recovery/re-observation. Record
   guest FS/mount/kernel and host hypervisor/cache/flush settings; random
   timing complements, not replaces, named-boundary cases. Calibrate a
   dropped-sync mutant against an appropriate reordering/storage fault
   tier; a lucky VM power-cut pass does not prove the mutant harmless.
   No destructive mount/power-cut experiment on developer/user disks.
8. Attach campaign receipts to R4. QEMU/VM abrupt power-off tests the
   configured virtual storage stack, not every physical controller's
   power-loss behavior. APFS qualification needs a legally provisioned
   macOS-capable test environment; absent infrastructure is an explicit
   External blocker to that claim. Do not substitute SIGKILL/TLC and
   mark a power-loss row green. Expand filesystems only after observed
   results; no requirement to support every mounted path before 1.0.

**Acceptance.** Existing identity/mode tests remain unchanged in
semantics; chmod-only destination drift remains preserved, and payload
hash normalization remains intentional. Native alias/hardlink/metadata
fixtures demonstrate the documented envelope (including refusal and
non-preservation), not inferred guarantees. Fault cuts preserve pending
evidence or a recoverable old/new generation and never falsely complete
an effect. For each advertised durability combination, archive VM run
configuration, seeds, cuts, recovery results and digests. Untested rows
remain untested on the website; no power-loss claims from this handoff.

**Dependencies / delivery.** NEXT includes applicability/identity docs
and all applicable named software-fault/process-crash verification.
Additional native naming support is M3; VM/mount evidence is CLAIM-GATED
in M5, required before the associated claim and for the declared pre-1.0
support gate. Missing owner infrastructure blocks that claim; no agent
may substitute SIGKILL or a model result for VM/hardware evidence.


### R7 — Current public contracts, security governance and assurance website

**Evidence / disposition (F-07, F-08, F-14, F-15, F-16).** Correct stale
surfaces, not the implemented architecture. Both the local
`crates/griplint/src/lib.rs` and published
[griplint 0.42.0](https://docs.rs/crate/griplint/0.42.0) describe the
shipped checker. The reviewer quoted the historical
[0.23.0 page](https://docs.rs/crate/griplint/0.23.0), which really does
say “lands next.” There is no missing-checker implementation to build.
Local org-profile still advertises Python/TypeScript; local site
workflows pin old example-test artifacts and floating actions. No
repository security policy was found in the inspected local core/site/
profile roots; private-reporting settings and live domain ownership are
not established by those files. §5 already lists actual API-doc drift.

**Changes.**

1. Complete §5's documentation corrections and add small rustdoc
   examples at the public policy, lint, FS and trace entrypoints that
   explain invariants, admission errors, ownership and exclusions.
   Keep plan IDs as supplementary rationale, not the API contract.
   Document typed policy construction after R2, including invalid-input
   and preserved-drift examples. Run examples with the existing doc
   harness; do not generate vacuous examples or chase coverage percent.
2. Treat historical docs.rs versions as immutable history. Fix current
   links and metadata descriptions that still say “move 3”/“lands next”;
   publish corrected docs through the normal package release. Do not
   yank a working old package to remove stale documentation. Complete
   §5's lint protocol schema and E6/E9 conformance gaps rather than
   creating another checker. Expose an offline `grip lint list --json`
   inventory of built-in packs and supported protocol versions; loading
   configured native plugin capabilities is a separate explicitly
   trusted operation under R5, never arbitrary code execution just to
   list metadata. Unknown versions fail deterministically; document
   which checks run before effects and where plugin failure is a
   warning versus a blocking diagnostic.
3. Make TypeScript/Deno the only supported frontend in active docs,
   org profile, examples and compatibility pages. Mark Python packages
   and repos historical with migration directions and no current
   compatibility promise; do not remove Python *transport plugins* or
   conformance tools merely because Python is no longer a frontend.
   R4's tested compatibility tuple drives SDK recommendations. Complete
   §8's scoped ecosystem changes; archival/yanking/deletion needs
   separate owner authorization and is not an implementation side effect.
4. Add `SECURITY.md` with the actual supported core/SDK release lines,
   private-reporting route, what to include, coordinated disclosure,
   artifact/provenance checks and revocation guidance. Proposed alpha
   support policy: fixes on the latest released compatible product
   tuple only, no implicit LTS/backport promise. The owner must ratify
   this and enable/verify GitHub private vulnerability reporting (or
   supply another real private route) before the release gate clears;
   no placeholder email or invented response SLA. Document who may
   publish, who can approve emergency releases, least-privilege grants,
   credential rotation and the R4 revocation drill. A single-maintainer
   project must describe its actual roles, not pretend two-person review.
5. Canonical website: `../gripsack-dev.github.io` per AGENTS.md. Update
   its `website/build.py`, `website/index.html`, `doc/`, and
   `.github/workflows/pages.yml`; consume R4's verified selected release
   and generated assurance matrix. Home page links directly to the
   threat model, property evidence, R6 support/identity matrix, verified
   install, frontend lifecycle and private security reporting. Label
   unknown/unrun boundaries prominently; no aggregated proof badge.
   Pin the site actions/dependencies like core. Use the exact same
   tuple for banner, examples, install commands and footer; remove
   mutable-API/fallback selectors. Verify live domain assignment before
   disabling the duplicate `gripsack-site` Pages workflow (**External**).
6. Gate actual website behavior: build the candidate site, run executable
   examples against the candidate tuple, check links and use a real
   browser for desktop/mobile layout, keyboard focus/navigation, contrast
   and the install/assurance path. Add a pinned accessibility scan and
   reproducible performance budget (record runner/network settings and
   justified baseline). Inspect deployed HTTP headers, not HTML source
   alone. Set enforceable CSP/referrer/MIME policies and document hosting
   limits. GitHub Pages may not allow required response headers;
   escalate a hosting/configuration change to the owner rather than
   treating a meta tag as `frame-ancestors`/HSTS enforcement.

**Acceptance.** Current published docs and core smoke examples show the
real checker and typed API; a broken config produces the documented
observable diagnostic. Inventory agrees with shipped packs/protocol
versions without executing a native plugin by surprise. Homepage version,
artifact links, examples and compatibility all resolve to one verified
manifest; no active page advertises the removed Python frontend. The
owner can submit a private report and complete a dry-run revocation;
record actual results. Browser verification and deployed-header evidence
are attached to the site release, with unsupported controls explicit.

**Dependencies / delivery.** §9 fixes the NEXT active-doc/security/site
scope and required browser/example/header evidence. M3/M4 may add
inventory convenience, broader examples, ecosystem conformance and
regression automation, never defer verification of a surface changed
now. Owner action is required for publishing/domain/archival/yanking;
this planning document authorizes none of those operations.


### R8 — Honest update survey coverage, not a pretend effect sandbox

**Evidence / disposition (F-11).** Existing behavior is intentional:
`commands/update.rs` renders a complete survey with 0/1/2 exits;
`update/prepare.rs::PreparedUpdate::acquire` invokes the real
`source/preflight.rs::inspect`. `LayoutEvidence` already records checked
paths and deferred recipe/runtime checks. Both local and
[public settings reference](https://gripsack.dev/docs/settings/reference.html)
explicitly exclude recipes, hooks and verifiers. Accept structured
coverage; reject the suggestion that no validation happens. A blanket
`validation_performed: false` is false for sema/source-layout admission.

**Changes.**

1. Land §3.2's shared equality/publish decision first. Preserve Check's
   all-results/error-precedence and Publish's all-or-nothing lockfile
   contract; do not rebuild two subtly different summary paths.
2. Extend `report.rs`, `LayoutEvidence`, `commands/update.rs` and the
   Update CLI arm with a versioned `--json` report rendered from the
   same report values as text. Include selected module outcomes,
   coverage categories (IR admission, source-layout paths, recipes,
   executable verifiers, deployment, hooks), each with
   `performed|deferred|not_applicable|failed` and reason/count where
   applicable. An optional `full_validation_performed` must be false
   for survey mode; do not call an unavailable check passed. Schema and
   terminal renderer consume one semantic result, not duplicate logic.
3. Keep the distinction between source layout and applying native
   effects prominent in help and receipts. Network acquisition, runtime
   provisioning, trust checks, throttles and log bookkeeping may occur;
   “no publication/deployment” is not “zero side effects.”
4. Do **not** add `update --preflight` that runs arbitrary recipes,
   verifiers or hooks in a temp directory and calls it isolated. Those
   programs can mutate real HOME/network/remote state. A future explicit
   build-and-verify mode requires a genuine restricted runtime (no real
   HOME, bounded resources/descendants, read-only inputs, separate
   disposable output tree, explicit network/secrets grants), reuses the
   production recipe/verify pipeline and records all omitted checks.
   Unsupported isolation must fail, not fall back to a normal subprocess.
   Activation hooks remain excluded; they require R3's deliberate
   fixture simulation. This is a separate product proposal, not a
   release-blocking defect or accepted implementation packet here.

**Acceptance.** Extend `e2e/test_update_surveys.py`,
`test_migration_feedback.py` and `report::update_model` only for the new
observable contract: text/JSON agree on outcomes and exit 0/1/2; a
missing source path is reported as a failed performed layout check; a
recipe-built path is deferred; a canary verifier/hook never runs;
lockfile/source cache/destinations remain unpublished. Exercise a
partial-failure survey and no-source module. Fix wording-only assertions
rather than pinning new prose; keep tests about classification and state.

**Dependencies / delivery.** New structured-output behavior is LATER / M3
and must land with its full CLI/schema/campaign evidence. Existing
source-layout validation and survey behavior remain verified by their
actual runtime/model checks now; this is not a claimed formal proof of
the source observer. Arbitrary-effect preflight needs a separate accepted
runtime contract; never ship an inert flag or pretend isolation.

## 14. Live execution/evidence record — M0 evaluator/host admission

This section starts the §9 leaf-level record; it does **not** mark
other §1–§8/R1–R8 or M-V1–M-V7 leaves complete. The original
edition-5 handover remains checksum-covered outside this repository,
and `verification/delivery.json` still registers all 178 IDs. NEXT
leaves lacking evidence remain release blockers, not deferred work.

| Leaf | Class / target | Prerequisite, owner and implementation | State / evidence | Unmet acceptance |
|---|---|---|---|---|
| M0-1.3a | NEXT / M0 | None; `gripsack-ir` owns the private `HostName` constructor and E132 allocation, reusing E116 safe-segment grammar. `grip init` and eval share ASCII-safe default sanitization; no IR wire/version or store-hash change. | **Implemented-unverified** at `681e87b`: committed-source Rust host-rule regression 1/1, registry fresh at 37 core IDs and five frontend allocations. | No theorem for arbitrary source/OS path resolution; protected CI and native Mac unrun. |
| M0-1.3b | NEXT / M0 | M0-1.3a; CLI `eval_repo` admits selected `--host` > `env.toml` default > sanitized name before throttle state, plugin/deno provisioning, build-env injection and frontend/lockfile I/O. `EvalOutcome`, executor `Ctx`, lockfile path/read/write, preview, and linter pin lookup carry `HostName`; lint uses the evaluated selection, not the optional raw flag. No unchecked overload. | **Implemented-unverified** at `681e87b`: original traversal and absolute-host witnesses failed-before; committed-source lockfile roundtrip 1/1 and three real CLI selection/boundary cases passed. Host selection re-exercised at `0b92905`. | Direct host boundary lacks protected CI/native Mac evidence. The separate §1.4 output bound is now implemented-unverified at `0b92905`; the R5/M-V7 process/correspondence campaign remains open. |
| M0-1.3c | NEXT / M0 | M0-1.3a; `adopt` validates `--host` before inspection, payload generation and host-file modification, then passes the admitted name through eval and scoped apply without a second string copy. | **Implemented-unverified** at `681e87b`: failing-before fixture wrote repo files and reported a missing `hosts/../modules/evil.ts`; committed-source e2e rejects E132 before generated repo writes. Host rejection re-exercised at `0b92905`. | Protected CI/native Mac unrun. The separate §1.4 trust-gate reordering is now implemented-unverified at `0b92905`; R1 source-bound approval remains open. |
| M0-1.3d | NEXT / M0 | M0-1.3a–c; sandboxed offline real CLI cases for `../modules/role`, absolute victim lock, `adopt --host ../…`, valid `role.dev` and default-host version-aware lint. E132 has terminal and `check --json` parity; no native worker/host path is inferred from Linux alone. | **Implemented-unverified** at `681e87b`: SHA-256-checked focused report binds 7/7 distinct checks (`verification/reports/2026-09-26-m0-host-681e87b.log`, SHA `65a710e52fd7d343b96aa3d4c34baa7138b73ed3c15129ff8e585f880680636c`). | Exact-commit protected `test`, native Mac and full M0/M1/M2 release evidence unrun; no waiver. |

### M0 §1.4 — supervised frontend and adopt trust

Responsibility/dependency map: `gripsack-process` owns process-group
termination, deadlines and byte/line ceilings; `commands/frontend.rs`
owns sandboxed command construction, bounded capture and JSON line
reconstruction; `commands/probe.rs` owns one fixpoint deadline, parsing
and diagnostic/error precedence; `commands/adopt/mod.rs` owns trust
admission before repository inventory/generation. The existing
`commands/mod.rs::trust_gate` remains the single trust policy. No
IR/wire/schema change or second process supervisor.

| Leaf | Class / target | Prerequisite, owner and implementation | State / evidence | Unmet acceptance |
|---|---|---|---|---|
| M0-1.4a | NEXT / M0 | Existing bounded `gripsack-process::run`; `commands/frontend.rs::run_bounded` uses the supervisor's 16 MiB stdout/stderr, 64 KiB retained tail and full-stdout JSON-line ceiling; `commands/probe.rs::eval_to_fixpoint` gives all probe rounds one ten-minute budget, checks StopReason before parse and retains bounded traceback/structured-envelope semantics. No new wire type. | **Implemented-unverified** at `0b92905`: the previous real CLI consumed 16 MiB+1 stdout/stderr before reporting malformed JSON; the exact committed-source report below exercises both limit failures, the real-Deno two-round probe and the five inherited host paths (12/12 including two Rust cases). Precommit local Rust/e2e/Verus gates passed. | Protected exact-PR-commit Linux/native Mac and required R5/M-V7 identity/FD/grant/descendant/trace/proof campaigns unrun; no filesystem/OS scheduling theorem or release authorization. |
| M0-1.4b | NEXT / M0 | Existing trust policy; `adopt` calls the gate after repo-shape and typed-host admission, before target inspection, managed-state reads or repo writes. Existing trusted adoption still works; `--yes` does not waive trust. | **Implemented-unverified** at `0b92905`: before the fix, non-interactive untrusted `adopt --yes` changed the host file then returned a trust hint; committed-source e2e now rejects without generated files, preserves original target and passes trusted adopt→rollback. | Protected exact-PR-commit Linux/native Mac unrun; R1 source-bound trust migration remains a separate NEXT blocker. |

The exact committed-source report
`verification/reports/2026-09-26-m0-boundary-0b92905.log`
(SHA `2048aa748eb5f867ec0412aa5fea74b3007468711be7062e0b4606ef048dca71`)
binds **12 executed / 12 passed / zero failed or skipped** to
`0b929052974b63ed7283b086017bff369b9f1a57`, with clean tracked
source roots and SHA-256 source fingerprint
`a53a49a4d505774fa43f1882515e4ceeb15890c3542c635deaaad1575c862b6b`.
It includes two Rust tests, ten sandboxed real CLI cases (two hostile
frontend outputs use an adversarial fake Deno solely to exercise the
supervisor; normal probe and adopt paths use real Deno).
The dirty-worktree five-gate archive
`verification/reports/2026-09-26-m0-supervision-precommit-five-gates.log`
(SHA `b47fcb9a803481d70454119bea308fed210f34ce6241485fdff4d30a84d77192`)
records fresh Rust fmt/clippy/full tests, real e2e **307 passed**, fresh
Verus **72/0** with seven mutants, and `ts-test`/`model` gate passes
without fresh RUN output (**[INFERENCE] cached layers**). This is
*not* exact-commit protected CI, a native Mac/VM result or a required
R5/M-V7 theorem.

The earlier `681e87b` host-only committed source-root fingerprint is
`11419c4e1d532fea98b8b2e687f0fed60d964d9388413f4111dfd9a53971968c`.
The pre-commit worktree five-gate transcript
`verification/reports/2026-09-26-m0-host-precommit-five-gates.log`
(SHA `9249ecbb2e1a0a39bb5d0360bd93d90391641c708100a0e89e72e072e281e5ce`)
records fresh Rust fmt/clippy/tests, real CLI e2e **304/304**
and fresh Verus **72 verified / 0 errors** with seven mutants;
TypeScript and TLC images were cached. Its source equivalence to
`681e87b` is **inferred** from no behavior-root edits between that
run and the source commit; it is *not* exact-commit CI evidence.
The separate focused report above ran on the committed source.
The original bundle checksum check passes all eight members; live
delivery inventory has **178/178** original IDs with registered
platform/case/proof/evidence-kind inventories; 160 pending, 11
in_progress, six implemented_unverified and one blocked. H0-02 still
requires source-bound review and stronger kind/proof enforcement before
closure; H0/global and full M0–M2 release remain blocked.

Code-quality ownership: the domain type lives in `gripsack-ir`, the
selected-host precedence in one CLI eval boundary, lockfile path
construction in `gripsack-exec`, and lint uses that admitted identity
without a second validator. These M0 evaluator leaves are only a
subset of NEXT; protected branch aggregation, the remaining §9 leaf
expansion and all other required plan 0048 packets remain open. No
release is authorized.

### M0 §1.1 — comma-delimited Deno read grants

Responsibility/dependency map: `commands/frontend.rs` alone admits
repo, inputs directory, embedded frontend and canonical optional
`@gripsack/core` pin paths before joining Deno's `--allow-read` list.
`gripsack-ir` owns the core-only diagnostic allocation; `probe.rs`
renders the admission failure through the existing sink before any
child spawn. The existing pinned-package name check and the
`gripsack-process` supervisor keep their separate invariants. No IR
wire change, second grant builder or permissive path fallback.

| Leaf | Class / target | Prerequisite, owner and implementation | State / evidence | Unmet acceptance |
|---|---|---|---|---|
| M0-1.1a | NEXT / M0 | Existing plan/0013 D2 permission boundary and E132 registry; `schema/diagnostics.json` allocates core-only E133 for an unsafe comma-containing read-grant path. Generated Rust has 38 core IDs, five existing TypeScript frontend allocations; no IR wire/version change. | **Implemented-unverified** at `953724a`: Docker Rust generator freshness/mutant calibration passed; terminal and `check --json` both report E133 naming the actual refused path. | Protected exact-head CI/native Mac and complete release matrix open; diagnostic does not prove filesystem containment or Deno internals. |
| M0-1.1b | NEXT / M0 | One `commands/frontend.rs::admit_read_grant` checks the repo, inputs, embedded frontend and canonical optional pin before interpolating `--allow-read`; typed `FrontendRunError::Grant` reaches `probe.rs`'s existing sink before spawn. A valid out-of-repo pin still earns only its named package grant; no alternate unchecked command caller. | **Implemented-unverified** at `953724a`: real old-source comma-pin witness read an outside canary and exited 0; new real CLI returns E133, while one committed-source Rust test covers all three built-in grant positions and normal pin tests stay green. | Race/OS path-resolution limits, R1 approval and the other M0 evaluator flaws remain open; native Mac/VM and protected full CI are separate. |
| M0-1.1c | NEXT / M0 | Sandbox HOME offline real-CLI pin canary, comma repo and frontend home; ordinary external package pin and nonpackage-symlink controls; actual Deno driver pin matrix and JSON parity. | **Implemented-unverified** at `953724a`: the source-bound local report below binds one Rust, five real CLI and twelve real Deno driver checks (18/18); dirty/live build five-gate transcript shows fresh Rust, e2e 311/311 and Verus 72/0 with seven mutants, but cached TS/TLC layers. | Exact-head protected Linux/native Mac check and required R5/M-V7 proof correspondence unrun; no claim that §1.2 or the whole handover/release is verified. |

The committed-source `953724a7acaa9e314f145175dee35026f6052f3d`
focused receipt is
`verification/reports/2026-09-26-m0-grants-953724a.log`
(SHA `861c6dce2000c764e77a54244ddbafa3cd812c9dbcd4b18d189114c61a61fbc7`,
SOURCE_ROOTS fingerprint
`68ef13d7676043c886626237cff226afdb9d0529f80b4f449fb83f13787e514d`):
**18 executed, 18 passed, zero failed/skipped**, with clean tracked
source roots before/after. One Rust unit covers the three mandatory
grant positions; five real CLI cases cover the original comma-pin
canary, a comma repo and frontend home, a valid external SDK pin and a
nonpackage symlink; twelve Deno driver tests exercise ordinary pins.
The original bug is a read-grant escape, not proof of a filesystem
write or credential theft. Its previous real-CLI canary check exited
0; afterward terminal/JSON E133 abort before frontend eval.

The local five-gate archive
`verification/reports/2026-09-26-m0-grants-local-five-gates.log`
(SHA `8efaa16213c359407583dd8fcdadc66d84277771d7021db13e52e5c0ec7c5a82`)
observed fresh Rust fmt/clippy/tests, **311/311 real e2e** and fresh
Verus **72/0** with seven named mutants on the `953724a` source tree;
TypeScript and TLC RUN layers were explicitly **CACHED**. The full
job did not record a runner-time source-clean assertion, so
source-equivalence is **[INFERENCE]**, not exact-commit protected CI;
the separate focused receipt supplies the committed-source boundary
execution. Neither report qualifies native Mac/VM, TLAPS or release.

### M0 CI advisory — independently discovered release blocker

Draft PR #164's protected `audit` job at merge SHA
`cf6ab9c963cbd0ba3dc9cb00e92e651c84130798` failed in
[`cargo audit`](https://github.com/gripsack-dev/gripsack/actions/runs/36222426178/job/108350065391):
the existing `rustls 0.23.43` lock has
[RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html),
whose minimum patched release is 0.23.45. This is a real failed
release gate, not an optional scan or waiver. `gripsack-fetch` is the
direct TLS owner; the same rustls-only feature policy and ureq caller
remain. `crates/gripsack-fetch/Cargo.toml` now requires rustls at
least 0.23.45; a targeted Docker-builder
`cargo update -p rustls --precise 0.23.45` changed only the locked rustls
crate. The protected [`audit` job](https://github.com/gripsack-dev/gripsack/actions/runs/36223635393/job/108353417198)
then **passed** at PR merge SHA
`c833a5188586869194b0a45b3ff0bd7e16fe8fe3` containing source
`953724a`. The local five-gate archive above passed with fresh Rust,
real e2e 311/311 and Verus 72/0; TypeScript/TLC layers were cached.
Separate reviewable hotfix PR #165 carries only the patched dependency
to main. Protected `typescript-env` still fails on the old unpinned
external Pixi ripgrep tree (`plan/0049`, A3-01), not a reason to ignore
its hash. Native Mac/VM and other release evidence remain separate.
No public release is authorized by the patched audit result.

### M0 §1.2 — repo environment, credential audience and TLS boundary

Responsibility/dependency map for the next executable packet:
`gripsack-config` admits declared build-time variable names;
`gripsack-exec::facts` detects operator-owned facts before repo
configuration may influence a subprocess; `gripsack-fetch::http`
captures operator-owned credential audiences and sends bearer headers
only to HTTPS origins; `gripsack-exec` and fetch child-process owners
attach admitted build/proxy/CA variables to their commands rather than
calling process-global `set_var`; `commands/eval.rs` orchestrates
these handoffs and never owns a second HTTP policy. No IR wire change;
the credential-routing TLA+ model and real adapter must agree.

| Leaf | Class / target | Prerequisite, owner and implementation | State / evidence | Unmet acceptance |
|---|---|---|---|---|
| M0-1.2a | NEXT / M0 | Facts are captured in `commands/eval.rs` before any repo build overlay; the operator environment chooses `facts::glibc_version`'s `ldd`, never repo `PATH`. | **Implemented-unverified at `1c693ef`**: the original real-CLI `PATH` shim executed and wrote a marker; the committed CLI's Linux `grip check` reports zero modules because fake `glibc-999.0` is not selected, and the marker is absent. Exact clean-source 12-test/four-model receipt below includes this case. | Native Mac facts parity, protected CI and R5/M-V7 process/identity evidence remain open. |
| M0-1.2b | NEXT / M0 | `gripsack-config::parse_env_as` rejects both GitHub host audiences and four token names alongside `GRIPSACK_*` with E400 and a source span; `http::Policy` reads operator credentials, not the repo build overlay. | **Implemented-unverified at `1c693ef`**: before the change a repo `GH_HOST=127.0.0.1` sent a dummy operator token over local HTTP and `grip update` exited 0; after admission terminal `update` and JSON `check` both report source-labeled E400 before any request. All six names pass the direct Rust admission unit in the source-bound receipt; the final Rust Docker gate passes. | Native Mac, protected exact-source CI and global credential/IR proof remain open. |
| M0-1.2c | NEXT / M0 | `FetchContext` owns `BuildProcessEnv`: artifact HTTP captures repo proxy/CA, and selected build/verify/git/pixi/plugin child commands receive explicit env maps. Provisioning, core facts and Deno retain operator env; per-step overrides and dependency-closure PATH win over the base overlay. | **Implemented-unverified at `1c693ef`**: original `std::env::set_var` loop removed; direct committed-source real build shell, structured run PATH, plugin discovery/capabilities/fetch, proxy/NO_PROXY and Deno-wrapper isolation probes pass. The complete final-source Linux Docker e2e suite passes **319/319**; a direct TypeScript/frontend run separately passes **64/64** with strict examples typecheck. | Native Mac and R5/M-V7 process-boundary proof remain open. |
| M0-1.2d | NEXT / M0 | `http::Policy` matches credentials only on HTTPS; the HTTP client refuses an initially bound non-HTTPS public/enterprise or explicit registry authorization before network, and never forwards auth on redirect. The dummy enterprise fixture now uses a separate offline TLS CA/leaf pair instead of shipping any loopback authorization exception. | **Implemented-unverified at `1c693ef`**: clean-source direct runner includes real enterprise TLS update→cold apply, wrong- and same-host redirects without bearer forwarding, repo-scoped CA/proxy, bound HTTP zero-request refusal and registry-auth Rust unit; the final Docker Rust fmt/clippy/tests gate passes. | Native Mac TLS, protected CI and a general TLS proof remain open. |
| M0-1.2e | NEXT / M0 | Join real PATH/GH_HOST/HTTP probes, ordinary child env, E400 terminal/JSON, production host selector and `CredentialRouting.tla` operator-audience/no-cleartext transitions with a named repo-audience mutant. | **In progress**, not an M0 or release closure: source-bound report `verification/reports/2026-09-26-m0-env-1c693ef.log` at commit `1c693ef` and fingerprint `9a82d56b48bef69d8a57c831a3efa16638123279150689d277ecdd8a81447825` binds two Rust and ten real CLI tests **12/12**, plus direct full TLC with credential positive/three named mutant cfgs **4/4**, zero skipped. The same source passes full Linux e2e **319/319**, Rust fmt/clippy/tests, direct Deno **64/64** plus examples typecheck and fresh Verus **72/0** with seven policy mutants; four full-gate outputs have only inferred commit attribution (candid archive `verification/reports/2026-09-26-m0-env-local-five-gates.log`). | Dispatched native Mac CI `36227812302` remains in progress, not protected branch enforcement or Mac-VM; R5/M-V7 proof-to-effect boundaries, other NEXT work and original 178-ID handover remain blocking. |
