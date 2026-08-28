<div align="center">

<img src="doc/logo.svg" alt="gripsack" width="480">

**gripsack — your whole environment in one bag**

**status: alpha** — the core flow ships: apply, plan, check,
generations, rollback, gc, why-owns, init; fetchers for github
releases, brew, git, tarballs, pixi, and `gripfetch-*` plugins; config
linters; ownership modes (symlink, tracked copy, managed block,
template). See [the plan](plan/) and the
[roadmap](https://gripsack.dev/docs/roadmap.html) for what's next.

[![ci](https://github.com/gripsack-dev/gripsack/actions/workflows/ci.yml/badge.svg)](https://github.com/gripsack-dev/gripsack/actions/workflows/ci.yml)
[![audit](https://github.com/gripsack-dev/gripsack/actions/workflows/audit.yml/badge.svg)](https://github.com/gripsack-dev/gripsack/actions/workflows/audit.yml)
[![crates.io](https://img.shields.io/crates/v/gripsack.svg)](https://crates.io/crates/gripsack)
[![npm](https://img.shields.io/npm/v/@gripsack/core.svg)](https://www.npmjs.com/package/@gripsack/core)
[![website](https://img.shields.io/badge/website-gripsack.dev-89b4fa)](https://gripsack.dev/)
[![status](https://img.shields.io/badge/status-design-yellow)](https://github.com/gripsack-dev/gripsack/tree/main/plan)

Packages from any source **plus** your dotfiles, described once in typed
TypeScript, deployed by a single static Rust binary into a
hash-addressed store — with generations, rollback, and no daemon, no
root, no sandbox dogma.

![demo](demos/demo.gif)

</div>

## Contents

- [What it does](#what-it-does)
- [How it works](#how-it-works)
- [The frontend](#the-frontend)
- [Sourcing](#sourcing)
- [Documentation](#documentation)
- [Development](#development)

## Install

```bash
cargo install gripsack          # the grip binary (static musl)
npm i @gripsack/core            # types for your IDE (optional; the
                                # frontend source ships inside grip)
```

Your first eval downloads the pinned, hash-verified Deno runtime
(~40MB, once, into gripsack's own cache) — eval runs sandboxed in it.


## What it does

- **Modules** describe everything: how to get a tool, build it, where
  its files and configs live. Modules depend on modules; build-only
  deps are ephemeral.
- **Any source** — GitHub releases, tarballs, git builds, cargo, your
  company's internal registry. Fetchers are pluggable; the escalation
  ladder is built-in args → `gripfetch-*` plugins
  ([plan/0002](plan/0002-sourcing.md)).
- **Sandboxed eval, explicit effects** — your config is normal typed
  TypeScript, but evaluation sees no environment variables, no
  network, and no subprocesses; host facts arrive core-injected, and
  probes (`ctx.probe`) are explicit, inspectable requests the core
  binds ([plan/0013](plan/0013-constrained-evaluation.md)).
- **A trust decision before code runs** — the first eval of an
  unfamiliar env repo prompts: path, remote, commit, and the exact
  capability set eval will get. `grip trust list|add|remove` manages
  it; `GRIPSACK_TRUST_ALL=1` is the CI hatch.
- **Dotfiles, first-class** — per-file ownership: `owned` symlinks,
  `tracked-copy` with drift detection, `merge` blocks, `template` for
  per-machine values. Dotfiles-only modules are a first-class usage
  level ([plan/0006](plan/0006-gradual-migration.md)).
- **Generations** — every apply (one module or the whole graph) is a new
  generation; activation is one atomic symlink flip; rollback is
  flipping it back.
- **Pinned, not hermetic** — fetches are impure by default,
  reproducible by pinned URLs and content hashes, per host.
- **Compiler-grade errors** — every IR node carries a source span;
  diagnostics are structured with stable codes. An LSP is a shim away
  ([plan/0004](plan/0004-rich-ir-and-passes.md)).

## How it works

```
your env repo (modules + env.toml + hosts/)
  → core detects host facts, writes the inputs envelope
  → frontend evaluates modules in sandboxed Deno → IR (JSON, span-annotated)
  → lockfile pins URLs + hashes per host
  → core passes: parse → validate → resolve → lower → plan
  → fetch & build as a DAG into /store/<hash>-<name>
  → one atomic flip: current → generations/N
```

## The frontend

One frontend: typed TypeScript ([npm `@gripsack/core`][npm] for IDE
types; the source ships embedded in the grip binary, so a repo needs
no install to eval). A host entrypoint returns the environment;
modules are values, not registrations:

```ts
// hosts/laptop.ts
import { defineEnv } from "@gripsack/core";
import { helix } from "../modules/helix.ts";
import { steam } from "../modules/steam.ts";

export default defineEnv((ctx) => ({
  tags: ["gui", "work"],
  modules: [
    helix,
    ctx.facts.os === "linux" && steam,          // falsy entries drop
    ctx.probe.executable("nvidia-smi") && cuda, // explicit probe
  ],
}));
```

Evaluation runs in Deno, spawned deny-by-default: no env vars, no
network, no subprocesses, read-only within the repo. Facts (os, arch,
libc, hostname) are detected by the core and injected — the same repo
on the same host always yields the same graph. The core never embeds a
runtime ([plan/0005](plan/0005-frontends-and-configuration.md),
[0013](plan/0013-constrained-evaluation.md)).

[npm]: https://www.npmjs.com/package/@gripsack/core

## Sourcing

Resolution happens in the core at lock/update time; transport happens
in the core at fetch time. Bespoke transport (mTLS, non-HTTP, internal
registries) gets a `gripfetch-*` plugin speaking NDJSON over stdio,
with the core verifying every byte against the lockfile.

## Documentation

| Doc | Contents |
|---|---|
| [0001 — architecture](plan/0001-architecture.md) | modules, store, generations, ownership, activation, invariants |
| [0002 — sourcing](plan/0002-sourcing.md) | resolvers, transports, fetchers |
| [0003 — repo & tooling](plan/0003-repo-and-tooling.md) | layout, docker gates, CI, releases |
| [0004 — rich IR & passes](plan/0004-rich-ir-and-passes.md) | spans, diagnostics, compiler passes, LSP |
| [0005 — frontends & config](plan/0005-frontends-and-configuration.md) | TypeScript, env.toml, evaluation order |
| [0006 — gradual migration](plan/0006-gradual-migration.md) | dotfiles-only adoption, coexistence |
| [0013 — constrained evaluation](plan/0013-constrained-evaluation.md) | sandboxed Deno eval, injected facts, probes, trust gate |

## Development

```bash
docker compose run --build --rm test     # fmt + clippy -D warnings + cargo test
docker compose run --build --rm ts-test  # typescript frontend tests (deno)
docker compose run --build --rm e2e      # flow tests (offline, fixture env repos)
```

CI runs all three gates on every push. See [AGENTS.md](AGENTS.md) for
working agreements (docker-first, rustls-only, IR changes touch all
three sides).

MIT licensed.
