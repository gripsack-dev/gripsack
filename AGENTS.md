# AGENTS.md — working agreements for gripsack

Your whole environment in one bag: packages from any source plus dotfiles,
described once in typed TypeScript, deployed by a Rust core into a
hash-addressed store with generations and rollback. The frontend
evaluates a workspace to IR (JSON) inside a sandboxed, provisioned Deno —
facts are core-injected, effects are explicit probes — and the core
only ever consumes IR (`plan/0001`, `plan/0013`). Sourcing splits into
core-side resolution and exec-time transports, with `gripfetch-*`
plugins for bespoke transports (`plan/0002`).

## Build & test — docker first

Run the gates in containers, not on the host. The host has no toolchain
contract; the image does.

```
docker compose run --build --rm test      # fmt + clippy -D warnings + cargo test
docker compose run --build --rm ts-test   # typescript frontend tests (deno)
docker compose run --build --rm e2e uv run --locked --no-sync pytest --ignore=test_persistence_matrix.py --ignore=test_golden.py
docker compose run --build --rm -e VERSION=x.y.z release   # musl tarball → ./dist/
```

The essential profile is `.ci/pilot.json`: Rust, frontend, ordinary offline flows,
dependency audits, exact packaging and installed consumers. Changed features also
need actual runtime smokes. `verification/release-handoff.md` lists the commands.
GNU duplicate builds, TLC/TLAPS/Verus, broad mutation/fuzz campaigns, exhaustive
persistence and two-build reproducibility are optional and run only on explicit
owner request (`REL-PRACTICAL-051-2026-10-28`). Unrun does not mean passed.

`--build` after every source change or the image runs stale code. Host
`cargo test` / `pytest e2e` are fine for fast iteration; always finish
with the compose gates. `./dist/` and `target/` may contain root-owned
files (container mounts) — delete via docker or `sudo`.

The active target scope is Linux/WSL x86_64 only
(`PLATFORM-LINUX-X64-2026-10-09`, plan0048). Other architectures, macOS and
native Windows are outside scope. Expansion requires a later explicit owner
decision and native evidence. Runtime safety remains required; optional assurance
must not be represented as current-source qualification.

TLC (the `model` gate) writes run artifacts next to the spec —
`specs/states/` fingerprint caches and `*_TTrace_*` counterexample
traces, root-owned from the container. Commit the spec and its cfgs
(`specs/Transaction.tla`, `specs/cfg/`), NEVER run debris (gitignored;
the gate runs TLC with `-cleanup`). Keep a counterexample as a cfg
that must FAIL, not as a committed trace — reproducible beats
archived.

## Storage hygiene — standing owner authorization

`STORAGE-HYGIENE-2026-10-30`: when capacity blocks work, reclaim inactive
rebuildable caches or relocate cold artifacts to the verified Seagate archive
without asking again. Delete non-rebuildable copies only after independently
verifying a complete retained copy. Preserve active work, source/uncommitted
changes, published releases, backups, checkpoints and unbacked failure evidence.
Record paths, hashes, retained locations and measured space recovery.

Docker execution does not determine where its files live. On the current
workstation, Docker-managed storage is on **B:**, Ubuntu's VHDX is on **C:**,
and the Seagate cold archive is **D:** (`/mnt/d/CI-Archive`). A bind mount of
`~/workspace/.../target` writes into Ubuntu on C:, even when Cargo runs in Docker.
Prefer a Docker-managed named target/cache volume for development iteration;
ordinary image-build layers already use Docker storage. Host editor checks can
also create Ubuntu caches. Verify volume identity and capacity, not drive letters
alone, before relocation. Never move live controller state onto an unchecked
filesystem or change an admission floor without an explicit owner decision.

`STORAGE-FAST-96-2026-10-31` is that explicit temporary exception: the owner
selected a 96-GiB physical B: reserve for the current drive until the 4-TB
replacement. The system-volume 128-GiB and state-volume 64-GiB floors remain;
no qualification gate is waived. Restore B:'s 128-GiB reserve when commissioning
the replacement drive and recording its new storage identity.

