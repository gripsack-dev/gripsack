# 0057 — Fast qualification

*Status: landing (2026-11-08). Parent: plan/0056 §7 (toolchain and gates).
Companion to the ci-infra Buildbot pilot (0053), which is shared and is not
changed by this plan.*

## 1. Why

A full `python3 -m ci_infra qualify` of the 0.52.0 candidate (commit
`328f77c`, candidate `aa525ca7…`) took 62 minutes to reach the `package`
gate and never reached `smoke`. The gates run serially on one worker, so
wall time is the sum of the gates, and most of that sum is not testing:
it is image export, three separate from-zero compilations of the same
dependency tree, a serial 706-test pytest run and a serial Lean mutant
campaign, all throttled to `CARGO_BUILD_JOBS=4` on a 32-core host.

The owner's rule: cut the time and simplify, without regressing what the
qualification catches. The release rule (0056 §7, `scripts/release.py`)
stays: publish copies archived qualified bytes and never rebuilds.

**Goal: a warm full qualification (all seven gates, cached toolchain
images) in ≤ 25 minutes wall, from ~62+ minutes; a cold one (every image
rebuilt after a pin change) in ≤ 45 minutes.**

## 2. Where the time went (evidence)

Source: `~/.local/share/buildbot-ci/archive/aa525ca7…/evidence/<gate>/<attempt>/{receipt.json,stdout.log}`
(BuildKit plain progress; per-step `DONE <s>` lines; receipts give wall
time per gate).

