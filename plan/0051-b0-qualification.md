# 0051 — B0 qualification: BuildKit integration inventory and lane status

Status: B0 in progress · Owner: implementation agent · Date: 2026-09-24
Scope: Epic B foundation · Source: bundle edition 5, `gripsack-builder-epic.md`

Backend decision (owner-settled by the bundle): stock BuildKit for whole
compatible Linux production subgraphs and OCI export; Gripsack-managed
Lima VM on supported macOS; no fork, no second frontend, no general
engine rewrite. B0 qualifies before production restructuring.

## B0-04 — concrete integration/guarantee inventory

### Supported LLB operation/field matrix (initial lowering set, B2 scope)

| Admitted operation (IR) | LLB lowering | Initially supported fields | Explicitly rejected |
|---|---|---|---|
| Captured source/file input | `llb.Local` via session with declared include set, or digest-pinned `llb.Image` for toolchains | declared paths, digest pins, follow-symlinks policy | ambient host paths, home mounts, undeclared globs |
| Archive extraction (verified bytes) | `FileOp` unpack of already-verified input via pinned extraction helper | supported formats + entry/path/link limits mirroring gripsack-fetch | re-downloading through the daemon, relaxed containment |
| Literal file content | `Mkfile` | mode, ownership, dedented body with source map | interpolation of unbound values |
| Directory assembly | `Mkdir`, `Copy` with explicit src/dst selectors | per-output selector, collision policy | implicit whole-tree copies |
| Command execution | `Run` with argv, env, cwd, network=none default, mounted inputs/outputs | typed tool refs, platform per-op, bounded stdout/stderr capture | network entitlements, privileged execution, host-socket access, shell-text dependency inference |
| Output export | solved ref → Gripsack staging via session export | declared outputs only | publication directly from worker cache |

Fields outside this matrix (and any Dockerfile generation path) are
rejected at admission, not silently defaulted. The independent Rust
checker (B2-02) validates the actual serialized LLB against exactly
this matrix.

### Bridge protocol states/fences (Rust↔Go, B1 scope)

States: `negotiate` (version/capability pair, pinned) → `submit`
(validated graph/image plan) → `running` (bounded status/log events,
bounded queues) → `export` (declared outputs into staging) → `done |
cancelled | failed`. Fences: submission binds exact checked LLB bytes +
exporter options (post-check substitution rejected); cancellation is
idempotent with unknown-outcome reporting (never silently retried);
EOF/unknown outcome leaves no partial visible installation; duplicate/
conflicting/stale events are rejected by frame epoch (`FenceEpoch`).
Framed structured transport, explicit maximum lengths, no whole-artifact
or unbounded log buffering; credentials only through the existing opaque
session boundary.

### Proof families and production mappings (B1/B2 rows)

| Family | Production mapping | Evidence |
|---|---|---|
| Worker/session lifetime | `gripsack-buildkit::worker` lease/transition decisions | TLC interleavings + TLAPS lease/fence safety + Verus transitions (B1-04) |
| Lowering/submission | `gripsack-buildkit::llb_validation` pure checker kernels | Verus + semantic mutants + independent pure-subset interpreter (B2-03) |
| Existing retained guarantees | `gripsack-policy` kernels (0046/0047) stay production-wired | unchanged gates keep passing |

### Independent emitted-LLB inspection witness (B0 deliverable)

The harness dumps the actual serialized LLB definition + exporter
config the Go client produces; the Rust side (initially a checker
function in the harness) decodes protobuf-independent fields (digests,
op indices, mounts, network policy, selectors) and compares against
the submitted plan. Witness = the dump file plus the diff report;
committed as a fixture once B2 lands. B0 demonstrates the mechanism on
the harness's own probe graphs.

### Scheduler guarantee migration plan (B5-01/B5-05)

