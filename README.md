<div align="center">

<img src="doc/logo.svg" alt="gripsack" width="480">

**gripsack — your whole environment in one bag**

**status: alpha (v0.1)** — the core flow works: apply, generations,
rollback. Fetchers are file/tarball for now; see
[the plan](plan/) for what's next.

[![ci](https://github.com/gripsack-dev/gripsack/actions/workflows/ci.yml/badge.svg)](https://github.com/gripsack-dev/gripsack/actions/workflows/ci.yml)
[![audit](https://github.com/gripsack-dev/gripsack/actions/workflows/audit.yml/badge.svg)](https://github.com/gripsack-dev/gripsack/actions/workflows/audit.yml)
[![crates.io](https://img.shields.io/crates/v/gripsack.svg)](https://crates.io/crates/gripsack)
[![pypi](https://img.shields.io/pypi/v/gripsack.svg)](https://pypi.org/project/gripsack/)
[![npm](https://img.shields.io/npm/v/@gripsack/core.svg)](https://www.npmjs.com/package/@gripsack/core)
[![website](https://img.shields.io/badge/website-gripsack.dev-89b4fa)](https://gripsack.dev/)
[![status](https://img.shields.io/badge/status-design-yellow)](https://github.com/gripsack-dev/gripsack/tree/main/plan)

Packages from any source **plus** your dotfiles, described once in typed
Python or TypeScript, deployed by a single static Rust binary into a
hash-addressed store — with generations, rollback, and no daemon, no
root, no sandbox dogma.

![demo](demos/demo.gif)

</div>

## Contents

- [What it does](#what-it-does)
- [How it works](#how-it-works)
- [Frontends](#frontends)
- [Sourcing](#sourcing)
- [Documentation](#documentation)
- [Development](#development)

## Install

```bash
cargo install gripsack          # the grip binary (static musl)
pip install gripsack            # python frontend  ·  npm i @gripsack/core
```

## What it does

- **Modules** describe everything: how to get a tool, build it, where
  its files and configs live. Modules depend on modules; build-only
  deps are ephemeral.
- **Any source** — GitHub releases, tarballs, git builds, cargo, your
  company's internal registry. Fetchers are pluggable; the escalation
  ladder is built-in args → Python/TS resolvers → `gripfetch-*`
  plugins ([plan/0002](plan/0002-sourcing.md)).
- **Dotfiles, first-class** — per-file ownership: `owned` symlinks,
  `tracked-copy` with drift detection, `merge` blocks, `template` for
  per-machine values. Dotfiles-only modules are a first-class usage
  level ([plan/0006](plan/0006-gradual-migration.md)).
- **Generations** — every apply (one module or the whole graph) is a new
  generation; activation is one atomic symlink flip; rollback is
  flipping it back.
- **Lockfiles, not sandboxes** — impure by default, reproducible by
  pinned URLs and content hashes, per host.
- **Compiler-grade errors** — every IR node carries a source span;
  diagnostics are structured with stable codes. An LSP is a shim away
  ([plan/0004](plan/0004-rich-ir-and-passes.md)).

## How it works

```
your env repo (modules + env.toml + hosts/)
  → frontend evaluates modules → IR (JSON, span-annotated)
  → lockfile pins URLs + hashes per host
  → core passes: parse → validate → resolve → lower → plan
  → fetch & build as a DAG into /store/<hash>-<name>
  → one atomic flip: current → generations/N
```

## Frontends

| | python | typescript |
|---|---|---|
| package | PyPI `gripsack` | npm `@gripsack/core` |
| IDE | pyright | tsc (native) |
| runtime | your python ≥3.10 | your node ≥18 or bun |

One frontend per env repo, declared in `env.toml`. Both emit the same
IR — the core never embeds either runtime
([plan/0005](plan/0005-frontends-and-configuration.md)).

## Sourcing

Resolution happens at eval (arbitrary, credentialed Python/TS in your
repo); transport happens in the core. Internal registry? A resolver in
your env repo's `lib/` usually suffices — the skill travels with your
dotfiles. Bespoke transport (mTLS, non-HTTP) gets a `gripfetch-*`
plugin speaking NDJSON over stdio, with the core verifying every byte
against the lockfile.

## Documentation

| Doc | Contents |
|---|---|
| [0001 — architecture](plan/0001-architecture.md) | modules, store, generations, ownership, activation, invariants |
| [0002 — sourcing](plan/0002-sourcing.md) | resolvers, transports, fetchers |
| [0003 — repo & tooling](plan/0003-repo-and-tooling.md) | layout, docker gates, CI, releases |
| [0004 — rich IR & passes](plan/0004-rich-ir-and-passes.md) | spans, diagnostics, compiler passes, LSP |
| [0005 — frontends & config](plan/0005-frontends-and-configuration.md) | TypeScript, env.toml, evaluation order |
| [0006 — gradual migration](plan/0006-gradual-migration.md) | dotfiles-only adoption, coexistence |

## Development

```bash
docker compose run --build --rm test     # fmt + clippy -D warnings + cargo test
docker compose run --build --rm pytest   # python frontend tests
docker compose run --build --rm ts-test  # typescript frontend tests
docker compose run --build --rm e2e      # flow tests (offline, fixture env repos)
```

CI runs all four gates on every push. See [AGENTS.md](AGENTS.md) for
working agreements (docker-first, rustls-only, IR changes touch all
three sides).

MIT licensed.
