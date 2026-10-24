# @gripsack/core

TypeScript frontend for [gripsack](https://gripsack.dev) — a typed
module DSL whose evaluation the `grip` core runs inside a **sandboxed,
provisioned Deno** (no environment variables, no network, no
subprocesses, read-only within your repo). Every host observation
(facts, tags, probes) is injected by the core; the frontend returns a
value; effects are explicit probe requests the core binds and feeds
back ([plan/0013](https://github.com/gripsack-dev/gripsack/tree/main/plan)).

The supported `grip` hosts are Linux and WSL2 on x86_64. macOS and native Windows are
outside the current support scope. Historical target metadata remains readable;
it does not grant a supported execution platform.

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

## A workspace is a function (IR v8)

Use matching **0.47.0 core and SDK** releases for captured templates/trees,
archive-root selection and managed profile launchers. Older strict cores do not
admit v8. The new core retains strict v7/v6/v5 execution with each version's
original field set, v4 read-only workspaces and v3 historical modules; older
envelopes never acquire v8 authority.

Linux and WSL2 on x86_64 are the supported host scope. Installing the npm package does
not upgrade the core binary. Workspace locks capture frontend identity, so
an intentional SDK change can require explicit `grip update` and reviewed
exact-digest source approval.

A root `gripsack.ts` — preferred over `hosts/<name>.ts` when present —
default-exports `defineWorkspace` and returns a `workspace({ outputs })`
value. No hostname selection and no fake host file: the core injects
the same facts/probes context, and the emitter produces the v8
workspace envelope (`{ir_version: 8, host, workspace}`):

```ts
// gripsack.ts
import {
  conda, defineWorkspace, environment, pkg, provider, targetPlatform, workspace,
} from "@gripsack/core";

const target = targetPlatform({ os: "linux", arch: "x86_64", abi: "gnu" });
const ripgrep = pkg("ripgrep", {
  producer: provider(conda.environment({
    channels: ["conda-forge"],
    packages: { ripgrep: "*" },
    systemRequirements: {
      libc: { family: "glibc", version: "2.28" },
      linux: "4.18",
    },
  })),
  commands: { rg: "bin/rg" },
  target,
  layout: { kind: "prefix_materialized" },
});

export default defineWorkspace(() => workspace({
  outputs: [ripgrep, environment("search", { packages: ["ripgrep"], target })],
}));
```

Run `grip update` to record the complete selection, then `grip build ripgrep`
or `grip run --env search -- rg --version`. Frozen consumers use those records
without re-solving. Provider packages do not need a synthetic recipe or worker.
Lock updates change captured inputs: review and approve the new source/policy
snapshot when prompted before the next build. This is not an approval bypass.

`conda.environment({ channels, packages })` describes one coherent Conda
environment, not independently solved packages. `pixi.fromLock({ manifest,
lock, environment })` explicitly imports a named Pixi environment; `manifest`
and `lock` name captured workspace inputs. Both namespaces are exported from
`@gripsack/core` and its deliberate-pin entrypoint. The old `pixi(package)`
constructor is removed; retained legacy IR remains versioned data, not a
second workspace producer API. Conda/Pixi native and image qualification is
tracked separately in plan 0052; this authoring surface alone is not evidence
that every consumer/platform is qualified.

For `pixi.fromLock`, `grip update` imports the selected external lock; it does
**not** solve, upgrade packages or rewrite `pixi.lock`. To change packages,
update the external manifest/lock with your chosen Pixi workflow first, then
review those files, run `grip update`, and approve the changed captured inputs.
Frozen builds use the imported identities without solving. Keep explicit
runtime dependencies such as pyright's `xorg-libx11`; no broad host fallback is
implied by an environment import.

`packages` values are version MatchSpecs: use `"==25.07.1"` for exact equality,
`"=25.07"` for a version-prefix match, `">=25.07.1,<26"` for a range, or `"*"`.
A bare `"25.07.1"` is not a valid constraint. The resulting lock always fixes
exact archive identities.

`systemRequirements` explicitly selects the corresponding Linux solve
virtuals; omitted libc/kernel capabilities still use updater measurements.
The declared policy is recorded and checked on frozen use. A solve's measured
`__glibc=2.39` is not by itself every selected archive's runtime floor. Native
archive constraints, actual CPU/libc/kernel facts and sealed-loader controls
remain separate checks. An explicit baseline is not universal portability proof.

Outputs are the nine typed kinds — `recipe`, `pkg`, `environment`,
`task`, `schedule`, `check`, `image`, `profile`, `hook` — each a pure,
frozen value with a mandatory source span. Duplicate catalog names
show both declaration sites; invalid references show the referring
declaration (and the conflicting target where applicable); dependency
cycles show the causal path. `runBash` bodies are literal text (`${…}`
interpolation is rejected; dynamic values enter through typed
`env`/`argv` refs) and require a declared `packageCommand` interpreter,
never ambient Bash. The reference identifies a package command; its
bytes are pinned when the core resolves and publishes the package. Strict Bash
options are explicit on the v6 wire; an authored digest claim is checked against
the publication receipt, never treated as executable authority.

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
profile hooks and attached file checks have their own lifecycle stages below.

Recipe execution is explicit. Production recipes declare
`kind: "isolated_linux"`, `worker: "buildkit"`, a Linux `platform`, and
`toolchain: { reference: "<registry/image>@sha256:<digest>" }`. They do not run
during `grip check`. A `{ kind: "host", access: "unconfined" }` declaration is
admitted as contract data but host recipe execution remains gated. Native
downloads are provider-backed packages, not a `recipe` "native" execution mode.
Supported Linux targets may declare `abi: "gnu"` or `"musl"` and
`minimum_os: { major, minor, patch? }`. Fixed-prefix packages use
`{ kind: "fixed_prefix", prefix: "/opt/tool" }`; selecting one into an
environment requires that environment's matching `prefix`.

A Linux GNU package may explicitly declare
`hostRuntime: { libraryDirectories: ["/opt/bb/lib64"] }` for reviewed host
runtime dependencies and absolute ELF search paths. This is a host-coupled
package policy, not portable retained content, an evaluator read grant or a
BuildKit host mount. Undeclared directories remain refused. Native invocation
re-admits the dependency graph and preserves sealed-loader search equivalence;
host-dependent persistent wrappers re-enter the absolute installed core path.
Keep that core path and private state available. Host libraries remain trusted,
mutable host inputs, not sealed package bytes, and such packages cannot be
silently placed into an OCI image.

An ABI selects compatibility and the loader, not ambient library access.
For GNU releases requiring `libm.so.6`, `libgcc_s.so.1` or `libstdc++.so.6`,
retain compatible libraries in the package or an explicit `runtime: ["runtime"]`
package edge, or review a host library directory containing them. For example,
`/usr/lib/x86_64-linux-gnu` is an explicit Ubuntu host-dependent choice, not a
portable default; review RHEL's `/usr/lib64` separately. X11 still needs its own
closure dependency (`xorg-libx11` for the reported pyright environment).
An `ET_DYN` ELF header alone does not imply dynamic execution: static PIE without
an interpreter or needed libraries is admitted as static; dynamically interpreted
payloads must declare their actual ABI.

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
`GRIPSACK_BRIDGE_MIRROR` redirects only the download origin). Linux helper
hashes are measured from independent pinned-toolchain builds and compiled into
the core. Unreleased source builds need a matching artifact mirror or override
until the matching core release publishes those assets. The owned worker is
qualified for Linux x86_64 with Docker; this does not claim other-architecture
worker qualification. macOS is outside the support scope.

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
cache hits. Linux x86_64 static, GNU dynamic and coherent Conda consumers have
runtime evidence, including independent read-only/no-network OCI execution and
clean exports. Other host/image platforms need their own qualification.

Profiles may select the same compatible environment used by `grip run` and
deploy `artifactFile` or `artifactTree` sources. A cold `apply` realizes required
packages before host mutation, then transfers its held lifecycle authority
directly into the existing generation transaction. It does not require a manual
pre-build. Exported command aliases and literal environment values persist with
the generation; rollback reuses retained package bytes without a worker.

### Captured files, trees and stable package roots

Keep the body in one captured file, not duplicated in a TypeScript string:

```ts
file({
  source: repoFile("scripts/editor-theme.sh.tmpl"),
  content: templateFile({ theme: "dark" }),
  destination: trackedCopyTo("~/.local/bin/editor-theme"),
});
```

`templateFile(variables, resultDigest?)` renders `{{ theme }}` from the approved
source bytes. An executable `repoFile` remains executable through rendering
and either tracked-copy or symlink deployment. Preserve the executable bit in
your repository; this is normalized executable classification, not arbitrary
mode/owner/ACL preservation. `literalText` has no executable source and produces
0644 files; `templateText` embeds a body instead of reading a captured template.
The optional result digest is checked, not an authority bypass. Neither form
needs evaluation-time `readFileSync`, Deno I/O or subprocesses.

```ts
file({
  source: repoTree(".claude/skills", {
    include: ["d2"],
    exclude: ["d2/scratch"],
  }),
  content: identity(),
  destination: trackedCopyTo("~/.claude/skills"),
});
```

Includes/excludes are literal, normalized segment prefixes relative to the
selected tree, **not globs**: `d2` includes that subtree, not `d20`; `*.d2`
does not mean “all D2 files”. Empty/omitted includes select all; exclusions win.
Safe internal file/directory aliases expand to logical destinations:
`d2/recipes/theme.d2 -> ../theme.d2` deploys `d2/recipes/theme.d2` backed by the
captured target's bytes/mode. Trees require `identity()`, not per-file templates.
The core rejects escapes, dangling targets, cycles, collisions and budget
violations. Add/remove/retarget changes participate in captured identity and
the deployment diff. Each expanded file is owned separately; foreign siblings
are not replaced.

Selection does **not** bypass whole-source capture. Capture rejects unsafe
aliases/cycles before `repoTree` can filter them. Exclude an irrelevant outbound
`.venv` link at the capture boundary with `env.toml`:

```toml
[capture]
exclude = [".venv"]
```

These literal repository-relative exclusions become approved capture policy;
they are not `.gitignore` rules or grants to read the excluded tree. Keep an
alias's target in the approved bundle. The current SDK replaces `treeFiles`
with `repoTree`; no compatibility walker or generated manifest is needed.

For a release archive with one versioned top-level directory:

```ts
const linuxMusl = targetPlatform({ os: "linux", arch: "x86_64", abi: "musl" });
const rg = pkg("ripgrep", {
  producer: provider(githubRelease({
    repo: "BurntSushi/ripgrep",
    asset: "ripgrep-{version.bare}-x86_64-unknown-linux-musl.tar.gz",
  })),
  archiveRoot: { kind: "single_directory" },
  commands: { rg: "rg" },
  target: linuxMusl,
  layout: { kind: "relocatable" },
});
```

`archiveRoot` selects exactly one real payload directory for a native relocatable
package. Root-level files/metadata siblings, multiple roots and symlink roots
are refused, not silently stripped. Commands, `artifactFile` and `artifactTree`
selectors all use that same receipt-bound root. A reviewed `grip update` can
therefore change the resolved version/root without editing `"rg"`. Existing
selectors such as `"ripgrep-15.2.0-x86_64-unknown-linux-musl/rg"` remain **literal**;
remove that prefix explicitly when opting into the policy. Asset placeholders
do not interpolate command/artifact selectors. This declaration is not a claim
that the upstream binary has been exercised on your host.

### Managed profile launchers

Select an environment and explicitly choose both the destination and commands:

```ts
const search = environment("search", { packages: ["ripgrep"], target: linuxMusl });
const personal = profile("personal", {
  environment: "search",
  launchers: { directory: "~/.local/bin", commands: ["rg"] },
});
// Return rg, search and personal in workspace.outputs.
```

Only uniquely resolved exports from that environment become launchers; missing
or ambiguous commands fail before mutation. Use `/usr/local/bin` only as an
explicit operator choice with appropriate destination permissions, not as an
implicit privilege grant. Put the chosen directory on your editor/SSH `PATH`;
the core does not edit those programs' environments.

The core owns executable launcher bodies, activation bindings and retained
package roots. Launchers re-enter the absolute installed core path and re-admit
the package/runtime contract on each invocation; they do not find `grip` via
`PATH`, recursively call the public command name, or fall back to an unsealed
shell wrapper. Keep that core path and private state available. If the core
moves/disappears, restore it or reapply using the replacement core; do not hand
edit the launcher. Rollback/GC retain the generation's dependencies.

Sealed invocation does not promise the original `/proc/self/exe` pathname.
LuaLS3.19.2's compiled bootstrap uses that pathname to find `bin/main.lua`;
granting its libraries does not fix this self-location requirement. For that
release, retain the complete tree with `trackedCopyTo` (including `bin/main.lua`,
root `main.lua` and `changelog.md`), use one executable captured `templateFile`
wrapper to invoke the deployed binary, and check that explicit host path.
Keep runtime-written log/meta data outside the store. This is deliberate host
execution, not sealed closure proof or a managed-launcher fallback.

`artifactTree` expands selected paths/subtrees into individual owned file
entries, with exclusions winning. It never replaces the destination directory:
foreign children remain, removals use existing drift/prune policy, and expanded
collisions fail before destination writes. Symlinks and special source entries
are refused rather than followed. Per-file symlinks do not preserve the real
sibling layout of a release tree: use `trackedCopyTo` for binaries that locate
runtime files relative to their resolved executable path. This does not add a
directory-level symlink or fixed-prefix relocation mechanism.

Use tracked copies for extension-sensitive data consumers too. Per-file store
blobs do not promise the source filename or extension. A font tree deployed with
`trackedCopyTo("~/.local/share/fonts/inter")` preserves its relative filenames as
real files; point `TYPST_FONT_PATHS` at that destination instead of maintaining a
second dereferenced font-copy cache. Both artifact and captured repository trees
can use this policy. Tracked copies retain ownership and drift checks; they are
not a reproducible mutable package-manager overlay.

Profile hooks, task postconditions and attached profile-file checks execute.
Schedules and host recipe execution remain explicitly gated.
Historical v4 workspaces remain read-only pending A5 migration. `check` never
bootstraps BuildKit or a scheduler. `adopt` does not edit workspace TypeScript
automatically: declare the file policy and use explicit `apply --take-over`
when reversible adoption of a foreign tracked-copy destination is intended.

### File checks and manual tasks

Attach a named `check` to `file({ checks: [...] })`, for example
`{ check: "validate-editor", subject: "rendered", stage: "pre_flip" }`.
The corresponding `check("validate-editor", { subject: "dotfiles", run: ... })`
names the owning profile. Its typed command may use a declared `packageCommand`
or an explicitly selected host executable; the latter is host execution, not
sealed package-closure proof. `sourcePath(".")` refers to that attached file,
not a writable output or working directory.

Available stages are `pre_flip`, `post_link` and `post_activate`. Source checks
need a captured source; rendered checks must be `pre_flip` and validate before
destination mutation. A deployed `pre_flip` check runs after candidate writes
but before the generation flip, within journal compensation. `post_link` and
`post_activate` run through the activation lifecycle; a late failure is not a
promise of automatic rollback. A profile-bound check cannot also be a recipe
validator or task postcondition.

```sh
grip task list
grip task plan format-now
grip task run format-now --closed-stdin
grip task status INVOCATION_ID
grip task log INVOCATION_ID
```

Use the exact invocation ID returned by `run`; `status` reads its durable
receipt and `log` reads its bounded private stdout/stderr log. `list`/`plan`
inspect declarations without package realization or running postconditions.
`run` creates a fresh invocation, not a cached task success. Task `deps` are
unordered prerequisite task names; they must succeed within that invocation,
while `steps` are ordered locally and `checks` are invocation postconditions.
Declare live paths with `context: { kind: "host", mutable_paths: [...] }` and
shared mutation keys with `locks: [lock("user", "editor-config")]` (or `"store"`
scope). The complete graph's keys are admitted before effects, not inferred
from command text; locks grant no execution authority. `--skip-busy` permits
skipping an active conflict, not bypassing an unknown interrupted owner.
`--timeout-seconds` bounds realization, prerequisites and verification together.
Interrupted outcomes remain unknown, not fabricated success.

`schedule(...)` and `profile({ schedules: [...] })` are declarations only:
registration/execution is unavailable. Use explicit manual tasks; do not infer
a running timer from a successful `grip check`.

Four [graduated workspaces](../examples/workspaces/) use the real SDK
without a hostname shim or a synthetic package: [dotfiles only](../examples/workspaces/01-dotfiles/gripsack.ts),
[an offline-staged native tool and profile](../examples/workspaces/02-native-tool/gripsack.ts),
[a source recipe with alternate producer and typed command consumer](../examples/workspaces/03-source-built/gripsack.ts),
and [manual plus scheduled task declarations](../examples/workspaces/04-scheduled-task/gripsack.ts).
The `ts-test` gate strictly type-checks their source, and `e2e` admits
each through the real `grip check`. The dotfile and native-tool profiles use
the native lifecycle; the source-built tool uses the pinned BuildKit policy.
Scheduler registration remains gated: a declared schedule activates nothing.

The legacy `hosts/<name>.ts` path emits a v8 modules-compatibility
envelope. Strict retained readers preserve their versioned meanings;
current fields are not silently added to an older declaration.

`grip check --json` emits one versioned document on stdout:
`{version: 1, ok, diagnostics, host?, outputs?, modules?, layouts?}`.
Diagnostics use the same codes, labels, spans and help as terminal
output; failures exit nonzero. Readable snippets come only from files
under the evaluated repo's pinned directory capability, at most 1 MiB
per file. Out-of-repo labels remain visible without reading their
contents; JSON carries diagnostic facts, not source-file bytes.
Operational failures such as a denied trust gate still report stderr
and may have no structured diagnostic in the JSON document.

### Reviewer migration: delete plumbing, keep verification

| Before: user-owned maintenance | After: reviewed declaration |
|---|---|
| `scripts/generate-tree-manifest.mjs`, `lib/tree-manifest.generated.ts`, traversal helper and freshness checks | `file({ source: repoTree(...), content: identity(), destination: trackedCopyTo(...) })` |
| Captured script plus duplicated inline template body and parity checker | One executable `repoFile` plus `templateFile(variables)` |
| Handwritten activation/PATH wrappers | Profile `environment` plus explicit `launchers.directory` / `launchers.commands` |
| Versioned archive prefixes repeated in commands/artifact selectors | Opt-in `archiveRoot`, then root-relative literal selectors |
| Dereferenced font-copy cache solely to recover filename extensions | Deploy the font tree with `trackedCopyTo` and point the consumer at that physical destination |

1. Install matching 0.47.0 core/SDK (or remove a deliberate npm pin to use the
   matching embedded frontend). Retain the previous generation and recovery
   data; an SDK change is not itself a lock refresh or source approval.
2. Replace declarations as above. Keep captured template bodies and executable
   bits; keep physical sibling trees on `trackedCopyTo`. Remove only the
   obsolete generator/manifest/parity/wrapper implementations and their checks,
   retaining one small consumer integration test of actual installed behavior.
   Do not delete unrelated legacy ownership records or use `--take-over` as a
   blanket symlink migration tool.
3. Inspect `grip trust inspect --json`, review the source inventory/policy and
   use exact-digest approval. Run `grip update` when frontend/import pins or
   external resolutions change, review the lock diff, then inspect/approve the
   new snapshot when required. A frozen Pixi import still does not upgrade it.
4. Run `grip check`, `grip plan personal`, then `grip apply personal` for your
   selected profile. Verify the actual deployed commands, file modes, logical
   alias destinations and sibling resources before dropping recovery data.
   These are migration instructions, not a claim of reviewer-host qualification.

For a current legacy generation `N`, review `grip plan personal --migrate-from N`
before `grip apply personal --migrate-from N`. This explicit ownership transition
prepares replacements before releasing old paths and retains predecessor recovery
data. Keep that generation until functional verification succeeds. Destination
changes are not atomic across the whole profile: commands can be temporarily
unavailable during cutover. Stop dependent invocations until verification passes.
Pre-flip failures compensate; late activation failure needs explicit recovery
such as `grip rollback N`, not an assumed automatic rollback. The older prune-first
sequence remains a separate legacy apply with a longer gap between generations.

The [dotfiles example](../examples/workspaces/01-dotfiles/gripsack.ts) is a small
offline CLI smoke target: typed `editorFiles(theme)` returns ordinary file
values and an executable captured template prints the selected theme. Copy it to
a scratch repository preserving executable bits. To also exercise a safe internal
alias, create it in that scratch copy before capture:

```sh
mkdir -p dotfiles/editor/recipes
ln -s ../theme.toml dotfiles/editor/recipes/theme.toml
```

With isolated `HOME` and `GRIPSACK_HOME`, approve that copy, then run `grip check`,
`grip plan dotfiles`, `grip apply dotfiles` and
`"$HOME/.local/bin/editor-theme"`. The optional alias deploys as a physical file
alongside `theme.toml` under `$HOME/.config/editor/assets`; adding it needs no
manifest regeneration. The checkout itself remains compatible with the local
release gate's no-source-symlink policy. No upstream editor/formatter is run.

### Frequent content edits: select a profile, not a trust bypass

Keep frequently edited files in their own ordinary profile within the same
workspace, alongside tool and editor profiles:

```ts
const claude = profile("claude", {
  files: [file({
    source: repoTree(".claude/skills"),
    content: identity(),
    destination: trackedCopyTo("~/.claude/skills"),
  })],
});
// Include claude alongside the other outputs in workspace.outputs.
```

`grip plan claude` and `grip apply claude` select that profile; they do not apply
or prune unselected profiles. With no selection, apply processes the whole
declared profile graph. A legacy wrapper such as `install.sh claude` must forward
to that profile name rather than assuming old module names exist in the workspace.

In a terminal, `grip trust add` already captures the source, shows its digests,
policy and changed inventory paths, then asks for approval in one command.
It is not necessary to run a separate `trust inspect` first. The terminal summary
shows at most 20 paths; use `grip trust inspect --json` for the complete
`.approval.changes` list and policy. Review the actual changed content too:
inventory changes are not a line-by-line content diff.

Approval still covers the exact whole source bundle and policy. Changes outside
the selected profile remain visible and require review; a text file or
non-executable mode is not a trust exemption. Subtree approval and recursive
capture-glob exclusions are not 0.47 APIs. `[capture].exclude` remains literal,
and capture does not automatically inherit `.gitignore`.

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
| commands/files | `exec` (object or fluent), `bash`, `bashBody`, `runBash`, `file`, `lit`, `artifact`, `hostPath`, `packageCommand`, `repoFile`, `repoTree`, `artifactFile`, `artifactTree`, `identity`, `literalText`, `templateText`, `templateFile`, `symlinkTo`, `trackedCopyTo`, `managedBlock`, `daily`, `weekly` |
| legacy modules | `module`, `ModuleSpec`, `ModuleValue` (A5 migration inventory) |
| probes | `ctx.probe` (`executable`, `file_exists`), `ProbeBuilder`, `ProbeKind` |
| facts | `HostFacts` (core-injected), `when`, `hasTag`, `Condition` |
| captured inputs / pure locks | `inputFile`, `inputDirectory`, `input`, `lock`, `ensureArtifact` |
| fetchers | `githubRelease`, `tarball`, `git`, `fileFetch`, `pluginFetch`, `brew` |
| workspace environment sources | `conda.environment`, `pixi.fromLock` |
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

Historical core/TypeScript **0.36.0 used IR v3**. Module/step `retries` was
removed: the field was accepted but did not execute retries; it is rejected.
This is historical reader guidance, not a recommendation to downgrade a current
SDK pin. v1/v2 input is rejected before decoding. Dependency purposes still
use `for`, not `edge`, with `dep(name, { for: "build" })`.
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