Current: `PureScheduler` (plan/0047) is the proved transition kernel
wired into production `run_all`. Migration: whole admitted Linux
subgraphs lower to one BuildKit solve; Gripsack keeps outer ordering
(acquisition, materialization, submission, validation, publication,
activation). After B2 lands: audit the production call path, delete
only per-vertex scheduling made redundant by delegation, update
SCHEDULER-001/GRAPH-CLOSURE-001 guarantee wording to the new boundary,
keep failure-fence/late-result acceptance tests (B5-01). No blanket
"BuildKit schedules all Gripsack work" claim (B5-05).

## B0 lane status (this workstation)

| Lane | Status |
|---|---|
| Linux/amd64 (local Docker 29.7.2 + Compose v5.5.0; required CI Docker 28.0.4; pinned buildkitd) | B0-01 **verified** in both declared lanes at source `ce3c7e0`; disposable harness only, not a production backend |
| macOS / Lima VM (B0-02) | **blocked** — no Mac hardware/runner; lane stays open, never inferred from Linux |
| B0-03 zero-builder baseline | native workflow probes runnable on Linux lane (grip check/plan/preview/fetch download zero builder bytes) |

## B0-01 results — Linux/amd64 lane (2026-09-24, run of `probes.sh`)

The 2026-09-24 observations below are historical, **not** passing
source-bound evidence: their artifacts were gitignored and their
Markdown-only summary lacked a report SHA-256, execution counts and an
exact tested source. A fresh run at source `6542fc9` additionally
exposed a false green: the baseline source-text search printed
`FAIL: builder references leaked into product sources` for legitimate
workspace tests/comments, but its plain-`sh` pipeline returned
`tee`'s zero and ran all later probes. The independent OCI verifier
was also piped through `tee`, and the Docker load-failure branch had
no nonzero exit. That run cannot qualify B0-01.

Environment: WSL2 Linux 6.18.33.2, docker 29.7.2, buildkitd v0.33.0
(`dddd5621`) from `moby/buildkit:v0.33.0@sha256:a461e7f0…` (amd64 leaf;
the manifest-list digest carries no amd64 manifest — recorded), Go
1.26.3 build container, in-graph toolchains digest-pinned
(`gcc:15.2.0@sha256:c101370…`, `alpine:3.22.1@sha256:eafc1ed…`).
Worker: rootful buildkitd in a privileged disposable container, TCP on
loopback, named cache volume destroyed between phases and at exit;
no `--oci-worker-no-process-sandbox` shortcut. Full artifacts:
`verification/buildkit-qualification/results/` (gitignored; summarized
here).

| Probe | Outcome |
|---|---|
| 1 baseline preservation | product sources: zero buildkit/moby references; offline e2e gate already proves native workflows fetch no builder bytes |
| 2 graph semantics | PASS (5.7s): identical re-solve CACHED `shared-dependency` + `consumer-a`; sibling `consumer-b` reused the shared dependency while rebuilding its own consumer; cancellation surfaced in 3.0s (RST_STREAM CANCEL) with the worker healthy afterwards; deliberate `exit 7` attributed to its operation with the marker present in bounded (≤64 KiB) captured logs |
| 3 retained executable | PASS (30.8s): 933,288-byte static binary (`sha256 7d177631…`) from digest-pinned gcc with `network=none` (in-graph TEST-NET-3 probe confirms); ran on the host AFTER worker container + cache volume destruction |
| 4 OCI export | PASS (2s per fresh-worker solve, warm images): layout tar 3.7 MB; independent python verifier checks every blob digest, DiffIDs, media types, extracts layers; two fresh-worker exports reproduce (DiffIDs, normalized config, file digests — manifest digests differ only by `created` timestamps, recorded); docker engine (independent of both workers) loaded and ran the image (`gripsack b0 oci fixture`) |
| 5 policy | PASS: include-pattern transport delivered only `allowed.txt` (secret + nested canaries absent); op environment holds no credential-shaped host variables; container root shows no host paths |
| 6 Mac VM | **blocked** — no Mac hardware; B0-02 lane stays open |

