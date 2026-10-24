# AGENTS.md — working agreements for gripsack

Your whole environment in one bag: packages from any source plus dotfiles,
described once in typed TypeScript, deployed by a Rust core into a
hash-addressed store with generations and rollback. The frontend
evaluates modules to IR (JSON) inside a sandboxed, provisioned Deno —
facts are core-injected, effects are explicit probes — and the core
only ever consumes IR (`plan/0001`, `plan/0013`). Sourcing splits into
core-side resolution and exec-time transports, with `gripfetch-*`
plugins for bespoke transports (`plan/0002`).

## Build & test — docker first

Run the gates in containers, not on the host. The host has no toolchain
contract; the image does.

```
docker compose run --build --rm test      # fmt + clippy -D warnings + cargo test
docker compose run --build --rm gnu-test  # real GNU loader + auxiliary-input boundaries
docker compose run --build --rm ts-test   # typescript frontend tests (deno)
docker compose run --build --rm e2e       # flow tests (real binary + real frontend)
docker compose run --build --rm model     # TLC model check of the transaction spec (plan/0028)
docker compose run --build --rm tlaps     # transaction induction + barrier-order calibration (plan/0048 M-V1; amd64)
docker compose run --build --rm verify    # Verus proofs of policy kernels + mutant calibration (plan/0046; amd64 only)
docker compose run --build --rm -e VERSION=x.y.z release   # musl tarball → ./dist/
```

`--build` after every source change or the image runs stale code. Host
`cargo test` / `pytest e2e` are fine for fast iteration; always finish
with the compose gates. `./dist/` and `target/` may contain root-owned
files (container mounts) — delete via docker or `sudo`.

The active target scope is Linux/WSL x86_64 only
(`PLATFORM-LINUX-X64-2026-10-09`, plan0048). Other architectures, macOS and
native Windows are outside scope. Expansion requires a later explicit owner
decision and native evidence; Linux/common proofs and persistence are not waived.

TLC (the `model` gate) writes run artifacts next to the spec —
`specs/states/` fingerprint caches and `*_TTrace_*` counterexample
traces, root-owned from the container. Commit the spec and its cfgs
(`specs/Transaction.tla`, `specs/cfg/`), NEVER run debris (gitignored;
the gate runs TLC with `-cleanup`). Keep a counterexample as a cfg
that must FAIL, not as a committed trace — reproducible beats
archived.

## Where things are

| Path | Contents |
|---|---|
| `plan/` | numbered decision docs — read before changing behavior; `plan/STATUS.md` is the landed/deferred/rejected ledger — update it in the same PR |
| `schema/ir/v8.json` | Current emitted workspace or legacy-modules contract. Strict v7/v6 retain their executing semantics, v5 native file profiles remain executable, v4 workspaces stay read-only, and v3 module maps remain readable; old versions never acquire v8 declaration authority |
| `crates/gripsack-ir` | IR types + validation (mirrors the schema) |
| `crates/gripsack-store` | store paths, generations, GC |
| `crates/gripsack-exec` | Native workflows, consumers and whole-subgraph production coordination |
| `crates/gripsack-fetch` | built-in fetchers + `gripfetch-*` plugin host + tool provisioning (deno) |
| `crates/gripsack-buildkit` | Checked Linux production graphs, optional bridge protocol and owned workers |
| `crates/gripsack-conda` | Coherent Conda lock/protocol admission and helper client |
| `tools/conda-helper` | Separately locked, pinned Rattler/Pixi installer and release packaging |
| `crates/gripsack` | the `grip` CLI |
| `typescript/` | the typed module DSL (npm `@gripsack/core`); also embedded in the binary |
| `e2e/` | pytest flow tests against the real binary + golden IR corpus |
| `demos/` | VHS tapes |
| `specs/` | the TLA+ transaction spec + TLC cfgs (plan/0028) |
| `.agents/skills/` | maintainer skills: IR evolution, e2e, demo capture, release, PR |
| `skills/` | public skills: `gripsack-debug` (reading run logs), `gripsack-adopt` |
| `crates/gripsack-trace` | run tracing: JSONL logs with causal spans |