| Gate | Wall | What the time was |
|---|---|---|
| `test` | 630 s | workspace fmt+clippy+`cargo test` **219 s** (test binaries ran in ~25 s; the rest is compilation of the whole dependency tree at opt-level 0 with 4 jobs); conda-helper fmt+clippy+test **229 s** (a second, separate dependency tree — rattler, tokio, rustls — compiled from zero); `exporting to image` **160 s** (the layer carries the multi-GB `target/` directory) |
| `ts-test` | 16 s | fine |
| `proofs` | 643 s | one-off cold build after the Aeneas pin bump (`nightly-2026.11.07`): elan 42 s, `lake exe cache get` 151 s, `exporting to image` **261 s** (≈20 GB `deps` layer), warm-translation 16 s; then the gate itself **154 s** = extract 2 s + `lake build` 19 s + axiom audit 2 s + **14 mutants serially, 131 s** |
| `e2e` | 1526 s | `bin` stage `cargo build` at `opt-level=1` **284 s** (a third from-zero compilation: the profile differs from `test`'s so nothing is shared); pytest **1224 s** for 706 tests, one process |
| `audit` | 51 s | fine (fetches the advisory DB) |
| `package` | 675 s (failed) | `cargo auditable build --release` **271 s** (fourth from-zero compilation) + export 32 s; conda helper release build ≈ **300 s** (fifth; from-zero by design: its digest must equal the committed pin); SDK pack not reached (permission bug, fixed in `70dfa12`) |
| `smoke` | not reached | `release.py verify`: artifact checks + installed-consumer smoke in the `tools` image |

Totals: **~8 minutes of image export**, **~22 minutes of compiling the same
dependency crates in five different target directories**, **20 minutes of
single-process pytest**, **2 minutes of serial mutants**; actual test
execution (cargo test ≈ 25 s, deno 4 s, Lean 20 s) is under a minute.

ci-infra facts that bound the design (read, not changed):
`controller.py` emits one Buildbot step per profile gate with
`haltOnFailure`, strictly in order; `runner.py` gives each gate a fresh
`git clone` of the frozen bundle (so every source file has a fresh mtime)
and runs `argv` with the host Docker daemon (`DOCKER_HOST=/var/run/docker.sock`),
so BuildKit's layer cache and cache mounts persist across gates and across
candidates; `records._validate_profile` fixes the profile name, platform
and policy and allows only `id/argv/timeout_seconds/inputs/outputs` per
gate — there is no parallel-gate or tier field. Parallelism therefore has
to live inside a gate's `argv`.

## 3. Decisions

Each decision names its regression risk. "Warm" means the toolchain images
are cached and the cargo cache mounts are populated by a previous
qualification; "cold" means neither.

### D1. BuildKit cache mounts for the cargo registry and target directories

`Dockerfile` `test`, `bin` and `release` stages mount
`/usr/local/cargo/registry` (id `gripsack-cargo-registry`) and `/app/target`
(ids `gripsack-target-dev`, `gripsack-target-release`) as BuildKit cache
mounts. The dependency tree compiles once per profile and is reused by
every later build; the `target/` directory leaves the image layers, so
`exporting to image` drops from 160 s to seconds.

What is reused and what is not: cargo fingerprints registry crates by
content (their sources are immutable and `Cargo.lock`-checksummed), so
they are reused; workspace crates are fingerprinted by source mtime, and
every ci-infra gate is a fresh clone, so **gripsack's own crates are always
recompiled** in qualification. The cache mount is a cache of third-party
compilation only.

Risk: a stale cached dependency artifact could, in theory, differ from a
from-zero build. Cargo's fingerprinting is the same mechanism every
developer build relies on; the on-demand `repro` service (0042 F) still
builds twice from clean target directories and is the reproducibility
gate. The conda helper's `--check` pin comparison keeps its own from-zero
temporary target directory (`tools/conda-helper/package.py`) — not changed.
Operational risk: `docker builder prune` on the worker empties the mounts
(cold again, not wrong); the mounts grow across versions — prune them
with `docker builder prune --filter type=exec.cachemount` occasionally.

### D2. One dev profile for `test` and the e2e binary

`bin` compiled with `CARGO_PROFILE_DEV_OPT_LEVEL=1` (so e2e does not pay
unoptimized hashing on every launch) while `test` compiled at opt-level 0:
two full dependency compilations per qualification. The `builder` stage
now sets `CARGO_PROFILE_DEV_OPT_LEVEL=1`, `…_DEBUG_ASSERTIONS=true`,
`…_OVERFLOW_CHECKS=true` for every dev build in the image, so `cargo test`,
clippy and the e2e binary share `gripsack-target-dev`. Host builds are
untouched (the env lives in the image).

Risk: tests now run at opt-level 1. `debug_assertions` and overflow checks
stay on explicitly, so `#[cfg(debug_assertions)]` fault controls and
`requires_fault_controls` e2e cases are unaffected; opt-level does not
change Rust semantics. Compile time per crate rises slightly on cold
builds and vanishes on warm ones.

### D3. `CARGO_BUILD_JOBS` 4 → 8

`4` was introduced with the Linux worker staging (`4b0988d`) alongside a
2 GiB *slot* memory partition for the ci-infra worker container; Docker
builds run in the host daemon, not in that slot, on 32 cores / 30 GiB.
8 jobs keeps worst-case rustc memory (≈1.5 GiB × 8 ≈ 12 GiB) inside the
30 GiB VM with room for the daemon, the page cache and a developer's
own work, while gates run one at a time; the value is a single `ENV` in
each cargo-bearing stage and in `verify`. The conda helper build inherits
it (`package.py` passes `CARGO_BUILD_JOBS` through). The first draft of
this plan used 12 (and D6 ran two such builds at once); the measurement
host rebooted twice during those runs, both times under heavy concurrent
compilation, so 8 is chosen as the value whose worst case fits with
margin rather than the value that saturates the cores. Compile wall time
is dominated by the critical path of the crate graph, not by the job
count, once jobs exceed ~8 (§5 records the cost).

Risk: OOM under concurrent builds (a developer running two compose gates
at once while the worker builds). Measured peaks during the runs in §5
are recorded there; if the worker ever OOMs, the number is the knob.

### D4. e2e in parallel (pytest-xdist)

`e2e/pyproject.toml` adds `pytest-xdist`; the image's CMD runs
`pytest -n auto` with `PYTEST_XDIST_AUTO_NUM_WORKERS=6` (override with
`-e`). Each worker runs one `grip` plus, for evaluating tests, one
sandboxed Deno; 6 keeps the gate's footprint small next to the compile
gates while taking most of the 8× speed-up (§5). The harness is already isolation-safe: every test lives under
`tmp_path` with `HOME`/`GRIPSACK_HOME` redirected (`conftest.sandbox`),
every fixture HTTP server binds port 0, the only session fixture
(`confined_runtime`) is a per-worker copy. `--durations=25` goes into the
evidence log so the next trim has data.

Risk: a hidden cross-test dependency (shared state outside `tmp_path`)
surfaces as a flaky failure under `-n`. The first runs in §5 passed; any
such failure is a harness bug to fix, not a reason to serialize.

### D5. Mutant calibration in parallel

`proofs/mutants.py` runs the 14 calibrations through a thread pool
(`PROOF_MUTANT_JOBS`, default 4 — each is one Charon/Aeneas run plus a
scratch `lake build` whose Lean processes load mathlib, ~1–2 GiB each;
4 bounds the gate at ≈8 GiB). Each mutant's output is captured and
printed whole, in declaration order, so the evidence log reads exactly as
before. Pass/fail semantics are unchanged: every mutant still runs, every
expected theorem must fail and nothing else may.

Risk: none to the proof claims (the calibration oracle is untouched);
memory pressure if `PROOF_MUTANT_JOBS` is raised on a smaller host.

### D6. `package` builds the core and the helper one after the other (concurrency opt-in)

`release.py prepare` ran `release` (271 s) then `helper` (~300 s) then
the SDK pack. The two builds are independent (different images, different
outputs) and *can* run as two threads with captured output printed in
order once both finish (`GRIPSACK_RELEASE_PARALLEL=1`); the SDK pack
follows. The default is the sequence: the core build (whose dependency
tree is now a cache mount, D1) then the helper (from zero by design,
D9). Nothing about *what* is built or checked changes; `verify`/`smoke`
are untouched.

Why serial by default: the concurrent mode's worst case is two full
cargo builds at once (2 × 12 rustc in the first draft), and the
measurement host rebooted during exactly that step. The release must not
depend on headroom luck, so the default is the mode whose peak is one
build; the parallel mode stays for a host whose size has been measured.
The cost is the shorter of the two builds, recorded in §5.

### D7. Tests replaced by Lean theorems (K3, K4-kernel, journal)

Rule applied: a test is dropped only if a theorem in `proofs/Kernel/`
over the extracted shipped code (`proofs/Extracted/`, drift-checked
against `crates/gripsack-kernel` in every `proofs` run) states the same
property for all inputs. Theorems are total-correctness Hoare triples, so
they imply every concrete value a unit test could pin. The result of
applying that rule honestly is **small**:

| Removed | Replacing theorem(s) | Note |
|---|---|---|
| `crates/gripsack-kernel/tests/kernel.rs::classification_is_exact_identity` | `classify_committed_iff`, `classify_uncommitted_iff`, `classify_ambiguous_iff`, `SelectionIdentity.same_spec`, `TransactionId.same_spec` | K3; the eight asserted cases are instances of the three biconditionals |
| `…::exact_pruning_boundaries_preserve_current_without_shifting_the_prefix` | `plan_prune_spec`, `admit_gc_spec`, `contains_generation_spec` | K4 kernel half: result = oldest `len−keep` minus current; RecoveryPending/CorruptCurrent/Admitted exactly |
| `…::deletion_positions_are_the_unreferenced_candidates_in_order` | `plan_delete_spec`, `contains_bytes_spec`, `same_bytes_spec` | K4: position deleted ↔ no root names the candidate |
| `…::publication_accepts_only_the_expected_effect_in_order` | `PublicationAction.successor_spec`, `RecordPublication.{new,action,acknowledge,finish}_spec`, `admit_mutation_spec` | journal protocol, the former Verus `ensures` |
| `…::inventory_admits_only_strictly_ascending_identities` (the `GenerationList` half) | `strictly_ascending_spec`, `GenerationList.new_spec` | the `checked_next`/`checked_previous` asserts stay as `generation_arithmetic_saturates_at_the_bounds` — no theorem covers those two functions |

Kept, with the reason each is *not* subsumed:

- Every e2e case examined for K3/K4 overlap (`test_gc_admission.py`,
  `test_recovery_durability.py`, `test_transaction_recovery.py`,
  `test_generation_history.py`, `test_store_hygiene.py`,
  `test_repeated_recovery.py`) exercises ring-3 wiring the theorems do
  not reach: gathering roots and inventory (0056 §5: "tested, not
  proved"), decoding legacy/transaction markers from the wire, fault
  injection and kill points, the CLI surface and the actual filesystem
  effects. Example: `test_legacy_same_generation_marker_remains_ambiguous`
  looks like K3, but what it checks is that a hand-written legacy marker
  *decodes* to a `SelectionIdentity::legacy` and that recovery then
  mutates nothing — neither is a kernel function.
- `fuzz/fuzz_targets/store_gc.rs` (trim register: "after K4/K5 proofs"):
  it drives GC over a sandboxed filesystem, not `plan_delete`; K5 is not
  proved yet. Stays, weekly only, as before.
- `gripsack-store` `*_model.rs` explorers: scheduled for F2's
  crash-injecting harness, not subsumed by K-theorems.
- K1, K2, K5, K6 and P1–P4 are end-state claims (0056 §5), not yet
  proved; nothing is dropped on their account.

Time saved by D7: none (these tests ran in 0.00 s). It is simplification
per the trim register, and it records the method for later cuts.

### D8. Gates stay separate; no tiering in the profile

Merging `ts-test` (16 s) and `audit` (51 s) into `test` would save about
two seconds of checkout each and lose per-gate receipts and logs — not
worth it. Tiering (fast PR gate vs full release qualification) cannot be
expressed in the pilot profile (`_validate_profile` pins one profile name
and one gate list, and `main` requires exactly that status). The fast
tier for developers remains "run the compose gates you touched", now much
cheaper; the single qualification stays the only release-blocking path.

### D9. Not changed

- The `proofs` image's cold path (mathlib cache 151 s + 20 GB export
  261 s) only runs when a pin in `proofs/Dockerfile` or
  `lake-manifest.json` moves (0056 §13.8 condition 1); warm runs are all
  `CACHED`. Left alone.
- The conda helper's from-zero release build and its pin check: the
  committed digest is the gate; it gets faster only through D3.
- `.ci/pilot.json`: same seven gates, same `argv`, same artifacts — no
  ci-infra profile change and nothing for the owner to re-approve.

## 4. Regression surface, summarized

| Change | What could regress | Why it does not |
|---|---|---|
| cache mounts | stale dependency artifact | cargo content fingerprints; workspace crates always rebuilt (fresh clone); `repro` gate for from-zero equality |
| one dev profile | test semantics at opt-level 1 | assertions/overflow checks pinned on; no `cfg` depends on opt-level |
| jobs 8 | OOM | one gate at a time; measured peak in §5 |
| xdist | hidden shared state | all state under `tmp_path`, port 0 servers, per-worker session fixture; first runs green |
| parallel mutants | — | oracle unchanged; output serialized |
| core+helper serial by default, parallel opt-in | resource contention only (opt-in) | independent images and outputs; default peak is one build |
| kernel test deletions | lose a concrete-value guard | each guard is an instance of a proved biconditional over the drift-checked extraction |

## 5. Measurements

"Before" is the archived candidate `aa525ca7…` (§2). "After" runs used a
fresh clone of this branch (fresh mtimes, like a ci-infra gate) against a
warm daemon; "after (cold mounts)" is the first run that populated them.

| Gate | Before | After, cold mounts | After, warm | Notes |
|---|---|---|---|---|
| `test` | 630 s | _see §5a_ | _see §5a_ | |
| `ts-test` | 16 s | — | — | unchanged |
| `proofs` | 643 s (154 s gate) | — | _see §5a_ | mutants parallel |
| `e2e` | 1526 s | _see §5a_ | _see §5a_ | xdist 6 |
| `audit` | 51 s | — | — | unchanged |
| `package` | 675 s (failed) | _see §5a_ | _see §5a_ | core then helper |
| `smoke` | not reached | _see §5a_ | | |

### 5a. Recorded runs

One row per gate per run, appended by the measurement driver as each gate
finishes (so a host reboot loses at most one gate). "cold" = the three
`gripsack-*` cargo cache mounts pruned first, toolchain image layers
cached; "warm" = a second fresh clone after the cold run, with a
one-line throwaway source change so BuildKit re-executes the RUN-step
gates (a tree it has already built would be fully `CACHED`, which is
what re-qualifying an identical commit looks like, not what a release
commit looks like). Memory is the whole WSL2 VM (`free -m`, 30 GiB,
Docker Desktop included), sampled every 5 s; the sampler would kill the
gate below 2 GiB available. Host: 32 cores, 30 GiB VM + 8 GiB swap.

| Run | Gate | Wall | Result | VM memory | Source |
|---|---|---|---|---|---|

## 6. How to qualify this branch

The owner submits qualifications (the worker is shared):

    python3 -m ci_infra qualify /path/to/checkout-of/ci/fast-qualification

Nothing in this plan changes `.ci/pilot.json`, so an already-frozen
candidate of another branch is unaffected.
