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
TypeScript filename. IR10 emits exactly `{ ir_version: 10, host, workspace }`.
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

Conda sources can additionally declare `wheels: [{ name, version, url, sha256 }]`.
Each entry names an exact binary distribution: canonical lowercase hyphenated
PEP503 name, exact version, HTTPS `.whl` URL and lowercase 64-hex SHA256 digest.
For example, `wheels: [{ name: "my-tool", version: "1.0",
url: approvedWheelUrl, sha256: approvedWheelDigest }]` selects reviewed wheel bytes.
Credentials, query strings, fragments, encoded filenames and duplicate names,
URLs or hashes are rejected. These are not a PyPI dependency solve: source
distributions, editable installs and build hooks are not admitted. The native
helper checks wheel metadata against the declared name/version. Wheel entries
inherit acquisition source provenance. Pixi retains its lockfile-selected wheel
contract.

Other acquisition constructors are `githubRelease`, `tarball`, `git`, `fileFetch`
and `pluginFetch`. A `provider(source)` acquires directly; a `pkg` names its
producer, commands, Linux x86_64 target and installation layout. Select archive
roots explicitly when an upstream archive has a wrapper directory.
Executable recipes, Cargo builds, OCI image production and macOS/Homebrew
authoring are not supported.

`provider(source.package({ manifest }))` selects a previously imported portable
package by its lowercase 64-hex signed-manifest digest. Import transport and trust
configuration remain core-owned; consuming the package requires current trust.

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

`exec` builds argv-based commands. `lit`, `hostPath`, `packageCommand` and artifact
references distinguish literal text from resolved paths. Command declarations are
immutable; constructing one does not execute it.

Use the `bash` tag for literal scripts, then bind an explicitly declared package
interpreter. For example, with a package named `shell` exporting `bash`:

```ts
import {
  bash, check, hook, hostPath, lit, packageCommand, runBash, task,
} from "@gripsack/core";

const interpreter = packageCommand("shell", "bash");
const body = bash`
  printf '%s\n' "\${MESSAGE:-ready}"
`;
const command = body.interpreter(interpreter)
  .env("MESSAGE", lit("two words; not shell code"))
  .cwd(hostPath("/tmp"))
  .build();

const hello = task("hello", { steps: [command] });
// "personal" is a separately declared profile/check subject.
const ready = check("ready", { subject: "personal", run: command });
const activated = hook("activated", { trigger: "post_link", run: command });

// Object form lowers through exactly the same command constructor.
const same = runBash({
  interpreter, body,
  env: { MESSAGE: lit("two words; not shell code") },
  cwd: hostPath("/tmp"),
});
```

The interpreter is resolved from the package's pinned artifact, never discovered
from `PATH`; `packageCommand` can additionally require an executable SHA-256.
Bash always receives `-e -u -o pipefail`. Options cannot silently override these
strict defaults. Typed environment values and cwd paths remain typed until native
execution; env values are not pasted into the script.

Leading-newline literals are dedented, with each generated line mapped to its
original source line. Shell escapes such as `\n` remain verbatim. Escape the
JavaScript template delimiters: `\${NAME}` authors Bash `${NAME}`, and an escaped
backtick authors a shell backtick. Immediately before either template delimiter,
write `2n+1` backslashes to retain `n` shell backslashes (for example,
`\\\${NAME}` emits `\${NAME}`). Elsewhere backslashes are unchanged.
`$NAME`, `$(command)`, quoting, heredocs and parameter expansion are Bash syntax,
not JavaScript interpolation.

JavaScript substitutions (`${expression}` without the template escape) are type
errors and are rejected with a source-located diagnostic at runtime. JavaScript
evaluates substitution expressions **before** calling any tag: this API does not
sandbox those expressions or authorize their effects. Use literal scripts and
typed `.env(...)` bindings instead. Plain object-form strings support one-line
scripts; use `bash` for multiline bodies so diagnostics retain their source map.

`task` declares steps, prerequisites, optional environment, mutation locks and
postcondition checks. `check` names a subject and command. `hook` declares a
lifecycle command; external side effects must be idempotent across replay.
`schedule` targets a task with `daily`/`weekly` calendar triggers. Authoring a
declaration does not grant execution authority: the core validates references,
platform constraints and required approvals before effects.

See [workspace examples](../examples/workspaces/) for dotfiles, native tools and
scheduled tasks. The exported TypeScript interfaces are the full field-level
reference.

## Managed services

`service(name, spec)` declares a persistent OCI service, not a finite task or
calendar trigger. Its image must be an immutable `name@sha256:<64 lowercase hex>`
reference. Declare loopback-only `ports: [{ address, host, container }]`, named
`volumes: [{ name, target }]` and an explicit command health check:

```ts
const database = service("database", {
  image: approvedPostgresImage,
  ports: [{ address: "127.0.0.1", host: 5432, container: 5432 }],
  volumes: [{ name: "postgres-data", target: "/var/lib/postgresql/data" }],
  env: {
    POSTGRES_PASSWORD: { kind: "inherited", name: "POSTGRES_PASSWORD" },
  },
  health: {
    kind: "command", argv: ["pg_isready"],
    interval_seconds: 2, timeout_seconds: 5, retries: 20,
  },
});
```

Import `service` from `@gripsack/core`; `approvedPostgresImage` is your reviewed
digest-pinned image reference, not a mutable tag. Environment bindings are explicit
`{ kind: "literal", value }` or `{ kind: "inherited", name }` values.
`depends_on` names other services. Optional `command` supplies an argv override.

Start services explicitly with `grip up`. A task's `services: ["database"]`
requires an already healthy service; it does not authorize automatic startup.
Profiles do not autostart services. Bind mounts, host networking, privileged
containers and restart policy are not authoring options. Stopping a service does
not delete its data volume; database data has no implicit rollback guarantee.

## Compiler interface

Ordinary authoring imports `@gripsack/core`. Compiler integrations explicitly use
`@gripsack/core/advanced`, which exports `emitWorkspaceIr`, `IR_VERSION`,
`parseInputs`, `createProbeBuilder` and workspace node types. The driver emits
`{ ir, diagnostics, probe_requests }`. Source-located authoring failures are
structured diagnostics; syntax and engine failures remain tracebacks.