## Contracts to read before changing behavior

- `plan/0001-architecture.md` — modules, store, generations, ownership
  modes, activation, invariants (§9 holds or nothing does).
- `plan/0002-sourcing.md` — resolver/transport split, fetcher protocol,
  hash verification rules.
- `plan/0013-constrained-evaluation.md` — the eval boundary: one
  frontend, sandboxed Deno, injected facts, probes, trust gate.
- `plan/0003-repo-and-tooling.md` — this file's details: gates, releases,
  versioning, IR compatibility policy.
- `.agents/skills/gripsack-ir/` — how to evolve the IR safely.

## Hard rules

- **rustls only, never openssl** — the musl static binary is the north
  star; openssl breaks it.
- **IR changes touch all three sides in one PR**: `schema/`,
  `crates/gripsack-ir`, `typescript/`. Bump `ir_version` on breaking
  change.
- **Provenance is mandatory** — every semantic IR declaration carries
  its emitting source location (`span: {file, line, col?}` on the
  current wire); nested field values inherit their declaring node's
  location. Legacy v3 optional spans remain readable, never a reason
  to omit v4/v5/v6/v7/v8 provenance.
- The core never evaluates code and never sees credentials. The
  frontend runs sandboxed (no env vars, no network, no subprocesses);
  host facts arrive via the inputs envelope, effects as probes. The
  lockfile is the sole source of pinning.
- Never auto-rollback on post-activation hook failure (0001 §3.8).

## Plan 0048 implementation contract

For work under `plan/0048-review-response-0.42.0.md`, read §6
(release-blocking verification), §9 (mandatory scope and authorized
deferrals) and §10 (code-quality acceptance) before changing code.
Maintain its leaf-level execution/evidence record. An agent cannot
downgrade NEXT work, substitute tests for a required proof, omit a
caller/schema migration, or defer necessary module/type cleanup.
Missing required evidence blocks release; only the owner's explicit
decision changes that gate. Use meaningful domain/unit types and
cohesive modules/crates, not terse APIs or naked policy numbers to
minimize the diff. Existing workflow/protection notes below describe
the baseline, not permission to bypass plan 0048's stronger gates.

## Workflow

- Exact-source local qualification is integration/release acceptance authority
  (`LOCAL-QUALIFICATION-AUTHORITY-2026-10-23`, plan0054). The project-owned
  `.ci/pilot.json` and local gate commands retain the full Linux/native/formal
  requirements and all 96 persistence partitions; retiring hosted triggers
  does not waive them. Fuzz/corpus replay remains excluded for this release.
- `ci`, `examples`, `repro` and `demo` are explicit manual diagnostics or
  maintenance only, not automatic acceptance. Do not dispatch them for release
  qualification. Pin-update proposals never dispatch hosted builds.
- Before landing this cutover, the owner must reconcile authenticated branch
  protection's legacy required `test` context with real local admission, not
  fabricate a successful hosted status or remove qualification enforcement.
- Releases use immutable `core-vX.Y.Z` and the coordinated locally prepared
  core/SDK/helper bundle. GitHub approves, signs and publishes those bytes via
  `release-core.yml`; there is no independent `ts-v*` publication for this flow.
- Configure the protected `release` environment before dispatch: trusted reviewer,
  selected `core-v*` tag policy and deliberate administrator-bypass policy.
  A sole owner who dispatches and approves must not enable `prevent_self_review`;
  enabling it requires a genuinely separate authorized reviewer.
- Site promotion carries the actual signing `GITHUB_RUN_ID` as integer `run_id`,
  alongside release tag and manifest digest. These are selectors; the site must
  authenticate the published signature/workflow/source, not trust the payload.
- The website lives in `gripsack-dev/gripsack-dev.github.io` — edit
  there, not here.
