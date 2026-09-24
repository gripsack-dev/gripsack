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
- `verify_oci.py` — independent OCI-layout verifier (no buildkit
  code): blob digest checks, DiffID walk, media-type allowlist,
  layer extraction, clean-build comparison.
- `results/` (gitignored) — machine-readable probe reports,
  environment/versions, logs, exported artifacts.

## Probes (Epic B §B0, Linux lane)

1. **baseline preservation** — the product carries zero buildkit/moby
   references; the offline e2e journey (container gate) already proves
   native workflows fetch no builder bytes.
2. **graph** — two outputs sharing one dependency: reuse (CACHED
   vertices on re-solve), sibling reuse, mid-solve cancellation with a
   healthy worker afterwards, deliberate failure attributed to its
   operation with bounded (≤64 KiB) captured logs.
3. **executable** — static Linux binary from a digest-pinned gcc with
   `network=none` (confirmed in-graph against TEST-NET-3); exported,
   then run on the host after the worker container and cache volume
   are destroyed.
4. **oci** — minimal OCI layout exported from two FRESH workers;
   independently verified (digests, DiffIDs, media types, normalized
   config, extracted file digests) and executed via the docker engine
   (a runtime independent of both torn-down workers).
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

## Honesty notes

- The worker runs rootful inside a privileged disposable container on
  a development host. That is a qualification-worker configuration,
  not the production worker policy; B1 owns the constrained managed
  worker. No `--oci-worker-no-process-sandbox` shortcut is used.
- Measurements (pull bytes, timings) land in `results/environment.txt`
  per run; `plan/0051` carries the summarized evidence.
