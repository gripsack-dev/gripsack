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

Evidence honesty note (2026-09-24): the probe outcomes below were observed
green on this workstation, but they are recorded only in this Markdown
document — the run artifacts under
`verification/buildkit-qualification/results/` are gitignored and were not
preserved as a repo-local, SHA-256-hashed, count-bearing report bound to an
exact source commit/dirty identity. Under the hardened delivery checker the
B0-01 row is therefore `implemented_unverified` (not verified) until the
probe suite is rerun with portable, source-bound runner evidence in
`verification/reports/`. The observations themselves stand and are preserved
in the ledger's `historical_claims`.

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

B0-03 measurements (Linux lane): buildkit worker image 267–363 MB,
in-graph gcc pull ~1.25 GB once per cold cache, alpine 13 MB, OCI
fixture output 3.7 MB, worker lifetime for probes 2+3+5: 35 s.
Initial budget rationale recorded: the optional builder costs are
scoped to explicit Linux-build requests only; native flows touch zero
of it (probe 1). Full cold/warm/offline separation including the Mac
VM lane waits for B1's managed worker; B0-03 stays `in_progress`
(Linux measurements recorded, VM lane blocked).

## Record

| Item | Status |
|---|---|
| B0-04 inventory | implemented_unverified (binds at B1/B2) |
| B0-01 harness | implemented_unverified — observed green on Linux/amd64 2026-09-24 (probes above), but evidence is this Markdown record only; verified requires a portable source-bound runner report (harness in `verification/buildkit-qualification/`) |
| B0-02 Mac VM | blocked (no Mac) |
| B0-03 footprint | in_progress: Linux measurements + zero-builder baseline recorded; VM lane and full budgets at B1 |
| Next | B1 gated on A1 + qualified lane; Mac gate stays open |
