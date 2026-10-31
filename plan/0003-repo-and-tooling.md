# 0003 — Repo layout, build system, CI, and releases

- Status: active; updated 2026-10-28
- Amends: 0001 §6 (components)
- Authority: owner-directed Linux/WSL x86_64 practical qualification;
  `REL-PRACTICAL-051-2026-10-28` supersedes historical all-gates release policy.

## 1. One repo, coordinated packages

The schema, TypeScript emitter and Rust consumer form one current IR contract.
Keep changes atomic across schema, emitter, runtime, tests and actual examples.
The CLI is `grip`; the SDK is `@gripsack/core`. Core, SDK and shipped Conda helper
versions remain coordinated while pre-1.0 (current integration target 0.51.0).
The selected distribution is manual unsigned GitHub assets, not a registry
migration. Preparing crate/npm-format packages checks exact consumer bytes; it
does not authorize publishing them to crates.io or npm.

## 2. Layout and responsibility

- `crates/`: typed IR, policy, execution, process boundaries, store and CLI.
- `typescript/`: workspace authoring, evaluation driver and SDK tests.
- `schema/`: current versioned authoring/evaluation contracts.
- `tools/conda-helper/`: pinned native Conda/Pixi helper and package measurements.
- `e2e/`: sandboxed real CLI/frontend flows and explicitly selected campaigns.
- `scripts/`: existing local qualification, receipt admission and packaging.
- `.ci/pilot.json`: committed required gate recipe and exact artifact inventory.
- `Dockerfile`, `docker-compose.yml`, `.github/workflows/`: executable gate entrypoints.
- `specs/`: optional model/proof programme, never automatically claimed by a release.
- `verification/`, `plan/`: evidence, policy and product handoff.

Homebrew, macOS and the BuildKit product backend/bridge are retired, not deferred
supported channels. Generic OCI transport and Docker service runtime are separate
capabilities; removing BuildKit does not imply removing those. Existing Docker
build caches are allowed but cached RUN instructions are not fresh test evidence.

## 3. Practical gates by default

The committed profile is the required subset, not a hard-coded historical gate
count. The default is Rust fmt/clippy/unit tests, TypeScript tests/examples,
normal e2e flows, dependency audits, exact core/helper/SDK packaging and installed
consumer smokes, plus real CLI scenarios for changed features. Inventory and
source/image/package receipts remain part of qualification.

```sh
docker compose run --build --rm test
docker compose run --build --rm ts-test
docker compose run --build --rm e2e
```

These three commands are focused developer gates, not a substitute for complete
release admission. The full profile additionally runs audits, package/static/
version checks and exact installed consumers. Use the existing controller and
source-bound receipts as documented in
[release-handoff](../verification/release-handoff.md). Rebuild after changing
source; do not run a stale image. Host-toolchain tests are useful for iteration,
but do not manufacture image-bound qualification.

Optional, only when explicitly requested:

```sh
docker compose --profile optional run --build --rm gnu-test
docker compose --profile optional run --build --rm model
docker compose --profile optional run --build --rm tlaps
docker compose --profile optional run --build --rm verify
docker compose --profile optional run --build --rm mutants
python3 scripts/delivery_checker_calibration.py
sh scripts/check_reproducible.sh /absolute/path/to/fresh-repro-output
python3 tools/conda-helper/package.py --target x86_64-unknown-linux-musl \
  --dist /absolute/path/to/fresh-helper-output --check --reproducible
```

Exhaustive persistence remains `python3 scripts/local_ci_persistence.py`, with
its source-bound image and candidate/checkpoint prerequisites in the runbook.
All 96 partitions and both drift states remain mandatory **when selected**.
Unselected checks are unrun, not passing; a selected failure/missing receipt
blocks qualification. Fuzz/replay retains its separate owner authorization.
Routine qualification does not launch managed services against production state.
Changed service behavior requires a disposable, isolated real Docker/PostgreSQL
scenario, explicit start authority and no automatic data-volume deletion.

## 4. Static Linux release and helper integrity

The only release target is `x86_64-unknown-linux-musl`, for Linux/WSL x86_64.
ARM, macOS and native Windows are not current support promises.

- Core artifact: `gripsack-<VERSION>-x86_64-unknown-linux-musl.tar.gz`.
- Conda artifact: `gripsack-conda-<VERSION>-x86_64-unknown-linux-musl`, with
  checksum, source-bound measurement manifest and exact dependency notices.
- SDK artifact: `gripsack-core-<VERSION>.tgz`.
- No BuildKit bridge, B0 qualification or OCI builder release asset is required.
- Package consumers check checksums, ELF/static properties, exact version and
  compiled helper pin before actual CLI execution. TLS remains Rustls-only.
- One helper build is routine; independent two-build equality is explicit opt-in.
  Source-bound, pinned existing helper measurements may be reused without
  recompilation via `local_ci.py helper --reuse-helper-dist PATH`. All inputs,
  pin/license/static/protocol checks remain; changed or old unbound measurements
  fail rather than silently falling back or borrowing old qualification.

## 5. Tests and retained-state safety

| Layer | Scope |
|---|---|
| Rust unit | IR admission, policy decisions, store/ownership, process and scheduler behavior |
| TypeScript/examples | actual emitted shape, provenance, API types and runnable recipes |
| Normal e2e | actual CLI/frontend in private HOME, real apply/reapply/rollback/GC, native and helper boundaries |
| Changed-feature scenarios | observable new CLI/service behavior, including failure and ownership refusal |
| Optional deep campaigns | proofs/models/mutants, exhaustive persistence, independent reproducibility |

Pure decision proofs do not prove filesystem or Docker behavior. Ordinary flows
do not claim exhaustive crash coverage. Runtime durability, ownership, retained
generations, receipts, journals, rollback and GC invariants are not weakened by
making expensive verification optional. Historical proof/campaign reports remain
bound to their original sources.

## 6. Workflows and publication

CI selects routine jobs by default; manual-dispatch booleans opt into deep lanes.
The aggregate requires every selected lane to succeed and every unselected lane
to be skipped. Selected persistence additionally requires its full receipt union.
There is no missing/failing-selected-gate exception.

`release-core.yml` is manual dispatch guidance only. Publication is the owner's
manual unsigned GitHub-only path, after actual local practical qualification.
It does not rebuild on hosted runners, sign fictional evidence, publish registries
or update Brew. The runbook gives the exact nomination and `gh release` commands.
Existing signed installer/update admission remains fail-closed and separate; do
not promise unsigned assets satisfy a signed installation contract.

## 7. Demo automation

Existing VHS tapes and automation remain the reproducible demo source. Capture
or rerender only when requested, following the demo skill; a changed workflow is
not evidence that a new recording was rendered or reviewed.

## 8. Versioning and persisted compatibility

The 0.51 cutover uses IR10, one required workspace catalog, and strict current
writer/reader admission. Obsolete authoring APIs/readers are removed rather than
kept behind shims. An incompatible authoring change increments the IR version.
Persisted generation/lock/receipt/journal contracts are distinct: retain their
meaning, and never reinterpret stored bytes merely because authoring changed.

## 9. Qualification status

The protected context is `local-practical-qualification`. Its producer privately
re-admits the selected profile's original controller receipts, then binds the
exact source and release nomination. Status readers reject a missing, pending,
failed, foreign or stale newest witness. An owner merge exemption is not test
success. No workflow or documentation edit alone claims live protection changed.
