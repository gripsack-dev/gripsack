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

## A workspace is a function (unreleased IR v6)

This workspace API requires the matching development core and SDK in
this checkout; the published 0.42.0 pair predates it.

A root `gripsack.ts` — preferred over `hosts/<name>.ts` when present —
default-exports `defineWorkspace` and returns a `workspace({ outputs })`
value. No hostname selection and no fake host file: the core injects
the same facts/probes context, and the emitter produces the v6
workspace envelope (`{ir_version: 6, host, workspace}`):

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
bytes are pinned only when core resolves the lockfile. The current
command wire has no resolved Bash digest or strict-options field yet.

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
pre-evaluation check. Recipe commands execute only through explicit production;
the native file-profile path below does not execute command declarations.

Recipe execution is explicit. `{ kind: "host", access: "unconfined" }`
declares host access but still requires its native executor. Isolated recipes
declare `kind: "isolated_linux"`, `worker: "buildkit"`, a Linux `platform`, and
`toolchain: { reference: "<registry/image>@sha256:<digest>" }`. They do not run
during `grip check`. Native downloads are provider-backed packages, not a
`recipe` "native" execution mode.
Targets may declare an ABI (`gnu`/`musl` on Linux, `darwin` on macOS)
and `minimum_os: { major, minor, patch? }`. Fixed-prefix packages use
`{ kind: "fixed_prefix", prefix: "/opt/tool" }`; selecting one into an
environment requires that environment's matching `prefix`.

`grip check` evaluates and admits the workspace without a host file or
`env.toml` and lists named outputs. Native **file-only profiles** now
execute: repository files are captured once, literal/template content
is prepared before deployment, and the same content can be linked,
copied or merged as a managed block. `check` and `plan` prepare without
deploying; `apply` uses the existing generations, ownership and journal;
`rollback` restores retained bytes without rerendering current inputs.
File-only `update` validates these inputs without writing a host lock.

`grip update --check` surveys external workspace resolutions without publishing
sources or writing `gripsack.lock`; `grip update` publishes the per-platform
source and captured frontend/import pins. Repository file sources use the
current approved snapshot rather than an old checkout stored in the lock.
Pin changes are captured inputs and therefore participate in trust approval.

`grip build <recipe-package-or-image> --json` returns retained native output paths.
Provider-only and already-retained outputs need no worker. New compatible Linux
production uses one checked solve through the pinned, hash-verified bridge
helper, provisioned lazily into `$GRIPSACK_HOME/tools` on first use
(`--bridge <helper>` selects a deliberate operator override instead;
`GRIPSACK_BRIDGE_MIRROR` redirects only the download origin). Four platform
hashes are measured from independent pinned-toolchain builds and compiled into
the core. Unreleased source builds need a matching artifact mirror or override
until the matching core release publishes those assets. The owned worker is qualified for
Linux x86_64 with Docker, not for Mac or other architectures.

`sourcePath(selector)` and `outputPath(selector)` bind immutable input and
writable staging paths without embedding host paths. A check's source is its
immutable subject. These values cannot grant staging authority to tasks/hooks.
Required checks gate publication; exported commands and artifacts survive
`grip builder cache-clean` because the native store owns retention.

`cargoPackage(name, spec)` returns a producer/package pair to spread into
`workspace.outputs`. Its source must include `Cargo.lock` and the vendored
dependency tree with `.cargo/config.toml`. Select a digest-pinned official Rust
image with native linker tools, an explicit Linux `gnu` or `musl` target, and a
map of public command names to Cargo binary targets. The helper runs release
tests and builds with `--frozen --offline`, installs only the named binaries,
and removes intermediate Cargo output before export. Cargo's vendor checksums
remain enforced; the lockfile alone does not provide dependency bytes.

```ts
const service = cargoPackage("service", {
  source: fileFetch("service"),
  toolchain: "docker.io/library/rust@sha256:a10e64dd139b7387337c7fbe8aca31b959b57b2fd4c8ae20a02cf1d6ea424dce",
  target: { os: "linux", arch: "x86_64", abi: "musl" },
  binaries: { service: "service" },
});
// Include ...service in workspace.outputs.
```

`image(name, { packages, target, base?, destinations?, config? })` uses the
BuildKit OCI exporter, not a native image assembler. An omitted base means
scratch; an explicit base must be digest-pinned and contributes no inherited
runtime configuration. Package placements default to `/opt/gripsack/<name>`.
`destinations` can set an explicit `{ path, owner: { uid, gid } }`; overlapping
placements and leftover base files inside a package prefix are rejected.
`config` declares typed literal/package-command `entrypoint`, string `args`,
`env`, absolute `cwd`, and numeric `user: { uid, gid }`.

The returned image path is an OCI archive. Native admission checks descriptor
sizes/SHA-256, gzip DiffIDs, platform/configuration, timestamps, package
bytes/links/modes/ownership and unselected payloads before publication, including
cache hits. Static Linux package/project/image reuse and two independent clean
exports are exercised; dynamic-loader and coherent Conda image qualification
remain separate gates.

Profiles may select the same compatible environment used by `grip run` and
deploy `artifactFile` or `artifactTree` sources. A cold `apply` realizes required
packages before host mutation, then transfers its held lifecycle authority
directly into the existing generation transaction. It does not require a manual
pre-build. Exported command aliases and literal environment values persist with
the generation; rollback reuses retained package bytes without a worker.