Follow [the storage flow](verification/release-handoff.md#storage-hygiene-and-archive-relocation).
No blanket Docker/volume pruning, deletion of unmatched evidence, or unannounced
WSL shutdown. Preserve in-use editor libraries rather than killing the editor.
Guest free space and Windows VHDX space are different measurements.
Start and stop Docker Desktop as the normal owning account. Elevation is only
for the specific disk/ACL maintenance step; do not launch Docker from that
elevated shell afterward. A successful compaction is not proof of usable WSL
permissions or preserved engine identity: verify both and run a retained image.


## Where things are

| Path | Contents |
|---|---|
| `plan/` | numbered decision docs — read before changing behavior; `plan/STATUS.md` is the landed/deferred/rejected ledger — update it in the same PR |
| `schema/ir/v10.json` | Current native workspace, service and portable-package authoring contract. Strict historical readers exist only for retained data; current execution admits IR10 |
| `crates/gripsack-ir` | IR types + validation (mirrors the schema) |
| `crates/gripsack-store` | store paths, generations, GC |
| `crates/gripsack-exec` | Native package/task/profile lifecycle, services, scheduled jobs, semantic review and portable sharing |
| `crates/gripsack-fetch` | built-in fetchers + `gripfetch-*` plugin host + tool provisioning (deno) |
| `crates/gripsack-conda` | Coherent Conda/wheel lock, archive, protocol and installed-tree admission |
| `tools/conda-helper` | Separately locked, pinned Rattler/Pixi/UV materializer and release packaging |
| `crates/gripsack` | the `grip` CLI |
| `typescript/` | The typed workspace SDK (`@gripsack/core`); also embedded in the binary |
| `e2e/` | pytest flow tests against the real binary + golden IR corpus |
| `demos/` | VHS tapes |
| `specs/` | the TLA+ transaction spec + TLC cfgs (plan/0028) |
| `.agents/skills/` | maintainer skills: IR evolution, e2e, demo capture, release, PR |
| `skills/` | public skills: `gripsack-debug` (reading run logs), `gripsack-adopt` |
| `crates/gripsack-trace` | run tracing: JSONL logs with causal spans |

### Native sharing ownership

`gripsack-exec::workspace::sharing` owns the passive portable package boundary:
`format` admits strict canonical closure metadata, `trust` authenticates exact
DSSE bytes against local namespace/layout/validator policy, `registry` transfers
digest-bound OCI artifact envelopes, and `runtime`/`receipt` validate, publish
and re-admit through the existing store/lifecycle roots. `pack` is an explicit
publisher effect, not a relocator. `sharing::conda` passes original archives and
the core-pinned helper to the ordinary independently validated Conda materializer.
`gripsack-conda::wheels` owns bounded original-wheel admission, Python/tag/
dependency compatibility and expected installed inventories. The separately
pinned helper's UV installer owns effects; `workspace::conda::validate` merges
Conda/wheel ownership before execution and independently validates the complete
result and normalized receipts afterward. No wheel resolver or pip invocation
is introduced.
The CLI supplies private signing/authentication files only to those effect
adapters; no key or registry credential enters IR, semantic snapshots or receipts.
Consumers and retained jobs call common current-trust admission before launch.
Transport cannot activate profiles, execute publisher checks or register jobs.
Retention contains imported receipts, payloads, original archives and helper
objects independently of producer paths and registry availability.

### Service, job and review ownership

`gripsack-exec::services` owns Docker Engine Unix-HTTP effects, workspace-private
resource ownership and readiness. Only explicit `up` grants startup authority;
task prerequisites observe existing health. No Docker CLI parsing, remote socket
fallback, Gripsack daemon or implicit volume deletion. Inherited credentials are
resolved only by that launch adapter, never by evaluation or snapshot capture.

`workspace::schedules` owns captured revisions and checked systemd user units.
The existing profile journal, activation ledger and lifecycle session own
registration and admission; the common task runner owns each finite invocation.
Unknown manager/process outcomes retain their roots. No automatic hook rollback.

`semantic` owns bounded admitted snapshots and pure old/new comparisons;
configuration views explain exact proposed bytes without rewriting them.
`saved_plan` revalidates the reviewed scope under the existing lifecycle session
before the shared planner. Reports and publisher signatures never grant host-write
authority. BuildKit, Homebrew and their executable authoring paths are retired;
stored data needed for rollback and integrity retains its original meaning.


## Contracts to read before changing behavior

- `plan/0055-workspace-only-product.md` — current workspace-only API and
  clean alpha cutover; supersedes historical module and old-reader contracts.

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
  location. Provenance is required throughout the workspace contract.
- The core never evaluates authored code; IR and the evaluator never receive
  credentials. Narrow native signing/authentication adapters own explicitly
  selected secrets and exclude them from IR, receipts and logs. The
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

- Exact-source practical local qualification is the release acceptance authority
  (`LOCAL-PRACTICAL-QUALIFICATION-2026-10-28`, plans0048/0055).
  The selected essential profile replaces the historical 17-gate/96-partition
  requirement. Additional assurance is opt-in and explicitly unrun unless
  executed. No assumed HP-mini access, new signer or hosted duplicate build
  is a prerequisite; publication remains manual and unsigned.
- `ci`, `examples`, `repro` and `demo` are explicit manual diagnostics or
  maintenance only, not automatic acceptance. Do not dispatch them for release
  qualification. Pin-update proposals never dispatch hosted builds.
- Main requires the strict `local-practical-qualification` status, not the
  retired hosted `test` context. Status descriptions and evidence must identify
  their actual source and check scope; a status alone is not release authority.
- Releases use immutable `core-vX.Y.Z` and coordinated locally prepared
  core/SDK/helper assets. The owner chose unsigned, Actions-free publication
  after practical local qualification; no signer or registry publication is
  required. Existing verified installer/updater checks are not bypassed.
- The website has no signature requirement. Build static pages locally from a
  selected core commit, then use the site's `website/publish.py` for the
  publish-only Pages job. Core release/demo workflows do not dispatch site builds.
- The website lives in `gripsack-dev/gripsack-dev.github.io` — edit
  there, not here.
