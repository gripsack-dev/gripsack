# @gripsack/core

TypeScript frontend for [gripsack](https://gripsack.dev) — a typed
module DSL whose evaluation the `grip` core runs inside a **sandboxed,
provisioned Deno** (no environment variables, no network, no
subprocesses, read-only within your repo). Every host observation
(facts, tags, probes) is injected by the core; the frontend returns a
value; effects are explicit probe requests the core binds and feeds
back ([plan/0013](https://github.com/gripsack-dev/gripsack/tree/main/plan)).

## A host is a function

```ts
// hosts/laptop.ts
import { defineEnv } from "@gripsack/core";
import { helix } from "../modules/helix.js";

export default defineEnv((ctx) => ({
  tags: ["gui", "work"],
  modules: [
    helix,
    ctx.facts.os === "linux" && steam,          // falsy entries drop out
    ctx.probe.executable("nvidia-smi") && cuda, // symbolic: bound by the core
  ],
}));
```

`ctx` is core-injected: `{ facts, tags, probe, settings }`. Nothing is
registered by import side effect — the function *returns* the
environment, so `Inputs → Environment` is testable and cacheable.

## A workspace is a function (IR v5)

A root `gripsack.ts` — preferred over `hosts/<name>.ts` when present —
default-exports `defineWorkspace` and returns a `workspace({ outputs })`
value. No hostname selection and no fake host file: the core injects
the same facts/probes context, and the emitter produces the v5
workspace envelope (`{ir_version: 5, host, workspace}`):

```ts
// gripsack.ts
import { defineWorkspace, workspace, recipe, pkg, targetPlatform } from "@gripsack/core";
import { githubRelease } from "@gripsack/core";

const tools = recipe("tools", {
  source: githubRelease({ repo: "example/tools", asset: "tools-{version}.tar.gz" }),
  execution: { kind: "host", access: "unconfined" },
  output_kind: "tree",
  target: targetPlatform({ os: "linux", arch: "x86_64" }),
});

export default defineWorkspace(() =>
  workspace({
    outputs: [
      tools,
      pkg("tools-bin", {
        producer: "tools", // or provider(githubRelease({…})) — no synthetic recipe
        commands: { tools: "bin/tools" },
        target: targetPlatform({ os: "linux", arch: "x86_64" }),
        layout: { kind: "relocatable" },
      }),
    ],
  }));
```

Outputs are the nine typed kinds — `recipe`, `pkg`, `environment`,
`task`, `schedule`, `check`, `image`, `profile`, `hook` — each a pure,
frozen value with a mandatory source span. Duplicate catalog names
show both declaration sites; invalid references show the referring
declaration (and the conflicting target where applicable); dependency
cycles show the causal path. `runBash` bodies are literal text (`${…}`
interpolation is rejected; dynamic values enter through typed
`env`/`argv` refs) and require a declared `packageCommand` interpreter,
never ambient Bash. The reference identifies a package command; its
bytes are pinned only when core resolves the lockfile. The read-only
v5 wire has no resolved Bash digest or strict-options field yet.

Object and immutable fluent command forms lower to the same command
IR, aside from their declaration spans:

```ts
import { bash, bashBody, exec, lit, packageCommand } from "@gripsack/core";

const args = exec(packageCommand("tools", "jq"))
  .arg(lit("--sort-keys")).arg(lit("two words"))
  .env("MODE", lit("strict")).build();
// Same command as exec({ argv: [packageCommand("tools", "jq"),
//   lit("--sort-keys"), lit("two words")], env: { MODE: lit("strict") } }).

const script = bash(packageCommand("shell", "bash"))
  .body(bashBody`
    echo "$INPUT"
  `)
  .env("INPUT", lit("two words")).build();
```

`bashBody` captures the template's source line before dedenting, so a
generated script error can map back to the original line. A plain
string is supported only for single-line `runBash({ body })`; multiline
strings without an original body location are rejected rather than
given a false `line_map`. JavaScript evaluates template expressions
before calling a tag: do not put `${…}` expressions in `bashBody`.
The frontend rejects them when invoked, but this is **not** a static
pre-evaluation check. No workspace command executes before A2.

Recipe execution is explicit: `{ kind: "host", access: "unconfined" }`
declares host filesystem/kernel/network access, or
`{ kind: "isolated_linux", worker: "buildkit" }` requests the later B2
worker; neither runs during `grip check`. Native downloads are
provider-backed packages, not a `recipe` "native" execution mode.
Targets may declare an ABI (`gnu`/`musl` on Linux, `darwin` on macOS)
and `minimum_os: { major, minor, patch? }`. Fixed-prefix packages use
`{ kind: "fixed_prefix", prefix: "/opt/tool" }`; selecting one into an
environment requires that environment's matching `prefix`.

This A1 authoring surface remains **read-only**. `grip check` evaluates and admits
the workspace without a host file or `env.toml` and lists named outputs.
`grip plan`, `apply` and `update` reject workspace execution with E124
before host effects; A2/A2-P/E/B own realization and scheduling. An
`isolated_linux` declaration does not bootstrap a builder in `check`.
The legacy `hosts/<name>.ts` path emits a v5 modules-compatibility
envelope until the A5 migration; the core keeps v3 and historical v4
readers. It is not a workspace executor.

## Probes are requests, not effects

The sandbox cannot run probes, so `ctx.probe.executable(name)` /
`ctx.probe.file_exists(path)` return the **bound** answer from the
inputs (absent → `false`) and record unbound calls into the eval
envelope as `probe_requests`. The core evaluates them (PATH lookup /
absolute-path stat) and re-runs eval with the answers bound — a
fixpoint, capped at 4 rounds. Probe results re-evaluate every run:
plug in a GPU and the next plan changes with zero repo changes.

Probe a *stable* reference, never the tool's own installed presence —
`!ctx.probe.executable("node") && node` oscillates, because installing
the tool makes the next eval drop the module. Gate on the specific
system path you must not overwrite:
`!ctx.probe.file_exists("/opt/vendor/bin/node") && node`.

## Evaluation

The core spawns the embedded driver under Deno with deny-by-default
permissions:

```
deno run --no-remote --cached-only --no-lock \
    --allow-read=<repo>,<inputs dir>,<frontend dir> \
    <frontend>/src/cli.ts <repo> --inputs <path>
```

and reads one JSON line off stdout:
`{"ir": …, "diagnostics": [], "probe_requests": […]}`. The IR (JSON)
is the only contract — the Rust core never executes your code. A
repo's own `node_modules/@gripsack/core` install still wins when it
shadows the embedded copy (the deliberate-pin rule); stale pins fail
with instructions.

First eval of an unfamiliar repo is an explicit trust decision
(`grip trust add`), recorded in `$GRIPSACK_HOME/trust.toml`.

## API overview

| area | exports |
|---|---|
| hosts | `defineEnv`, `Env`, `EnvContext`, `EnvFn` |
| workspace | `defineWorkspace`, `workspace`, `emitWorkspaceIr`, `WorkspaceValue`, `WorkspaceContext` |
| outputs | `recipe`, `pkg`, `environment`, `task`, `schedule`, `check`, `image`, `profile`, `hook`, `provider`, `targetPlatform` |
| commands/files | `exec` (object or fluent), `bash`, `bashBody`, `runBash`, `file`, `lit`, `artifact`, `hostPath`, `packageCommand`, `repoFile`, `artifactFile`, `identity`, `literalText`, `templateText`, `symlinkTo`, `trackedCopyTo`, `managedBlock`, `daily`, `weekly` |
| modules | `module`, `define`, `Module`, `ModuleSpec`, `ModuleValue` |
| probes | `ctx.probe` (`executable`, `file_exists`), `ProbeRequest` |
| facts | `HostFacts` (core-injected), `when`, `hasTag`, `Condition` |
| graph | `emitIr`, `mergeTags`, `IR_VERSION`, `parseInputs` |
| fetchers | `githubRelease`, `tarball`, `git`, `fileFetch`, `pluginFetch`, `brew`, `pixi` |
| destinations | `symlink`, `trackedCopy`, `merge`, `template` |
| dependencies | `dep(module, { for? })` |
| activation | `service`, `fonts`, `desktopEntry`, `customHook` |
| steps | `step`, `fetchStep`, `buildStep`, `installStep`, `configStep`, `runStep`, `shellStep` |
| verify | `verifyBinary`, `verifyFile`, `verifyShell`, `verifyDeployed` |
| resources | `resource`, `CORE_RESOURCES` |

Everything is fully typed — your editor gives you autocomplete and
inline errors for free.

## Build dependencies and IR v3

`dep("compiler", { for: "build" })` supplies store artifacts to the
consumer's build/custom/run steps through a prepended PATH and
`GRIP_DEP_COMPILER`. A dependency referenced only by build edges has
no destinations, activation hooks, or shell-profile exports; a runtime
incoming edge wins. Both modules still belong in the host environment.
Build closures follow build edges transitively, with graph-ordered
PATH and deduplicated members. Runtime deps of a build tool still
deploy normally. This is not a hermetic environment.

Core/TypeScript **0.36.0 uses IR v3**. Remove module/step `retries`:
the field was accepted but did not execute retries; it is now rejected.
Update a pinned `@gripsack/core` to `^0.36.0`, or remove it to use the
embedded frontend. v1/v2 input is rejected before decoding. Dependency
purposes still use `for`, not `edge`, with `dep(name, { for: "build" })`.
Old lockfiles and generations remain readable; rollback restores files
without rebuilding. GC retains closure paths for every retained generation.

## Execution contracts

Explicit steps and declarative fields share source staging, config lint
and module-level verification. Do not mix `steps` with
`fetch`/`build`/`install`/`config`/`activate` fields.

Cross-module `needs: ["producer:step"]` waits for the producer module's
pre-activation work, including verification. This is module-granular
ordering, not a global step scheduler. It adds no deployment role or PATH
export; use `dep` for those. Activation targets, self-qualified refs and
cycles are rejected. Use sibling ids within a module.

Build, shell and run steps are cached artifact recipes. Declared `outputs`
must exist after shell and run actions; omitting outputs does not mean
"always run". Put repeatable activation effects in `customHook` instead.

## Development

```
deno task test     # frontend contract + sandbox driver tests
npm run build      # tsc — typecheck + emit the npm dist
```

API is pre-alpha and will change with the IR schema
([plan](https://github.com/gripsack-dev/gripsack/tree/main/plan)).
