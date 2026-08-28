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
docker compose run --build --rm ts-test   # typescript frontend tests (deno)
docker compose run --build --rm e2e       # flow tests (real binary + real frontend)
docker compose run --build --rm -e VERSION=x.y.z release   # musl tarball → ./dist/
```

`--build` after every source change or the image runs stale code. Host
`cargo test` / `pytest e2e` are fine for fast iteration; always finish
with the compose gates. `./dist/` and `target/` may contain root-owned
files (container mounts) — delete via docker or `sudo`.

## Where things are

| Path | Contents |
|---|---|
| `plan/` | numbered decision docs — read before changing behavior |
| `schema/ir/v1.json` | THE contract between frontend and core |
| `crates/gripsack-ir` | IR types + validation (mirrors the schema) |
| `crates/gripsack-store` | store paths, generations, GC |
| `crates/gripsack-exec` | DAG scheduling |
| `crates/gripsack-fetch` | built-in fetchers + `gripfetch-*` plugin host + tool provisioning (deno) |
| `crates/gripsack` | the `grip` CLI |
| `typescript/` | the typed module DSL (npm `@gripsack/core`); also embedded in the binary |
| `e2e/` | pytest flow tests against the real binary + golden IR corpus |
| `demos/` | VHS tapes |
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
- **Provenance is mandatory** — every IR node carries `source: {file,
  line}` from the frontend that emitted it.
- The core never evaluates code and never sees credentials. The
  frontend runs sandboxed (no env vars, no network, no subprocesses);
  host facts arrive via the inputs envelope, effects as probes. The
  lockfile is the sole source of pinning.
- Never auto-rollback on post-activation hook failure (0001 §3.8).

## Workflow

- `main` is protected: PRs only, `test` check required. The repo owner
  merges with admin override — bot PRs (demo artifacts) never get CI,
  that's expected.
- The `demo` workflow re-renders `demos/demo.gif` and opens a reused
  `demo/artifacts` PR — merge those to keep the README current.
- Releases: two artifacts, two tag namespaces.
  - Core: tag `core-vX.Y.Z` matching the `gripsack` crate version →
    musl tarball verified → crates.io → GitHub release.
  - TypeScript: tag `ts-vX.Y.Z` matching `typescript/package.json` →
    npm `@gripsack/core` (needs the `NPM_TOKEN` secret).
- The website lives in `gripsack-dev/gripsack-dev.github.io` — edit
  there, not here.
