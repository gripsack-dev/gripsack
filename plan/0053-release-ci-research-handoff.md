# 0053 — CI and release-flow research handoff

Status: **research requested; no migration or replacement workflow approved.**
Evidence snapshot: 2026-10-04. The release owner is continuing 0.44.1 separately.
This document is input to another agent, not an additional 0.44.1 release gate.

**Subsequent owner decision:** the stalled 0.44.1 delivery may proceed without a
fresh hosted full-CI campaign, under `REL-ALPHA-0441-2026-10-04` recorded in
[PR175's plan0048](https://github.com/gripsack-dev/gripsack/blob/fix/bridge-relative-dist/plan/0048-review-response-0.42.0.md#owner-exception-rel-alpha-0441-2026-10-04).
Artifact verification and truthful platform/provenance claims remain required.
This scoped release exception does not approve a general CI migration or turn
unrun checks into passing evidence.

## 1. Assignment and coordination

Research a predictable, resumable CI/release flow that catches publication defects
before irreversible uploads, preserves the existing assurance, and avoids wasting
qualified work. Recommend a design with evidence and a staged adoption plan.
Do not implement a hosting migration or change the active release's gates.

The owner explicitly asked for this research while the current agent lands 0.44.1.
Keep research findings/proposals separate from this dated requirements snapshot.
Do not edit the release branch, move tags, publish packages, cancel its runs, or
change branch protection/secrets without coordinating with the release owner.

Current release pointers, **as of this snapshot**:

- Core/workspace integration [PR173](https://github.com/gripsack-dev/gripsack/pull/173)
  and external example [PR1](https://github.com/gripsack-dev/example-env-typescript/pull/1)
  are merged. Exact source `f8c68f6a80e08ad511896274c19e3d14d6919b03` passed full CI.
- Both `core-v0.44.0` and `ts-v0.44.0` point to that source. SDK 0.44.0 is published;
  core 0.44.0 publication failed before core compilation/upload.
- The owner chose **a coordinated core/SDK 0.44.1 release**, not moving either
  existing tag. [PR175](https://github.com/gripsack-dev/gripsack/pull/175) owns it.
- Patch measurement source was `d7e4d285491de2d3551c66227f808804480220bb`.
  Native helper [run37195005178](https://github.com/gripsack-dev/gripsack/actions/runs/37195005178)
  was pending; do not treat it or the new patch as qualified from this snapshot.
- Website [PR20](https://github.com/gripsack-dev/gripsack-dev.github.io/pull/20)
  is staged for 0.44.1, not deployed. Consult PR175 and `plan/STATUS.md` for later facts.

## 2. Observed pain: distinguish causes

### P1 — A green source candidate did not cover the exact publication invocation

[CI37134187816](https://github.com/gripsack-dev/gripsack/actions/runs/37134187816)
passed on `f8c68f6`: 203 jobs, 202 successful and only the explicitly waived fuzz
job skipped, including all 192 persistence partitions. The real-caller
[canary37134189898](https://github.com/gripsack-dev/gripsack/actions/runs/37134189898)
also passed. Nevertheless, core publication
[37182423088/job111377500605](https://github.com/gripsack-dev/gripsack/actions/runs/37182423088/job/111377500605)
failed with `cannot open /out/targets.txt`.

Cause: `tools/buildkit-bridge/dist.sh --check --dist dist` passed `dist:/out` to
Docker. That is a named volume, not the populated host directory. Earlier local
measurements used absolute output paths. Resolving the path fixed the actual
invocation: all four bridge assets built twice and matched unchanged pins.
PR175 adds that real distribution command to required CI before tagging.

A previous 0.43 repair had the same class of gap: the native verifier was invoked
from `dist/` with the wrong script path; see
[PR172](https://github.com/gripsack-dev/gripsack/pull/172). Test the production
working directory, argument form and filesystem layout, not a convenient equivalent.

### P2 — Two publication workflows can leave a partial release

The [0.44.0 SDK publisher](https://github.com/gripsack-dev/gripsack/actions/runs/37182422705)
succeeded while the core publisher failed. npm's
[0.44.0 metadata](https://registry.npmjs.org/@gripsack/core/0.44.0) records `gitHead`
`f8c68f6`; there was no matching core GitHub release or core crate in the index at
failure investigation. Core build/release jobs were skipped; all three Conda
helper qualifications had succeeded.

Registries are not a distributed transaction. A future flow needs an explicit
partial-publication state, a safe ordering, and a precise resume/patch policy.
A published package must not be silently replaced, and a website/pin update must
not claim a complete release merely because one publisher succeeded.

### P3 — Tag identity, CI-skip markers and manual entrypoints require choreography

Commits deliberately marked `[skip ci]` avoid automatically replaying owner-excluded
fuzz/corpus work, but tag-triggered workflows can then be skipped too. Core
publication now supports dispatch against an existing `core-v*` tag; the SDK
workflow also accepts an existing tag. A release should not require moving a tag
just to invoke its publisher. The owner explicitly chose 0.44.1 after the 0.44.0
packaging failure; preserve that decision and both existing tags.

### P4 — Queueing, execution and repeated preparation are conflated

The final 0.44.0 full-CI observer took about 14.5 hours including queueing and
execution; that is **not** a measurement of CPU/test execution time. The caller
canary was still queued after roughly six hours. Its eventual job ran from
2026-10-03T23:01:54Z to 23:34:14Z (32m20s).

The provider-side cause of the queue was not established. Do not assume billing
limits, macOS alone, or product failure. Linux-inclusive queues were observed.
The current CI source also records an earlier serial job hitting the six-hour
limit before later formal gates could run; the lanes were subsequently split.

The persistence matrix is now 2 platforms × 6 scenarios × 2 fault modes × 8 shards
= 192 partitions. Native Mac shards each install tooling/build the binary/prefetch
Deno. Linux shards each enter the Compose build path. Measure actual setup,
compile, test, queue and artifact-transfer costs before proposing reuse or regrouping.

### P5 — Artifact work and evidence are repeated across surfaces

Core gates, the real-caller canary, helper qualification, reproducibility and
publication rebuild overlapping inputs. The bridge and Conda helper have their
own toolchains, locks and platform matrices. A core patch also updates helper
release identities under the current design.

Observed: the measured Go bridge bytes were identical for 0.44.0 and 0.44.1;
the release identity/asset names changed. New native Conda measurements were still
required and pending when this was written. Do not assume their bytes are identical.
Investigate component-scoped artifact identities and validated reuse, not copying
an old pass to a new SHA. The existing same-input reproducibility experiment is
not a general cross-time/cross-platform reproducibility theorem.

### P6 — Registry publication and dependency order are operationally fragile

The publisher has a hand-maintained fourteen-crate sequence, sparse-index waiting,
and per-crate retry handling. Earlier preparation exposed omitted/wrongly ordered
workspace crates; the current order includes Conda and production BuildKit after
its dependencies. A workspace packaging dry-run caught this before upload.

Research a checked publication DAG and resumable per-package receipts. Distinguish
index propagation/rate limits/transport failures from compile, authentication,
version and artifact-identity failures. `already exists` alone is not proof that
the existing registry bytes belong to the intended candidate.

### P7 — A candidate checkout is not the installed public package

The external example originally linked `file:../gripsack/typescript`. During its
registry cutover, changing the manifest to `0.44.0` and regenerating the lock did
not remove the linked lock entry; `npm ci && npx tsc` failed TS2307 in both
entrypoints. Removing the linked dependency, reinstalling the exact published
version and removing its extraneous sibling lock record fixed a clean install.

The earlier cold-HOME caller also exposed real product defects (incidental umask
in definition identity and cache-only frozen archive acquisition), recorded in
`plan/0052-workspace-contract.md` §33. Those were product fixes, not reasons to
relax the consumer checks. Preserve both embedded-frontend and installed-package
journeys, frozen locks, cold homes, default helper provisioning and retained reuse.

### P8 — Container/host ownership can masquerade as product failure

A local SDK build hit EACCES in root-owned `node_modules/.bin`; an isolated,
user-owned build copy fixed the harness. A website preview initially returned
404 for copied 0600 assets owned by the container's root user. Correcting only
the generated preview ownership restored it; no CSS/product patch was needed.
Define source/output ownership deliberately. Do not solve this with broad chmod
or chown over an operator's repository or HOME.

### P9 — Observing a run is itself fallible (tooling, not gripsack)

The canary's `gh run watch` stopped on an HTTP504 while the actual workflow was
still queued. Reattaching to the same run/source observed its eventual success;
the workflow did not need a rerun. Several assistant-harness background completions
also required explicit recovery instead of the promised notification. Those
harness delivery defects are outside gripsack's repository.

Operator state must survive lost notifications, API errors and session restarts:
run ID, attempt, exact source, current stage, artifacts and next action cannot
live only in chat memory. A watcher error must never become a fabricated job
failure or success. Minimum supported CLI/tool versions also belong in preflight;
this session's older host `gh` lacked newer JSON fields and `--slurp`.

## 3. Requirements for the proposed future flow

These describe research acceptance, not permission to change today's gates.

| ID | Required outcome |
|---|---|
| CI-R01 | One immutable candidate record identifies source, core/SDK/helper versions, locks, toolchain/image inputs, caller/site revisions and expected artifacts. Changed inputs invalidate the affected evidence explicitly; no informal "same enough" reuse. |
| CI-R02 | Non-publishing preflight exercises the actual publication commands/layouts and every intended platform artifact before irreversible registry writes. Relative paths, clean output directories, native dependency checks, package contents and installed entrypoints are covered by behavior, not YAML/source-text assertions. |
| CI-R03 | Qualify/promote the same artifacts where possible, or prove a rebuild's equivalence with retained input/digest evidence. Separate component identities and explain exactly when cached build, proof or test evidence is reusable. |
| CI-R04 | Resumption is explicit and idempotent at stage/package boundaries. Preserve successful same-input work, retain failure evidence, classify retryable errors, and avoid duplicate uploads or stalled superseded concurrency slots. |
| CI-R05 | Required aggregation validates the complete measured inventory and source identity. Failed, missing, duplicate, foreign-source, skipped, cancelled, timed-out and unsupported required work cannot become green. Any scheduling change preserves all current obligations. |
| CI-R06 | Publication follows a validated workspace dependency DAG; each registry version is tied to its expected artifact/source. Partial core/SDK publication is represented honestly. Existing tags/packages remain immutable; recovery does not depend on force-retagging or blanket "already exists" acceptance. |
| CI-R07 | A small repeatable operator entrypoint/procedure covers preflight, qualification, publication, resume and final verification, with actionable machine-readable status. Reuse the repository's existing producers/validators rather than creating another competing workflow framework. |
| CI-R08 | Runner/platform decisions preserve actual supported behavior and provenance policy. Separate queue delay from execution; quantify cold/warm setup and transfer costs. Native evidence cannot be replaced by cross-compilation or a different virtualization capability. |
| CI-R09 | Compact, durable, source-bound receipts link to retained raw evidence and exact tool versions/commands/artifact hashes. Keep generated logs, prover caches and traces out of source control; define retention/retrieval rather than relying on temporary chat artifacts. |
| CI-R10 | Post-publication verification uses actual registry packages, downloaded native/helper assets and the real installer in fresh private homes. Verify provenance, checksums, native execution, deliberate SDK pinning, default helper bootstrap, frozen consumers, and the deployed website before declaring the release complete. |

## 4. Non-negotiable constraints

- Follow `plan/0048-review-response-0.42.0.md` §§6, 9 and 10. Existing proof,
  runtime, native, attribution and evidence requirements are not optimization
  variables. Do not replace required theorems with tests or shrink the corpus
  of required persistence states to make a pipeline appear faster.
- The owner instructed **no fuzz-engine or corpus replay** in this work.
  `REL-FUZZ-2026-09-27` is an explicit exclusion, not passing evidence. Do not
  trigger those lanes while researching. The old 0.43 alpha-CI exception does
  not apply to 0.44.1.
- `MAC-NATIVE-QUAL-2026-10-02` excludes Mac VZ lifecycle and the full coherent
  Mac Conda runtime journey. Hosted VZ was unavailable; the native Conda fixture
  refused a `libgcc_s.1.1.dylib` format before publication. Ordinary confined Mac
  core flows, persistence and helper/native build qualifications remain required.
  No maintained self-hosted Apple Silicon facility was available to this effort.
- Preserve rustls-only/musl goals, pinned toolchains/images and the real confined
  frontend. Do not restore an unconfined evaluator escape hatch or infer user
  approval from CI. Consumer fixtures explicitly approve their captured digests.
- Preserve native artifact/SBOM/dependency checks and signed provenance binding
  to repository, source, ref and authorized workflow. Existing verification
  rejects self-hosted release-runner provenance; a hosting proposal must address
  that policy explicitly, not quietly disable verification or launder provenance.
- Keep the two tag namespaces, registry version guards and deliberate-pin rule.
  Website/example cutovers follow actual compatible publication, not a source
  version bump. Do not hard-code an unpublished SDK into a live consumer.
- Same-input two-clean-build reproducibility does not imply hermetic,
  cross-time, cross-platform or externally audited builds. Keep these limits.
- Research is not authorization to spend money, provision permanent runners,
  move providers, change protections/secrets, or interfere with PR175.

## 5. Research questions and expected deliverable

Start with an inventory of the **actual current DAG**, artifact producers,
consumers, input identities, platform constraints, publication side effects and
required evidence. Separate repository defects from provider capacity and operator
or assistant-harness failures. Mark unmeasured explanations as hypotheses.

Compare at least:

1. A simpler flow remaining on GitHub Actions, improving exact preflight,
   artifact reuse, matrix preparation and durable/resumable orchestration.
2. A hybrid with dedicated capacity for suitable lanes while retaining the
   required native platforms and authorized release provenance.
3. A provider/hosting change only if measured shortcomings justify it. Include
   operational ownership and capability gaps, not merely advertised features.

For each option, show trust boundaries, platform fidelity, cache/evidence
invalidation rules, partial-publication behavior, queue/runtime measurements,
artifact retention, costs with dated primary sources, operational burden and a
reversible adoption path. Do not invent measurements or promise an unavailable
Mac virtualization capability.

The recommendation should include:

- A concise failure/requirements-to-design mapping and before/after DAG.
- The smallest useful first change, followed by independently reviewable stages;
  identify explicit owner decisions and keep the live release independent.
- An input-change matrix: which source/lock/toolchain/platform/script changes
  require which rebuilds, tests, proofs and native qualifications, and why.
- A concrete candidate/publication receipt shape and resume state model using
  existing repository concepts where possible.
- Behavioral acceptance/calibration cases: stale or foreign artifacts; missing
  persistence partitions; wrong helper pins; verifier/source/ref mismatch;
  registry version containing unexpected bytes; API observation failure;
  interrupted/partial publication; and an example retaining a local SDK link.
- Honest unresolved questions. No new mandatory gate or broad migration is
  accepted merely because it appears in the research proposal.

## 6. Source map

Read the matching source revision, not just current filenames after they move:

- `.github/workflows/{ci,examples,release-core,release-typescript,repro,qualify-conda-helper}.yml`
- `Dockerfile`, `docker-compose.yml`, `scripts/check_ci_gate.py`
- `scripts/check_reproducible.sh`, `scripts/check_native_binary.py`
- `scripts/check_examples.py`, `scripts/examples_check/`, `scripts/gen_frontend_embed.py`
- `tools/buildkit-bridge/{build,dist}.sh`, `crates/gripsack-fetch/src/bridge_pins.rs`
- `tools/conda-helper/{dist.sh,package.py,Cargo.toml,Cargo.lock}` and
  `crates/gripsack-fetch/src/conda_pins.rs`
- `e2e/test_persistence_matrix.py`, its reported inventory and required aggregate
- `plan/0003-repo-and-tooling.md`, `plan/0048-review-response-0.42.0.md`,
  `plan/0052-workspace-contract.md`, `plan/STATUS.md`
- `gripsack-dev/example-env-typescript` and the separate
  `gripsack-dev/gripsack-dev.github.io` repository's published-example gate.

Useful evidence links: [full candidate CI](https://github.com/gripsack-dev/gripsack/actions/runs/37134187816),
[real caller](https://github.com/gripsack-dev/gripsack/actions/runs/37134189898),
[failed core publisher](https://github.com/gripsack-dev/gripsack/actions/runs/37182423088),
[published SDK](https://github.com/gripsack-dev/gripsack/actions/runs/37182422705),
[tag-bound two-clean-build comparison](https://github.com/gripsack-dev/gripsack/actions/runs/37182422842).
