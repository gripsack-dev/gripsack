# 0003 — Repo layout, build system, CI, and releases

- Status: draft
- Date: 2026-08-22
- Amends: 0001 §6 (components)

## 1. Monorepo, two packages

The IR schema is a three-party contract (schema, Python emitter, Rust
consumer). One repo makes every contract change a single atomic commit and
lets CI run the real frontend against the real core. Precedent: ruff/uv
publish PyPI wheels from a Rust repo. Our case is easier — the frontend is
pure Python (no PyO3/maturin), so its wheel is a trivial hatchling build.

Independent release pipelines, one repo. Split later only if a community
demands it.

## 2. Layout

```
gripsack/                    github.com/gripsack-dev/gripsack
  plan/                      decision docs (0001, 0002, 0003, …)
  schema/ir/v1.json          IR JSON Schema — THE contract, versioned
  crates/
    gripsack                 CLI binary `grip`      → crates.io `gripsack`
    gripsack-ir              IR types + validation
    gripsack-store           store paths, generations, GC
    gripsack-exec            DAG scheduling
    gripsack-fetch          built-in fetchers + fetcher plugin host (0002)
  python/                    typed module DSL, emits IR → PyPI `gripsack`
  e2e/                       uv+pytest flow tests against the real binary
  demos/                     VHS tapes
  .agents/skills/            maintainer skills (private)
  Dockerfile  docker-compose.yml  .github/workflows/
```

Names are free on both registries (checked 2026-08-22). No stub squatting —
both prohibit it (PEP 541, crates.io policy); we claim the names with the
first real 0.1 publish.

## 3. Docker-first gates (the house rule)

The host has no toolchain contract; the image does. Same discipline as
rootle:

```
docker compose run --build --rm test      # fmt + clippy -D warnings + cargo test
docker compose run --build --rm ts-test   # TypeScript frontend tests and examples
docker compose run --build --rm e2e       # flow tests (real binary + real frontend)
docker compose run --build --rm model     # finite protocol models + counterexamples
docker compose run --build --rm tlaps     # transaction induction pilot + calibration (amd64)
docker compose run --build --rm verify    # production Verus kernels + calibration (amd64)
docker compose run --build --rm -e VERSION=x.y.z release   # musl tarball → ./dist/
```

`--build` after every source change or the image runs stale code. Host
`cargo test` / `uv run pytest` are fine for iteration; always finish with
the compose gates. `./dist/` and `target/` may contain root-owned files.

## 4. North star: one static musl binary

- Base image `rust:alpine` — host triple is already
  `x86_64-unknown-linux-musl`; plain `cargo build` produces a static
  binary. The release stage proves it with `ldd`.
- **rustls only. Never openssl.** TLS via rustls keeps the static musl
  build trivial forever. Any dependency that drags in openssl is a bug.
- Shipping image is `FROM scratch` with just the binary.
- Install artifact contract (for install.sh, later):
  `gripsack-<VERSION>-x86_64-unknown-linux-musl.tar.gz` containing `grip`.

The active host/release scope is Linux, including WSL2
(`PLATFORM-LINUX-WSL-2026-10-08`, plan0048). Linux x86_64 and aarch64 release
targets retain their separate artifact qualification. macOS/native Windows are
not supported; historical target metadata, readers and released assets do not
establish an active platform promise.

## 5. Testing pyramid

| layer | what | where |
|---|---|---|
| unit | IR validation, store hashing, topo order, fetcher discovery | per crate, in `test` gate |
| frontend | emit shape, provenance capture, IR round-trip | `typescript/test`, in `ts-test` gate |
| e2e flow | the product working | `e2e/`, in `e2e` gate |

E2E drives the real binary + real frontend against a **fixture env repo**
in a sandboxed `HOME`, offline (`file://` fixture tarballs — no network).
The contract it defends:

1. `grip apply` → store paths created, symlinks deployed, generation 1
   registered.
2. Re-apply with one module changed → new generation; untouched modules
   reuse store paths (no refetch).
3. `grip apply neovim` → only that module's subtree changes; new
   generation still totals the whole profile.
4. `grip rollback` → previous generation restored exactly.
5. `tracked-copy` drift is detected and reported.

These flows execute against the real binary and sandboxed frontend.
The protocol/induction/kernel gates complement them; they do not prove
the filesystem or replace the concrete persistence campaigns.

## 6. Workflows

| workflow | trigger | job |
|---|---|---|
| `ci` | main push / PR / dispatch | required `test`: Rust + TypeScript + Linux e2e/persistence + TLC + TLAPS + Verus; fuzz, docs and audit have separate jobs |
| `release-core` | tag `core-v*` | musl tarball → verify (sha, static, `--version`) → **crates.io first** (irreversible) → GitHub release |
| `release-typescript` | tag `ts-v*` | package-version guard → npm tests/build/dual-form pack → npm → GitHub release (`NPM_TOKEN`) |
| `demo` | dispatch (+ paths once `apply` lands) | VHS render → `demo/artifacts` bot PR |
| `audit` | weekly | cargo audit |

Version guards fail mistagged releases instead of publishing the wrong
version: `core-vX.Y.Z` must equal the `gripsack` crate version;
`ts-vX.Y.Z` must equal `typescript/package.json`'s.

## 7. Demo automation

VHS tapes of CLI flows (`demos/demo.tape`), rendered in the
`ghcr.io/charmbracelet/vhs` container against the freshly built musl
binary, re-rendered whenever `crates/` or `demos/` change. Changed renders
land as a reused, force-pushed `demo/artifacts` bot PR (main is
protected; bot PRs get no CI, merge with admin override). Dispatch-only
until `apply` exists; the placeholder tape exercises `--version` and
`doctor` so the pipeline itself is proven now.

## 8. Versioning and IR compatibility

- The core and TypeScript SDK speak one current **IR version**. As of the
  owner-directed 0.50 cutover ([0055](0055-workspace-only-product.md)), that is
  IR9 and the workspace is its only envelope. Obsolete readers, schemas and
  authoring APIs are removed; historical bytes remain in Git history.
- Keep core, SDK and shipped helper versions coordinated while pre-1.0.
- Strict tagged admission and `deny_unknown_fields` reject structural extras.
  An incompatible wire change bumps `ir_version`; writers emit only fields
  described by that schema. Do not add compatibility shims to preserve an
  obsolete authoring API.
- Persisted generations, locks, receipts and recovery journals are separate
  contracts. Preserve retained ownership and rollback data; never reinterpret
  old persistent bytes merely because the authoring IR changed.

## 9. Branch protection

`main` uses strict `local-practical-qualification` status enforcement.
Changes go through PRs; the owner may merge with the existing admin exemption.
