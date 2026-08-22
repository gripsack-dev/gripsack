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
    gripsack-source          built-in fetchers + sourcerer host (0002)
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
docker compose run --build --rm pytest    # python frontend tests
docker compose run --build --rm e2e       # flow tests (real binary + real frontend)
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

## 5. Testing pyramid

| layer | what | where |
|---|---|---|
| unit | IR validation, store hashing, topo order, sourcerer discovery | per crate, in `test` gate |
| frontend | emit shape, provenance capture, IR round-trip | `python/tests`, in `pytest` gate |
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

Scaffolded and skipped until `apply` lands (0004+); the gate runs them
from day one so the harness itself is never allowed to rot.

## 6. Workflows

| workflow | trigger | job |
|---|---|---|
| `ci` | push/PR | `test` — compose test + pytest + e2e (required check) |
| `release-core` | tag `core-v*` | musl tarball → verify (sha, static, `--version`) → **crates.io first** (irreversible) → GitHub release |
| `release-python` | tag `py-v*` | version guard → `uv build` → PyPI (`PYPI_API_TOKEN` secret) |
| `demo` | dispatch (+ paths once `apply` lands) | VHS render → `demo/artifacts` bot PR |
| `audit` | weekly | cargo audit |

Version guards fail mistagged releases instead of publishing the wrong
version: `core-vX.Y.Z` must equal the `gripsack` crate version;
`py-vX.Y.Z` must equal `python/pyproject.toml`'s.

## 7. Demo automation

VHS tapes of CLI flows (`demos/demo.tape`), rendered in the
`ghcr.io/charmbracelet/vhs` container against the freshly built musl
binary, re-rendered whenever `crates/` or `demos/` change. Changed renders
land as a reused, force-pushed `demo/artifacts` bot PR (main is
protected; bot PRs get no CI, merge with admin override). Dispatch-only
until `apply` exists; the placeholder tape exercises `--version` and
`doctor` so the pipeline itself is proven now.

## 8. Versioning and IR compatibility

- Package versions are independent; the **IR version** is the real
  contract. The core declares the IR range it accepts; the frontend
  declares what it emits; `grip doctor` flags mismatches.
- Pre-1.0: keep package versions loosely synced to spare confusion.
- IR readers MUST tolerate unknown fields (forward compatibility);
  writers never emit fields the schema doesn't describe.

## 9. Branch protection

`main`: PRs only, `test` check required, linear history. The repo owner
merges with admin override (bot PRs never get CI — expected).