The old probe-1 “zero source references” line is a lexical snapshot,
not a current native dependency or no-bootstrap guarantee. The
corrected driver parses actual `Cargo.lock` packages; its isolated
checkout had no `target/debug/grip`, so binary linkage was **not**
assessed there. The independent offline real-CLI e2e gate remains
required for product behavior.

B0-03 measurements (Linux lane): buildkit worker image 267–363 MB,
in-graph gcc pull ~1.25 GB once per cold cache, alpine 13 MB, OCI
fixture output 3.7 MB, worker lifetime for probes 2+3+5: 35 s.
Initial budget rationale recorded: the optional builder costs are
scoped to explicit Linux-build requests only; native flows touch zero
of it (probe 1). Full cold/warm/offline separation including the Mac
VM lane waits for B1's managed worker; B0-03 stays `in_progress`
(Linux measurements recorded, VM lane blocked).

## B0-01 current Linux qualification (2026-09-26)

At committed source `ce3c7e0`, the qualification remains a
disposable Go/LLB harness, **not** a `grip` build backend. The
driver parses actual native Rust lock dependencies, checks
worker-health/version and independent OCI verification, and
prints `B0_LINUX_QUALIFICATION=6` only after an independent
Docker engine runs the content. Report
`verification/reports/2026-09-26-b0-linux-ce3c7e0.log`
(SHA-256 `e9359a12bd81207cbc00175e205a24388650824f642188118af9b7d6ab5c53ea`,
fingerprint `872ccc2079878af8ff598d9458345e9358afc76e2712cf5eb321d5a38cbc36fc`)
contains **6/6** Linux cases: pinned tool/image/Go-module
identities; three disposable real workers; shared graph/cancel/
bounded failure; input/credential policy; a retained static
executable after worker/cache removal; two independently decoded
and reproduced OCI exports; and Docker-engine load/run. GitHub
Docker 28 cannot load the original pure OCI layout. The
independent verifier therefore *first* checks/reproduces both
OCI exports, then repackages the same verified config/layer bytes
in a Docker-save tar. The driver compares loaded image platform,
runtime Env/cwd, tag and exact layer DiffIDs with the OCI
report before running the expected file. Docker may re-encode
config metadata and assign a different image ID: no raw Docker
image-ID equivalence or second BuildKit solve is claimed.

Three same-source real-worker negatives reject a native builder
dependency before startup, a corrupt OCI blob before conversion
and an intentionally failed Docker archive load after valid
independent OCI checks. Their SHA-256 reports are
`2026-09-26-b0-lock-mutant-ce3c7e0.log`
(`98077a3c2f47a7693c9cc38c562607788e8a4188596c6c65385853bbd591b10a`),
`2026-09-26-b0-verifier-mutant-ce3c7e0.log`
(`bd48a7999c34b1759be2595b00ba4521eefcc88cd4ab2dcb439a7ef8efd14198`),
and `2026-09-26-b0-load-mutant-ce3c7e0.log`
(`43c325646a67ea9877cc7339475c9a36fcae1965cadd3b7e3605268c0ccbeeba`).
Four further substitutions in an *actual* loaded Docker image
inspection (tag, platform, Env, layer DiffID) each fail their
named qualification-checker condition; report
`2026-09-26-b0-loaded-mutants-ce3c7e0.log`
(`5cb91dbb82e4ae43705899d8acc0de2167333129ec1fa953c7ee3aee44ff76f6`).
Earlier `8352793`/`0be1eaa`/`6909bbc` local receipts are
historical under changed source roots.

