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

## Probes are requests, not effects

The sandbox cannot run probes, so `ctx.probe.executable(name)` /
`ctx.probe.file_exists(path)` return the **bound** answer from the
inputs (absent → `false`) and record unbound calls into the eval
envelope as `probe_requests`. The core evaluates them (PATH lookup /
absolute-path stat) and re-runs eval with the answers bound — a
fixpoint, capped at 4 rounds. Probe results re-evaluate every run:
plug in a GPU and the next plan changes with zero repo changes.

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
| modules | `module`, `define`, `Module`, `ModuleSpec`, `ModuleValue` |
| probes | `ctx.probe` (`executable`, `file_exists`), `ProbeRequest` |
| facts | `HostFacts` (core-injected), `when`, `hasTag`, `Condition` |
| graph | `emitIr`, `mergeTags`, `IR_VERSION`, `parseInputs` |
| fetchers | `githubRelease`, `tarball`, `git`, `fileFetch`, `pluginFetch`, `brew`, `pixi` |
| destinations | `symlink`, `trackedCopy`, `merge`, `template` |
| dependencies | `dep(module, edge?)` |
| activation | `service`, `fonts`, `desktopEntry`, `customHook` |
| steps | `step`, `fetchStep`, `buildStep`, `installStep`, `configStep`, `runStep`, `shellStep` |
| verify | `verifyBinary`, `verifyFile`, `verifyShell`, `verifyDeployed` |
| resources | `resource`, `CORE_RESOURCES` |

Everything is fully typed — your editor gives you autocomplete and
inline errors for free.

## Development

```
deno task test     # frontend contract + sandbox driver tests
npm run build      # tsc — typecheck + emit the npm dist
```

API is pre-alpha and will change with the IR schema
([plan](https://github.com/gripsack-dev/gripsack/tree/main/plan)).
