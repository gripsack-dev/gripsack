# B0-01 BuildKit/Go-LLB qualification harness (Linux lane)

Disposable qualification harness for the handover's Epic B: it verifies
the selected backend (stock BuildKit) before any production
restructuring, per `plan/0051`. It is NOT the production
`tools/buildkit-bridge` (B1/B2).

## Layout

- `pins.env` — digest-pinned images (buildkitd, Go build toolchain,
  in-graph gcc, alpine). Bumps are deliberate edits with re-recorded
  evidence.
- `bridge/` — Go program over upstream `moby/buildkit` v0.33.0
  client/LLB APIs. One file per probe; `main.go` holds the shared
  solve/stats/report plumbing. Built inside the pinned `golang`
  container (`go.sum` pins module hashes); no host Go required at
  build or run time.
- `probes.sh` — the driver: worker lifecycle (start → probe → destroy
  container AND cache volume), retained-output execution, two-fresh-
  worker reproduction, separate-runtime execution, environment
  recording.
- `verify_oci.py` — independent OCI-layout verifier (no BuildKit
  code): blob digest checks, DiffID walk, media-type allowlist,
  layer extraction, clean-build comparison; only after validation
  it packages the same checked bytes for Docker's legacy loader.
- `check_loaded.py` — compares the *actual* Docker-inspected image
  tag, platform, exact layer DiffIDs and supported runtime config
  against the verified OCI report; importer config-ID rewrites are
  recorded, not assumed byte-identical.
- `results/` (gitignored) — machine-readable probe reports,
  environment/versions, logs, exported artifacts.

## Probes (Epic B §B0, Linux lane)

1. **baseline preservation** — the parsed native Rust lock has no
   BuildKit/Docker client dependencies; ordinary comments and rejected
   input fixtures do not count as product dependencies. The separate
   offline e2e gate exercises native workflows; an available host
   `grip` binary is also checked for container-runtime library linkage.
2. **graph** — two outputs sharing one dependency: reuse (CACHED
   vertices on re-solve), sibling reuse, mid-solve cancellation with a
   healthy worker afterwards, deliberate failure attributed to its
   operation with bounded (≤64 KiB) captured logs.
3. **executable** — static Linux binary from a digest-pinned gcc with
   `network=none` (confirmed in-graph against TEST-NET-3); exported,
   then run on the host after the worker container and cache volume
   are destroyed.
4. **oci** — minimal OCI layout exported from two FRESH workers;
   independently verified (blob digests, DiffIDs, media types,
   normalized config, extracted file digests). Only after both
   exports reproduce, the independent verifier wraps those *same
   checked config/layer bytes* in a Docker-save tar with its own
   collision-checked temporary tag. The Docker engine loads it;
   an independent check compares loaded platform, runtime Env/cwd,
   exact layer DiffIDs and tag to the verified OCI evidence, then
   runs the expected file. Docker may re-encode config metadata and
   assign a different image ID; we do not claim raw config-ID parity.
   Hosted CI's Docker 28.0.4 rejected the original pure OCI tar
   (`blobs/json` not found), before this checked-byte adapter.
   No second BuildKit solve or unverified image substitutes here.
5. **policy** — include-pattern local-source transport carries only
   declared files (canaries absent), the op environment holds no
   credential-shaped host variables, container root shows no host
   paths.
6. **Mac VM qualification** — blocked: no macOS hardware on this
   workstation. Recorded as a blocked lane in `verification/delivery.json`;
   never inferred from Linux results.

## Running

```sh
sh probes.sh        # from verification/buildkit-qualification
```

Requires: docker, python3. The bridge binary is rebuilt via
`bridge/build.sh` if the Go sources change.

The driver fails closed if the baseline check, worker health/version
observation, independent OCI verification, Docker-engine image load
or runtime execution fails. Plain `sh` has no `pipefail`: probe
status is checked **before** a passing report is printed, never
inferred from `tee`'s exit code.
`probe4-verify.json` records the OCI and portable-archive digests;
`probe4-runtime.txt` records the independent engine result.
The baseline prints the actual Docker engine version/platform. A
failed image load or runtime invocation prints that operation's
captured error before refusing qualification; a hidden
`results/` file must not be the only CI diagnostic.
Only a completed six-case run prints `B0_LINUX_QUALIFICATION=6`;
the required CI `test` job records that marker after the real daemon
and independent runtime finish.

## Honesty notes

- The worker runs rootful inside a privileged disposable container on
  a development host. That is a qualification-worker configuration,
  not the production worker policy; B1 owns the constrained managed
  worker. No `--oci-worker-no-process-sandbox` shortcut is used.
- Measurements (pull bytes, timings) land in `results/environment.txt`
  per run; `plan/0051` carries the summarized evidence.