The B0-01 `linux-amd64` **and** `container-gates` lanes are now
verified at identical behavior source `ce3c7e0`. The actual full
required [PR `test` job](https://github.com/gripsack-dev/gripsack/actions/runs/36239162419/job/108396317823)
completed successfully: delivery checker **30/30** negatives,
architecture, Rust fmt/clippy/tests, TypeScript, B0 **6/6** pinned
real Go/BuildKit cases on Docker 28.0.4 with independent OCI checks,
Docker load/inspect/run, real CLI e2e **319/319**, TLC and Verus
**72 verified/0 errors** with seven mutants. Complete job output is
archived as `verification/reports/2026-09-26-b0-required-ci-ce3c7e0.log`
(SHA-256 `50b48797ce77092a311188b687d18943515a37945b6fe08fdbf6fb00b64815e3`).
This qualifies B0-01 only. The current draft PR head includes
later evidence-only ledger/plan edits, not a new implementation;
the protected-head CI status must still be judged at its actual head.
B0-02 Apple Silicon Mac-VM remains blocked; B0-03 budgets and
B0-04 production witness remain open, and B1 managed worker/
protocol plus B2 LLB lowering are **not** landed in `grip`.

### Required CI loader refusals and checked-byte correction (2026-09-26)

At exact source `0be1eaa`, protected draft PR #164's required
[`test` job](https://github.com/gripsack-dev/gripsack/actions/runs/36235965010/job/108387656393)
passed delivery calibration, architecture, Rust, TypeScript and real
Linux e2e **319/319**. The pinned B0 real daemon and independent
Python OCI verifier then produced and reproduced valid exports,
but `docker load` **rejected** the first layout; B0 failed closed.
TLC and Verus were skipped by CI and cannot be counted from that run.
The actual job step is archived byte-exact in
`verification/reports/2026-09-26-b0-ci-failed-0be1eaa.log`
(SHA-256 `5e021075706371754dbd3661c1544470cc6645264247639eb487fd118f525b5b`).
The CI checkout had saved Docker's refusal only to a gitignored
`probe4-load.txt`; the log could not establish its precise cause.
Source `fc67212` printed the actual failure: CI Docker **28.0.4**
tried `/blobs/json` while importing a pure OCI-layout tar
(`verification/reports/2026-09-26-b0-ci-failed-fc67212.log`,
SHA-256 `dedceed7658c7cd0bb727e04423f50daadad16db6dae5ef274c88b8c18c114c9`).
Source `ce3c7e0` converts only *independently verified OCI bytes*
to a legacy Docker-save archive and compares the actual loaded
image's effective platform/config/DiffIDs before execution.
The later exact-source `ce3c7e0` required `test` job succeeded with
the checked-byte correction; the older failures stay archived as
regressions, never counted as successful cases.



## B1 start — the bridge wire contract (2026-09-26)

The first production component of the backend landed at exact source
`d6560758cd4decbb3272926fba3cc5c8bd60b817`: crate `gripsack-buildkit`
with `protocol` — the bounded Rust↔Go frame transport and pure fence
kernel from this plan's B1 fences section. Length-prefixed JSON
frames validate their header against a 256 KiB cap BEFORE any body
allocation; all messages are strict tagged shapes (unknown fields
reject); `Submit` binds the exact validated definition/exporter
digests and length; log chunks cap at 64 KiB after base64 decode;
negotiation accepts exactly protocol version 1. The `EventGate`
kernel enforces epoch fencing (stale/future rejection), exactly one
terminal per session with duplicate-terminal rejection,
 cancellation-fences-acceptance, and a 4096-chunk log budget.
Dependencies: base64/serde/serde_json/thiserror/sha2 only — no
IR/store/exec/fetch coupling (Epic B §5 boundaries).

Evidence: focused unit group 7/7 plus a real fuzz target
(`buildkit_protocol`, registered in `fuzz/run.py`, 7 shipped seeds;
the round-trip law and gate-feeding run through the production
decoder) and all six gates on the identical tree (fresh Rust
fmt/clippy/tests, Deno 67/67, e2e 322/322, TLC, Verus 72/0+7
mutants, fuzz replay). Receipt
`verification/reports/2026-09-26-b1-protocol-d656075.log`, SHA-256
`893b9b1a254e882ce0cbe98b545300c49c216d03fcc181ea8dd1445c61b0a3fa`.
Scope honesty **at `d656075`**: B1-03 was partial — no Go
transport, worker provisioning or LLB lowering had landed then.
Later B1 protocol/transport packets are recorded below; `grip`
still has no BuildKit backend and no B row is verified. A process defect was
caught and fixed pre-commit: the first fuzz-gate run silently skipped
the new target because `fuzz/run.py` keeps its own TARGETS tuple.

## B1 Linux worker prototype (2026-09-26)

At exact implementation source `80b50a34f4fc10627d2ecb81495db5da215142cb`,
`gripsack-buildkit::worker::linux` exercises a **standalone Linux**
provision/lease/stop/cache lifecycle against the pinned BuildKit
v0.33.0 container. The former registry manifest-list digest stopped
resolving; `pins.env` and the provider now pin the available
`6c2fa84a…` list with the same qualified linux/amd64 leaf. The
provider requires a local `/var/run/docker.sock`, a preinstalled pinned
image (`--pull=never`), an admitted worker ID and an idle lease.
It captures Docker's immutable container ID and checks container and
volume owner labels before deletion. An ordinary stop retains the
cache; explicit owner teardown removes it. No user-owned volume is
adopted, no remote Docker context is followed, and effect output is
bounded by `gripsack-process`.

Committed-source Linux smoke: **2/2 real Docker cases** (pinned
worker provision/lease/drained stop/cache retention and explicit
teardown; foreign pre-existing volume refusal), plus **7/7**
failure/ownership/identity unit cases. The pinned Go B0 probe was
rebuilt from this source and the real Linux qualification reran
**6/6** with independently checked OCI bytes and Docker execution.
The five non-fuzz compose gates passed on the same tracked source
bytes: Rust fmt/clippy/tests fresh, TypeScript gate **cached**
(unchanged TS source), real CLI e2e **322/322**, TLC including two
worker-lease mutants, and Verus **72/0** plus seven **existing**
policy mutants. Receipt
`verification/reports/2026-09-26-b1-linux-80b50a3.log`
(SHA-256 `0769f8a44950e4c6dc22b8bd74aba5f6135ca1cfe5d45e402b96b14631968e0a`,
tracked-source fingerprint `22d6e1467b8a25f65846d63360b8405b63e8638147938f86802ae4a9e97ebfc1`).
Fuzz was **not run**, per owner request. No new protected Docker28
CI result binds the rotated manifest; the earlier B0 negative/CI
evidence remains historical at `ce3c7e0`.

**Open**: `grip` has no caller of this provider; B1-01 still needs
private sockets, Mac VM bootstrap with byte-pinned helpers, a
real daemon/platform/capability handshake, immutable declared-input
transfer and store export. B1-02 still needs two real clients, crash
reconciliation and idle upgrade. Go Submit remains fail-closed,
B1-04 needs worker effect proofs, and B2 lowering has no shipped
path. This Linux prototype does not authorize a release, a website
claim or new config guidance.

## Record

| Item | Status |
|---|---|
| B0-04 inventory | implemented_unverified (binds at B1/B2) |
| B0-01 harness | **verified historically** in `linux-amd64` and `container-gates` at `ce3c7e0`: 6/6 real Linux and required Docker28 CI with three real-worker negatives and four loaded-image mutants. The rotated identical-leaf manifest reran 6/6 on Linux at `80b50a3`, but protected Docker28/negative evidence has not been rebound to this source; qualification only, no production backend |
| B0-02 Mac VM | deferred-authorized for the 0.44 Linux integration only (`MAC-NATIVE-QUAL-2026-10-02`); native SDK tests passed, hosted VZ preflight refused unavailable virtualization; no successful VM qualification |
| B0-03 footprint | in_progress: Linux measurements + zero-builder baseline recorded; VM lane and full budgets at B1 |
| B1-01 Linux worker | **in_progress** at `80b50a3`: isolated Docker provider with 2/2 real Linux cases and 7/7 unit cases in the source-bound receipt above; no `grip` caller, private socket, full daemon handshake, Mac VM or B2 path |
| B1-02/04 kernels | **in_progress**: the pure lease kernel + CleanupSet landed at `80b7738` with a two-client TLC model and two attributed early-stop/crash-wipes mutants (receipt `2026-09-26-b1-worker-leases-80b7738.log`). Follow-up `db51101` corrected B0 baseline dependency admission without allowing upstream builder packages. The Linux prototype at `80b50a3` calls the lease kernel for single-client stop and retained-cache teardown; no real two-client recovery/upgrade, `grip` caller, TLAPS or worker Verus proof yet |
| B1-03 protocol | **in_progress**: wire contract + fence kernel (`d656075`), Go mirror (`1d43f0d`), stdio transport + Rust driver (`50b829c`), with receipts `2026-09-26-b1-protocol-d656075.log`, `-bridge-go-d0d0a4a.log`, `-transport-50b829c.log`; no BuildKit Go client/LLB submission or `grip` worker caller, and Submit still fails closed |
| Next | Connect a private, capability-checked owned worker to the Go BuildKit client and `grip`, then B2 lowering/store export; Mac VM and release proofs remain open |

## Active workspace/backend integration (2026-09-30)

Integration branch: `feat/workspace-buildkit`, based on `5a50fba`.
The separate 0.43 candidate is not changed by this work.

The next implementation boundary is the real two-phase path required by
B1-03/B2-01/B2-02: admitted workspace production graph → Go LLB lowering →
independent Rust validation of actual bytes/options → exact-byte solve →
validated host-store publication. The existing standalone Submit refusal
and the blocking, identity-incomplete transport are not production backends.

Responsibility map: the IR crate owns workspace semantics and recipe
identity; the adapter owns the admitted lowering projection, byte checker,
bounded bridge process and worker leases; the Go bridge owns upstream LLB
construction, RPC/session effects and export; exec/store retain acquisition,
publication, consumer roots and activation. Native acquisition and host
effects do not become LLB vertices. No second store or per-vertex scheduler
is introduced.

Required evidence remains the B1/B2 protocol, lease and lowering matrices,
production-used Verus decisions, TLC/TLAPS safety, exact-byte substitution
negatives, real worker cancellation/cache/check/export cases, and the
applicable native lanes. B3–B5 consumers and their prerequisites remain
part of the requested backend delivery, not silently satisfied by a bridge.
Fuzz testing and corpus replay are not run, per owner instruction; that
exception is not a substitute for any other required evidence. All new
integration claims are pending until the corresponding effects and gates
have actually run.

The first integration development receipt is
`verification/reports/2026-10-01-buildkit-bridge-development.log`.
The actual Go bridge now lowers through upstream LLB and executes received,
digest-checked definitions against the pinned real daemon. Its pinned Go
format/vet/race/build gate passed. Three standalone real-program scenarios
observed exact file export, a fail-fast isolated process, and cancellation
after an actual process-ready event. The receipt excludes an earlier invalid
network oracle and records the disposable daemon service's nonzero stop exit.
Rust-checker/CLI integration, managed lifecycle, store publication and the
formal/native/CI matrices are still unverified; no delivery row is closed.

The owner authorized temporary deferral of blocking native Mac runtime
qualification on 2026-10-02; plan/0048 §9 records exact scope, failures, risks and
resumption requirements under `MAC-NATIVE-QUAL-2026-10-02`. Linux delivery may
continue without claiming Mac VM qualification. The VZ lifecycle test remains
available for explicit execution; the diagnostic workflow disables that lane by
default. Hosted Mac builds and non-VM tests remain enabled. No source/runtime
refusal is bypassed.
