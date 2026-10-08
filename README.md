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
- [Workspace production](#workspace-production)
- [Sourcing](#sourcing)
- [Documentation](#documentation)
- [Development](#development)

## Install

Supported hosts are **Linux, including WSL2**. macOS and native Windows are
not supported. Historical Mac releases are retained as history, not an
installation or compatibility promise.

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
- **Approve the bytes that run** — approval binds the canonical repository,
  a copied source-bundle digest, and the evaluator/runtime/native-action policy.
  Edits, changed pins or expanded grants require renewed approval.
  `grip trust inspect|add|list|remove` exposes that boundary; CI has no blanket bypass.
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

## Workspace production

Core/SDK 0.46 emits IR v7 for typed `gripsack.ts` workspaces with recipe,
package, environment, task, image and profile outputs. Strict v6 execution
keeps its original semantics. Production does not create a personal generation:

```bash
grip build service
grip run --env dev -- service "" "two words"
grip shell dev
grip task test
grip build container
grip builder cache-clean
grip gc
```

Compatible Linux production and required checks enter one checked BuildKit
graph. Source acquisition, independent export validation, immutable publication,
retention and GC remain native. The optional bridge is provisioned lazily with
its compiled-in hash. `GRIPSACK_BRIDGE_MIRROR` changes only the download origin;
`--bridge <helper>` deliberately selects an operator-provided helper. Cached outputs and
provider-only native paths need no builder bootstrap.

Profiles can cold-build their package environment and artifact files through
the existing generation/rollback transaction. Environment values remain literal
data, and artifact trees retain per-file ownership. Exported command aliases
preserve their underlying selector, including multicall binaries.
Profile hooks (`post_link`, `post_activate`, `on_remove`) execute through the
durable activation ledger — per-intent identity, replay-safe outcomes, and no
automatic rollback after a post-activation failure. Task `checks` run as
ordered postconditions after successful steps. Profile file-check stages,
schedules and task prerequisites remain explicitly unavailable.

Native Conda declarations can set
`systemRequirements: { libc: { family: "glibc", version: "2.28" }, linux: "4.18" }`.
These are explicit solve/runtime requirements, not an inference that every
archive needs the updater's libc. Package version values are MatchSpecs
(`"==25.07.1"`, not a bare `"25.07.1"`); frozen baseline changes require an
explicit update and renewed review of captured inputs.

Linux GNU dynamic commands need a loader with the sealed-launch controls
(`--argv0`, `--glibc-hwcaps-mask`, `--inhibit-cache`, `--library-path`,
`--inhibit-rpath`), measured from the bound loader itself rather than a glibc
version number — RHEL 8's glibc 2.28 backports qualify. The core separately
binds the loader and executable bytes, checks the retained library closure,
refuses audit/filter objects, legacy hwcap shadows and an active
`/etc/ld.so.preload`, and translates `$ORIGIN` paths only when lookup remains
equivalent. Shared libraries are retained and inspected, not sealed executable
images. Images use the fixed OCI exporter profile and are independently
checked for content, configuration and image-local runtime closure; deleting
the builder cache does not delete them.

For reviewed native host libraries, Linux GNU packages may declare
`hostRuntime: { libraryDirectories: ["/opt/bb/lib64"] }`. It grants only the
named native runtime capability, not evaluator access or portable image
content. The core still admits the dependency graph and sealed-loader lookup;
host-dependent persistent commands re-enter the installed core on each launch.
Host libraries remain mutable trusted inputs, not sealed package bytes.

Migration requires a real legacy apply that prunes old declarations before the
workspace claims their destinations; `--take-over` does not transfer recorded
ownership. For release trees that load physical siblings, use `trackedCopyTo`,
not per-file symlink projections. The existing executable `repoFile` + `file`
profile path can manage persistent launchers; no separate launcher constructor
is required. [Persistent environments](https://gripsack.dev/docs/environments.html)
and [migration](https://gripsack.dev/docs/workspace-migration.html) contain the
complete recipes and staging/approval boundaries.

Linux x86_64 Conda/Pixi native and OCI journeys have runtime qualification.
The supported platform scope is Linux and WSL under the owner's
`PLATFORM-LINUX-WSL-2026-10-08` decision. macOS, its VM backend, launchd and
native Mac Conda qualification are outside the active roadmap, CI and future
release targets; revisiting them requires a new explicit support decision.
Historical platform metadata/readers and release evidence retain their original
meaning, without implying current Mac support.

The **0.46.0** release target is Linux x86_64/WSL, including its native
Conda and BuildKit helpers. ARM artifacts are unavailable in this release,
not qualified by emulation or inherited pins. The installer rejects ARM and
macOS rather than silently selecting an older release. Locally produced
assets have checksum/build receipts, not GitHub-hosted build attestations.
See [plan0048](plan/0048-review-response-0.42.0.md) for
`REL-X64-FIRST-0460-2026-10-08` and source-bound qualification.

## Source approval and migration

Evaluation reads one private, read-only snapshot, including ignored/untracked
files, dirty submodules and the admitted frontend pin. Every probe round uses
that same snapshot with a separate immutable host-input file. `.git` and the
selected gripsack runtime-state subtree are excluded and unavailable to eval.
Lockfiles inside the repository are source too: an apply/update that changes
them can require renewed approval on the next command.
Ignored and untracked files are captured by design; Git ignore rules never
decide source selection. Declare literal repository-relative exclusions in
`env.toml` — `[capture] exclude = [".venv"]` — for ordinary build trees or
editor-only SDK links. The captured configuration binds those rules to the
approved bundle, and evaluator runtime grants never overlap the repository or
its exclusions.

Old path-only trust entries do not authorize execution. Unset
`GRIPSACK_TRUST_ALL`; `=1` now fails with migration guidance. Interactive
commands show the captured digest and policy before asking. For a reviewed
disposable CI fixture, pass both expected fingerprints explicitly:

```sh
source=$(grip trust inspect --json)
printf '%s\n' "$source"  # review the inventory, changes and actual policy
grip trust add \
  --bundle "$(printf '%s' "$source" | jq -r .bundle_digest)" \
  --policy "$(printf '%s' "$source" | jq -r .policy_digest)"
grip check
```

Inspection and approval do not evaluate repository code. A change between them
fails approval rather than blessing newer bytes. Remote/HEAD are sanitized
provenance, not trust keys. `grip trust inspect --receipt ID --json` inspects
private source/input/process evidence; `completed` covers frontend evaluation,
not a later build or deployment.

`adopt --yes` skips only apply confirmation. Generated source needs its own
approval. After reviewing and approving it, repeat adoption with `--resume`
and omit `--mode`; the approved module supplies the mode, no repo files are
rewritten, and takeover remains restricted to the requested target.

Source capture is not a sandbox for native plugins or protection against a
privileged/same-UID attacker modifying trusted runtime storage. It identifies
the bytes actually copied, not an atomic Git checkout or proof of source intent.

## How it works

```
your env repo (modules + env.toml + hosts/)
  → capture source + select runtime → explicit source/policy approval
  → core detects host facts, writes one immutable input per probe round
  → frontend evaluates the captured bundle in sandboxed Deno → IR (JSON, span-annotated)
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

`--host` and `[env] default_host` select one entrypoint and one
`locks/<host>.lock` file. They must be single ASCII names (letters,
digits, `_`, `-`, and `.` after the first character; no `..` or path
separators). E132 rejects unsafe names **before** tool provisioning or
host-derived file access. `grip init` sanitizes the detected machine
hostname to this spelling; role-named hosts such as `work.dev` remain
valid.

The workspace path accepts a root `gripsack.ts` without a host shim.
Use matching 0.46 core and SDK releases for the current v7 writer; older strict
cores do not admit the new fields. Retained v6 execution and native v5 file
profiles keep their original semantics. See the
[dotfile workspace](examples/workspaces/01-dotfiles/gripsack.ts) and
[workspace contract](typescript/README.md#a-workspace-is-a-function-ir-v7).
Strict v3–v6 readers preserve their historical meanings; the current SDK
does not silently add new fields to those retained formats.

Evaluation runs in Deno, spawned deny-by-default: no env vars,
network, or subprocesses. Reads are limited to the repo, injected
inputs, embedded frontend, and an explicitly detected
`@gripsack/core` pin whose canonical target proves its package name
(the pin may live outside the repo). E133 rejects a comma in **any**
granted path before Deno starts: Deno treats commas in `--allow-read`
as new path grants. Move a comma-named repo, Gripsack home, or pinned
package to an unambiguous path rather than widening permission.
Facts (os, arch, libc, hostname) are core-injected; the core never
embeds a runtime ([plan/0005](plan/0005-frontends-and-configuration.md),
[0013](plan/0013-constrained-evaluation.md)).

`env.toml` `[eval].env` is a build/fetch-child environment, not grip's
process environment: build steps, fetcher plugins and artifact
proxy/CA configuration receive it, while host facts, tool provisioning
and the Deno evaluator do not. Operator-only `GRIPSACK_*`,
`GH_HOST`/`GITHUB_HOST` and GitHub token names are rejected with E400
if declared there. Supply credentials and their host binding in the
invoking environment; the HTTP client refuses to send a bound Bearer
token to a non-HTTPS URL, including loopback. A repo may still declare
`SSL_CERT_FILE` for an artifact server's CA.

Frontend evaluation is supervised: one ten-minute budget covers all
probe rounds; stdout and stderr are each limited to 16 MiB, and error
output retains at most the final 64 KiB. Exceeding a limit fails the
operation rather than parsing a partial envelope. `grip adopt` checks
repo trust before inspecting the target or generating repo files;
`--yes` skips confirmation, not trust.

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
docker compose run --build --rm model    # finite protocol models and counterexamples
docker compose run --build --rm tlaps    # transaction induction pilot (amd64)
docker compose run --build --rm verify   # production Verus kernels and mutants (amd64)
```

The required CI `test` job runs these gates for pull requests and pushes
to `main`. The TLAPS pilot has one destination and one recovery; it is
not the required generalized transaction theorem. See [AGENTS.md](AGENTS.md)
for working agreements (docker-first, rustls-only, coordinated IR changes).

MIT licensed.
