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
contract; the image does. Images build with BuildKit.

```
docker compose run --build --rm test      # fmt + clippy -D warnings + cargo test
docker compose run --build --rm ts-test   # typescript frontend tests (deno)
docker compose run --build --rm e2e       # offline flow tests (real binary + real frontend)
docker compose run --build --rm audit     # cargo audit, core + helper locks
docker compose run --build --rm proofs    # Lean proofs of the kernel (plan/0056 F1)
python3 scripts/release.py prepare        # release assets → ./dist/
python3 scripts/release.py verify         # artifact checks + installed-consumer smoke
docker compose run --build --rm diagram   # render docs/diagrams/*.d2
```

The required gates are `.ci/pilot.json` (test, ts-test, e2e, audit,
package, smoke), run by the local Buildbot (see Workflow). Optional and only
when asked: `model` (TLC), `tlaps`, `fuzz`,
`scripts/check_reproducible.sh`. `tlaps` is being retired by plan/0056 as
the Lean proofs replace it; don't extend it. Never dispatch fuzz without an
owner request.

`--build` after every source change or the image runs stale code. Host
`cargo test` / `pytest e2e` are fine for fast iteration; always finish
with the compose gates. `./dist/` and `target/` may contain root-owned
files (container mounts) — delete via docker or `sudo`.

Scope is Linux/WSL x86_64 only (`PLATFORM-LINUX-X64-2026-10-09`).
Other architectures, macOS and native Windows are out of scope until an
explicit owner decision.

TLC writes run artifacts next to the spec (`specs/states/`, `*_TTrace_*`,
root-owned). Commit specs and cfgs, never run debris. Keep a
counterexample as a cfg that must FAIL, not as a committed trace.

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

Follow [the storage flow](docs/operations/storage.md).
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
| `plan/` | numbered decision docs — read before changing behavior; `plan/STATUS.md` is the one-row-per-plan ledger — update it in the same PR |
| `plan/0056-verified-foundation.md` | **the active epic**: rings, Lean-proved kernel, target crate layout, trim register, phase ledger |
| `docs/diagrams/` | d2 sources + rendered SVGs (end-state architecture) |
| `docs/operations/` | workstation operator runbooks (storage) |
| `schema/ir/v10.json` | Current native workspace, service and portable-package authoring contract. Strict historical readers exist only for retained data; current execution admits IR10 |
| `crates/gripsack-ir` | IR types + validation (mirrors the schema) |
| `crates/gripsack-kernel` | ring 0: pure decisions and protocol machines, no dependencies, proved in Lean under `proofs/` |
| `crates/gripsack-host` | ring 2: storage trait + MemFs, protocol drivers, cap-std `fs`, process supervisor (`process`), store layout and wire codecs (`store`: paths, hashes, journal, manifests, selections, generations, activation storage) |
| `crates/gripsack-exec` | Native package/task/profile lifecycle, services, scheduled jobs, semantic review and portable sharing |
| `crates/gripsack-fetch` | built-in fetchers + `gripfetch-*` plugin host + tool provisioning (deno) |
| `crates/gripsack-conda` | Coherent Conda/wheel lock, archive, protocol and installed-tree admission |
| `tools/conda-helper` | Separately locked, pinned Rattler/Pixi/UV materializer and release packaging |
| `crates/gripsack` | the `grip` CLI |
| `typescript/` | The typed workspace SDK (`@gripsack/core`); also embedded in the binary |
| `e2e/` | pytest flow tests against the real binary + golden IR corpus |
| `specs/` | TLA+ models: TLC cfgs are bug-finders; TLAPS proofs retire under plan/0056 F3 |
| `.agents/skills/` | maintainer skills: IR evolution, e2e, release, PR |
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

- `plan/0056-verified-foundation.md` — the active direction. It supersedes
  0047's tooling decisions and 0048's verification/evidence/release contract.

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

## Foundation rules (plan/0056)

- **Rings.** Ring 0 kernel (pure decisions) and ring 1 protocol machines
  (sans-IO) are proved in Lean over the Aeneas translation of the shipped
  Rust. Ring 2 host code stays small with registered assumptions; ring 3
  admits untrusted input. Move code inward to gain guarantees.
  Dependencies flow inward only: `gripsack → exec/fetch → host → kernel`,
  `ir → kernel`; `scripts/check_architecture.py` (in the `test` gate)
  enforces the edge map and rejects the retired `-store`/`-policy`/`-fs`/
  `-process` crates.
- **Kernel code** (`gripsack-kernel`, once it exists): no IO, no floats, no
  `dyn`, no dependencies. Change it together with its Lean proofs; the
  `proofs` gate checks the committed translation has not drifted.
- **One proof language.** Verus is gone (plan/0056 F2 step 10a). Don't add
  TLAPS proofs or hand-mirrored Rust "model" explorers. Never close a theorem with `sorry`
  or an axiom that assumes the claim; the axiom audit rejects it.
- **The survival rule.** An artifact stays only if it is a machine-checked
  proof, a test guarding user-visible behaviour, or a counterexample finder
  that caught a real bug. No committed logs, receipts, evidence indexes,
  waiver IDs or checkers of checkers.
- **The release rule.** A release is the required gates green on the exact
  source. Nothing hand-assembled.

## Workflow

- `main` is protected and requires the `local-practical-qualification`
  status. Qualify the exact source on the owner's local Buildbot
  (`cd ~/workspace/ci-infra && python3 -m ci_infra qualify <clean checkout>`,
  see `~/workspace/SHARED-BUILDBOT-CI-PLAN.md` §15), then post the status with
  `python3 scripts/release.py status <candidate_id>`. `ci-infra` and the
  shared plan serve several projects — don't edit them from here.
- Hosted workflows (`audit`, `upstream-watch`,
  `template-mirror`, `fuzz`) are diagnostics or maintenance and never gate
  anything.
- Releases: `python3 scripts/release.py publish <candidate_id> --notes N.md
  [--yes]` uploads the qualified bytes (never rebuilt) as immutable
  `core-vX.Y.Z` matching the `gripsack` crate version. Unsigned,
  GitHub-asset-only; no npm/crates.io. See `.agents/skills/gripsack-release`.
- The website lives in `gripsack-dev/gripsack-dev.github.io` — edit there,
  not here. Build static pages locally from a selected core commit and
  publish with the site's `website/publish.py`.
