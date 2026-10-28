# @gripsack/core

The TypeScript workspace SDK for [gripsack](https://gripsack.dev). Supported hosts
are Linux and WSL2 on x86_64. The core evaluates the repository's **`gripsack.ts`**
in provisioned, sandboxed Deno: no network, subprocesses or environment-variable
access, and reads confined to approved captured sources. Host facts, tags,
settings and probe answers are injected, not discovered by authoring code.

## Declare a workspace

```ts
// gripsack.ts
import {
  defineWorkspace, workspace, profile, file, repoFile, identity, symlinkTo,
} from "@gripsack/core";

export default defineWorkspace(() => workspace({
  outputs: [profile("dotfiles", {
    files: [file({
      source: repoFile("config/editor.toml"),
      content: identity(),
      destination: symlinkTo("~/.config/editor/config.toml"),
    })],
  })],
}));
```

The default export synchronously returns a workspace value. Compose declarations
with ordinary functions and arrays; falsy output entries are omitted. Names are
unique across the output catalog. The context is `{ facts, tags, probe, settings }`.
`ctx.probe.executable("ninja")` and `ctx.probe.file_exists("/path")` request core-bound
observations. Unbound probes return false and are recorded with source locations;
the core re-evaluates after binding answers. `when` and `hasTag` are pure helpers
for this injected context.

Only the root `gripsack.ts` is an entrypoint. Host identity does not select a
TypeScript filename. IR9 emits exactly `{ ir_version: 9, host, workspace }`.
Declaration spans capture the caller's file, line and column for diagnostics.
Use matching core and SDK versions. A deliberate repository-local SDK pin must
export both `@gripsack/core` and `@gripsack/core/advanced`; changing frontend identity
can require a lock update and renewed exact-digest source approval.

## Packages and environments

```ts
import {
  conda, defineWorkspace, environment, pkg, provider, targetPlatform, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64", abi: "gnu" });
const tools = pkg("tools", {
  producer: provider(conda.environment({
    channels: ["conda-forge"],
    packages: { python: "3.12.*", ripgrep: "*" },
    systemRequirements: {
      libc: { family: "glibc", version: "2.28" }, linux: "4.18",
    },
  })),
  commands: { python: "bin/python", rg: "bin/rg" },
  target,
  layout: { kind: "prefix_materialized" },
});

export default defineWorkspace(() => workspace({
  outputs: [tools, environment("dev", { packages: ["tools"], target })],
}));
```

`conda.environment` solves one coherent environment. `pixi.fromLock({ manifest,
lock, environment })` imports an existing Pixi lock; manifest and lock name
captured workspace inputs. It does not re-solve or upgrade the external lock.
Use `inputFile`/`inputDirectory` declarations for captured producer inputs.

Other acquisition constructors are `githubRelease`, `tarball`, `git`, `fileFetch`
and `pluginFetch`. A `provider(source)` acquires directly; a `recipe` declares
source, execution policy, target and output kind for source-built artifacts.
`cargoPackage` provides Cargo-oriented package authoring. A `pkg` names its
producer, commands, target and installation layout. Select archive roots explicitly
when an upstream archive has a wrapper directory.

Run `grip update` to record selections, then `grip build tools` or
`grip run --env dev -- rg --version`. Frozen consumers use the recorded selection
without re-solving. Review changed captured source and policy digests before
approving them; lock updates do not bypass approval.

## Profile files

`file` combines independent origin, content and destination axes:

| Axis | Constructors |
| --- | --- |
| Origin | `repoFile`, `repoTree`, `artifactFile`, `artifactTree` |
| Content | `identity`, `literalText`, `templateText`, `templateFile` |
| Destination | `symlinkTo`, `trackedCopyTo`, `managedBlock` |

Only `literalText` needs no origin. Repository paths are relative to the captured
repository. Artifact selectors are normalized relative paths. `repoTree` selections
are enumerated by the core, not by TypeScript filesystem traversal. `templateFile`
renders captured source bytes with bound variables. `symlinkTo` owns a read-only
store-backed destination; `trackedCopyTo` detects drift; `managedBlock(path, marker)`
shares a file with other writers. Profiles may also select an environment and
managed command launchers. Review destination conflicts before applying; never
silently overwrite foreign files.

## Commands, tasks and checks

`exec` builds argv-based commands. `lit`, `hostPath`, `packageCommand`, `sourcePath`,
`outputPath` and artifact references distinguish literal text from resolved paths.
`bash`/`bashBody` and `runBash` are explicit shell execution, not implicit string
interpolation. Command declarations are immutable.

`task` declares steps, prerequisites, optional environment, mutation locks and
postcondition checks. `check` names a subject and command. `hook` declares a
lifecycle command; external side effects must be idempotent across replay.
`schedule` targets a task with `daily`/`weekly` calendar triggers. `image` composes
packages into an OCI image with explicit target and destination policy. Authoring
a declaration does not grant execution authority: the core validates references,
platform constraints and required approvals before effects.

See [workspace examples](../examples/workspaces/) for dotfiles, native tools,
source-built packages and scheduled tasks. The exported TypeScript interfaces are
the full field-level reference.

## Compiler interface

Ordinary authoring imports `@gripsack/core`. Compiler integrations explicitly use
`@gripsack/core/advanced`, which exports `emitWorkspaceIr`, `IR_VERSION`,
`parseInputs`, `createProbeBuilder` and workspace node types. The driver emits
`{ ir, diagnostics, probe_requests }`. Source-located authoring failures are
structured diagnostics; syntax and engine failures remain tracebacks.