`artifactTree` expands selected paths/subtrees into individual owned file
entries, with exclusions winning. It never replaces the destination directory:
foreign children remain, removals use existing drift/prune policy, and expanded
collisions fail before destination writes. Symlinks and special source entries
are refused rather than followed. Profile hooks, schedules, staged file checks,
host recipe execution and remaining platform qualification stay gated.
Historical v4 workspaces remain read-only pending A5 migration. `check` never
bootstraps BuildKit or a scheduler. `adopt` does not edit workspace TypeScript
automatically: declare the file policy and use explicit `apply --take-over`
when reversible adoption of a foreign tracked-copy destination is intended.

Four [graduated workspaces](../examples/workspaces/) use the real SDK
without a hostname shim or a synthetic package: [dotfiles only](../examples/workspaces/01-dotfiles/gripsack.ts),
[an offline-staged native tool and profile](../examples/workspaces/02-native-tool/gripsack.ts),
[a source recipe with alternate producer and typed command consumer](../examples/workspaces/03-source-built/gripsack.ts),
and [manual plus scheduled tasks](../examples/workspaces/04-scheduled-task/gripsack.ts).
The `ts-test` gate strictly type-checks their source, and `e2e` admits
each through the real `grip check`. The dotfile example can be planned
and applied natively. The other archives are executable offline fixtures;
their environment/profile and scheduler consumer gates remain separate.

The legacy `hosts/<name>.ts` path emits a v6 modules-compatibility
envelope until the A5 migration. Strict v3/v4/v5 readers preserve historical
wire meanings; current fields are not silently added to an older declaration.

`grip check --json` emits one versioned document on stdout:
`{version: 1, ok, diagnostics, host?, outputs?, modules?, layouts?}`.
Diagnostics use the same codes, labels, spans and help as terminal
output; failures exit nonzero. Readable snippets come only from files
under the evaluated repo's pinned directory capability, at most 1 MiB
per file. Out-of-repo labels remain visible without reading their
contents; JSON carries diagnostic facts, not source-file bytes.
Operational failures such as a denied trust gate still report stderr
and may have no structured diagnostic in the JSON document.

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
deno run --no-remote --cached-only --no-lock --no-config \
    --node-modules-dir=manual --import-map=<captured frontend>/deno.json \
    --allow-read=<captured repo>,<captured frontend>,<optional captured pin>,<input file> \
    <captured frontend>/src/cli.ts <captured repo> --inputs <input file>
```

and reads one JSON line off stdout:
`{"ir": …, "diagnostics": [], "probe_requests": […]}`. The IR (JSON)
is the only contract — the Rust core never executes your code. A
repo's own `node_modules/@gripsack/core` install still wins when it
shadows the embedded copy (the deliberate-pin rule); stale pins fail
with instructions.

Approval binds canonical repository identity, the copied read set and the
runtime/grant policy. Ignored and untracked imports count as source; edits,
pin changes and expanded native policy require renewed approval. Every probe
round uses the same captured bundle. `grip trust inspect --json` exposes the
inventory and fingerprints; non-interactive `grip trust add` requires both
`--bundle` and `--policy`. Old path-only entries and `GRIPSACK_TRUST_ALL=1`
do not authorize evaluation.

## API overview

| area | exports |
|---|---|
| hosts | `defineEnv`, `Env`, `EnvContext`, `EnvFn` |
| workspace | `defineWorkspace`, `workspace`, `WorkspaceValue`, `WorkspaceContext` |
| outputs | `recipe`, `pkg`, `environment`, `task`, `schedule`, `check`, `image`, `profile`, `hook`, `provider`, `targetPlatform` |
| commands/files | `exec` (object or fluent), `bash`, `bashBody`, `runBash`, `file`, `lit`, `artifact`, `hostPath`, `packageCommand`, `repoFile`, `artifactFile`, `identity`, `literalText`, `templateText`, `symlinkTo`, `trackedCopyTo`, `managedBlock`, `daily`, `weekly` |
| legacy modules | `module`, `ModuleSpec`, `ModuleValue` (A5 migration inventory) |
| probes | `ctx.probe` (`executable`, `file_exists`), `ProbeBuilder`, `ProbeKind` |
| facts | `HostFacts` (core-injected), `when`, `hasTag`, `Condition` |
| captured inputs / pure locks | `inputFile`, `inputDirectory`, `input`, `lock`, `ensureArtifact` |
| fetchers | `githubRelease`, `tarball`, `git`, `fileFetch`, `pluginFetch`, `brew`, `pixi` |
| destinations | `symlink`, `trackedCopy`, `merge`, `template` |
| dependencies | `dep(module, { for? })` |
| activation | `service`, `fonts`, `desktopEntry`, `customHook` |
| steps | `step`, `fetchStep`, `buildStep`, `installStep`, `configStep`, `runStep`, `shellStep` |
| verify | `verifyBinary`, `verifyFile`, `verifyShell`, `verifyDeployed` |
| legacy resources | `resource` (A5 migration inventory; new workspaces use pure `lock` values) |

Compiler integrations explicitly import `@gripsack/core/advanced` for
`emitIr`, `emitWorkspaceIr`, `IR_VERSION`, `mergeTags`, `parseInputs`,
`createProbeBuilder`, `Inputs`, `ProbeRequest` and compiler node DTOs.
These are absent from the ordinary installed-package root and its embedded
import-map equivalent. Registry-reset utilities are internal, not advanced SDK.
The driver resolves both entry points from the same deliberate pin; historical
pins retain their original compiler ABI rather than mixing package instances.

`inputFile`/`inputDirectory` declare captured inputs; task `context.mutable_paths`
describes live host paths instead. `lock(scope, key)` returns an immutable value:
only locks reachable through returned task declarations enter the workspace.
Equivalent scope/key pairs intern without import registration or reset calls.
They grant neither worker ownership nor execution/network authority.

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
