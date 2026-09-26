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
| Linux/amd64 (docker 29.7.2 + compose v5.5.0; buildkitd via pinned image) | ready — B0-01 harness is the next concrete action |
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

The committed `0be1eaa` driver reports native Rust dependencies
from the parsed lock instead of rejecting harmless comments/tests.
It checks baseline, worker-health/version and OCI verifier exits
before accepting an output, requires independent Docker-engine
load/run, and prints `B0_LINUX_QUALIFICATION=6` **only after**
every named case succeeds. Its source-bound real-worker report
`verification/reports/2026-09-26-b0-linux-0be1eaa.log`
(SHA-256 `95bf677bbc5a55c57b3a7736d75d46bf3ecec9bbdbf67daecf72af3feb75c983`,
fingerprint `32d73e886142cb8e758222128128faf9368f9ccf417a146437fa716cb0fcc31b`)
records **6/6** Linux cases, pinned tool/image/Go-module identities,
three disposable workers with actual health/version witnesses, a
retained static executable run after worker/cache removal, two
independently decoded and reproduced OCI exports, and Docker-engine
load/run after both workers were destroyed.

Three attributable negatives at the **same source** reject a native
builder dependency before worker startup (dirty lock patch digest
recorded), an actually corrupted exported OCI blob through the real
independent verifier, and an intentionally failed Docker-engine
load *after* valid independent OCI checks. None prints the
six-case success marker. SHA-256 reports:
`2026-09-26-b0-lock-mutant-0be1eaa.log`
(`414165f69675f427e2eeada45e32b08defa15411a69e9f4f05c245dd9edd217d`),
`2026-09-26-b0-verifier-mutant-0be1eaa.log`
(`910b1b1c83e9cad672adea60aae8b1f62ac64d1804cc5f76dc4cad6cd1015279`),
and `2026-09-26-b0-load-mutant-0be1eaa.log`
(`a21d692b295240cc197f61625b0acd3451cc84e69591a0f159001259d0dcb21c`).
Earlier `8352793`/`ace9496` receipts are historical after the
required-CI workflow and runner marker changed behavior roots.

The B0-01 `linux-amd64` lane is verified. Its `container-gates`
lane is wired into the **required** CI `test` job, but is not verified
until an exact-source successful job/report actually runs this harness
alongside all five Compose gates. The row remains
`implemented_unverified`. B0-02's Apple Silicon virtualization/VM
lane remains blocked; the privileged Linux qualification worker
is not B1's managed Mac worker.

## Record

| Item | Status |
|---|---|
| B0-04 inventory | implemented_unverified (binds at B1/B2) |
| B0-01 harness | implemented_unverified — `linux-amd64` lane source-bound **6/6** and three intended negatives at `0be1eaa`; the required CI test job now runs B0, but the `container-gates` lane and row await observed exact-source completion |
| B0-02 Mac VM | blocked (no Mac) |
| B0-03 footprint | in_progress: Linux measurements + zero-builder baseline recorded; VM lane and full budgets at B1 |
| Next | B1 gated on A1 + qualified lane; Mac gate stays open |
