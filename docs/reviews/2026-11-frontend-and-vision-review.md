# Gripsack frontend and vision review — November 2026

- Date: 2026-11-09
- Source reviewed: `origin/main` at `e60ca37` (the 0.53.0 merge). Website
  source at `gripsack-dev.github.io/doc/*.md`.
- Method: read the SDK (`typescript/src`), the schema, the examples and
  golden corpus, plans 0001/0002/0013/0054/0055/0056/0058, the website
  docs, and the previous review. Then **installed `grip` 0.53.0 into a
  scratch directory with an isolated `HOME` and `GRIPSACK_HOME`** and wrote
  five small workspaces by hand: the `grip init` dotfile, a CLI tool from a
  GitHub release, a Conda environment, a guided `adopt`, and five
  deliberately wrong declarations to see the diagnostics. Every command
  count, timing and error message below is from those runs.
- Relation to the previous review (`docs/reviews/2026-11-design-review.md`):
  it covered the core, the transaction and the verification programme and
  touched the DSL in §1.2. This review does not repeat those findings; it
  builds on §1.2 with measurements and a concrete redesign, and it takes
  positions on features and vision that the previous review left as
  "freeze". Where it disagrees it says so.
- Stance: the owner asked for candid. Where I think the owner is wrong I say
  so (§3.2, §1.2 item 2).

Tags: **[tedium]** authoring cost · **[bug]** · **[cut]** · **[add]** ·
**[rework]** · **[strategy]**.

---

## Executive summary

The frontend is not cumbersome because it is TypeScript. It is cumbersome
because it is **a wire format with function names**: every IR field is a
required argument, every cross-reference is a string, every legal value of
a one-value enum must be spelled out, and the approval gate re-fires on
every byte the core itself writes. Measured on 0.53.0:

- The **documented hello-world for packages fails** on an ordinary glibc
  host (`packages.md:15` declares `abi: "musl"`; `grip run` refuses with
  E128 because the declared ABI is compared to the host's libc,
  `gripsack-exec/src/workspace/consumer/admit.rs:115-156`). The correct
  spelling for a static binary is *no* ABI, which no page says.
- The **documented hello-world for Conda fails** (`typescript/README.md:54`,
  `python: "3.12.*"`): `grip run` refuses with "Conda prefix requires
  libX11.so.6 outside the explicit OS runtime" because conda-forge's Python
  depends on Tk. The docs know (`environments.md:55`) and ask the user to
  add `xorg-libx11` instead of fixing the default.
- Adding one tool costs **two approvals**, because `gripsack.lock` is inside
  the approved bundle (`gripsack-store/src/source_bundle/policy.rs:11`) and
  `grip update` rewrites it. Editing the README costs one more.
- Running `printf` from a task requires **a Conda solve for Bash**
  (`tasks.md:42-76`, `commands.ts:221`): ambient `/bin/sh` is refused by
  design while the task itself runs unsandboxed on the live checkout.
- A realistic personal workspace carries 13–17 named imports and ~20
  concepts before the first useful line (`examples/workspaces/*`).

The discipline that is earning its keep: per-file ownership modes, the
plan, generations, the sandboxed evaluator, hash pinning in the lock,
provenance spans, digest-pinned images. The discipline that is not:
`target`/`layout`/`archiveRoot` on every package, `producer: provider(x)`,
`defineWorkspace`, `lit()`/`.build()`, string references, a pinned
interpreter for every shell line, the Conda closure refusal, whole-repo
approval including the lock, `probe` two-stage evaluation, and `check`/
`hook`/`schedule`/`environment` as separately named outputs.

On vision: gripsack is **not** a Nix/devenv successor and should stop
saying so to itself. It has no package set, no builds (recipes were removed
in 0.51), one platform, and a solo maintainer. What it has that nothing
else has is a *transactional home directory*: ownership modes, drift
preservation, plan, generations, rollback, now with proofs. The pitch I
would pick: **"chezmoi with undo, and mise inside it."** §3.

Top five actions (the full ranked list is §4):

1. Handles instead of strings, inference instead of required fields, and a
   map-form file API — one IR bump, ~60% fewer lines per workspace.
2. Approve the *evaluation closure*, not the repository; take the lockfile
   out of the approved bundle.
3. Make the host shell the default task interpreter; pinning stays an option.
4. Fix the ABI/closure model so the two documented hello-worlds run.
5. Cut: `grip package` (sharing), Pixi import, wheels, gripfetch plugins,
   `griplint` (a dead crate), `snapshot`/`diff-config`/saved plans, probes.

---

## 1. Frontend syntax and ergonomics

### 1.1 What a minimal workspace costs today

Measured with `grip` 0.53.0, isolated `HOME`, first run (Deno provisioning
downloaded 98 MB into `$GRIPSACK_HOME/tools/deno-2.9.6`).

| Use case | TS lines / chars / imports | Commands to first effect | Approvals | Outcome |
|---|---|---|---|---|
| One dotfile (`grip init`) | 15 / 413 / 7 | `init`, `check`, `trust add --bundle … --policy …`, `check`, `plan`, `apply` = 6 | 1 | works; 0.3 s per command once approved |
| CLI tool from a release archive (`packages.md` example) | 19 / ~700 / 7 | `check`, `trust add`, `update`, `check` (refused: lock changed), `trust add`, `run` | 2 | **E128**: `environment targets linux-x86_64 with ABI Some(Musl) …; host … has ABI Some(Gnu)` |
| Same, with `abi: "gnu"` on a *musl* binary | 19 | as above | 2 | works (the binary is static; the declared ABI is a lie the checker likes) |
| Conda env `python 3.12 + ripgrep` (`typescript/README.md:49-63`) | 20 / ~800 / 7 | `check`, `trust add`, `update` (3.3 s, 642-line lock), `trust add`, `run` | 2 | **E128**: `Conda prefix requires "libX11.so.6" outside the explicit OS runtime` |
| Dev shell for a project | — | there is no project mode: `grip shell dev` needs a `pkg` + `environment` first; no direnv hook, no `.envrc`, no `grip hook bash` (grep: zero hits in plans and docs) | — | not a supported flow |
| Guided `adopt ~/.config/foo` | generated: 8 lines | `trust add`, `adopt` (exits 1 by design), `trust add`, `adopt --resume` | 2 | works; rollback restores originals |

Three things stand out that the previous review could not see because it
did not run the binary:

1. **Both package hello-worlds are broken for the common case.** A new user
   following the website in order hits E128 twice, each time with a
   diagnostic that points at the right line and gives the wrong idea (it
   says the target is incompatible; the target is fine, the *declaration
   style* is wrong).
2. **Every lock change is a source change.** `REQUIRED_ROOTS` in
   `source_bundle/policy.rs:11` forces `gripsack.lock` into the bundle, so
   the core writing the lock invalidates the approval of the thing that
   produced the lock. Two approvals per tool is the floor; the README
   example's `grip update` step silently implies a third approval nobody
   mentions.
3. **The generated code from `adopt` is the best gripsack.ts in the
   repository** (`crates/gripsack/src/commands/adopt/generate.rs:72`):
   one line per file, no `defineWorkspace`, `export default profile(...)`.
   The tool already knows the shorter form; the SDK makes users write the
   long one.

A fourth observation: with the file deployed as a tracked copy and then
edited by hand, `grip plan foo` printed `= ~/.config/foo/foo.toml (satisfied)`
and `apply` said `unchanged` (`render.rs:474`). The edit *was* preserved
(correct), but drift is only reported when the declaration also changes
(`render.rs:488`). There is no way to ask "what has drifted?" — the
previous review's `grip status` ask, now with evidence.

### 1.2 Named sources of tedium

**1. Required fields with exactly one legal value.** [tedium]

- `target: targetPlatform({ os: "linux", arch: "x86_64" })` on every `pkg`
  and `environment` (`outputs.ts:179-232`), when `target.ts:31-34` rejects
  anything but `linux`/`x86_64`. The test suite itself passes a plain object
  (`ergonomics.test.ts:6`), so `targetPlatform()` is validation theatre.
- `layout: { kind: "relocatable" | "prefix_materialized" | "fixed_prefix" }`
  (`target.ts:56`): fully determined by the source kind in every example in
  the repo (archive → relocatable, Conda/Pixi → prefix_materialized).
  `fixed_prefix` has no example anywhere.
- `producer: provider(source)` wraps a one-variant union
  (`ir.ts:200`, `outputs.ts:150`). The previous review said this; it is
  still there.
- `content: identity()` is required whenever `source` is given
  (`files.ts:132`), which is the overwhelming case. The one case where
  content is not identity (`templateFile`) is the one that should carry
  the extra word.
- `defineWorkspace(fn)` is `return fn` (`outputs.ts:469`).
- `exec(x).arg(lit("docs")).build()` (`commands.ts:108-130`): `lit` around
  every literal because the argv union has no string member
  (`ir.ts:52`), and `.build()` because the builder is distinct from the
  value.
- `schedule(..., { scope })` is always `"user"` (`outputs.ts:304`), and the
  doc comment still says registration "is rejected … until E3"
  (`outputs.ts:289-291`) in a release where it works.

**2. Strings for references — and no compile-time check to catch them.**
[tedium][strategy]

`environment: "search"`, `packages: ["tools"]`, `schedules: ["nightly"]`,
`task: "format"`, `packageCommand("shell", "bash")`, `subject: "dotfiles"`.
Passing the value is refused at runtime: `environment("dev"): packages[0]
must be a non-empty string` (my run, variant 4). A typo is E126 at
`grip check` time, not an editor squiggle.

The owner's framing is "disciplined but tedious". I disagree with the
first half: **the frontend is not typed where it matters.** The evaluator
runs `deno run` with no type check (`frontend.rs:132-145`; Deno does not
type-check on `run` by default and `--no-config` is passed), so every
`PackageSpec` field is checked by hand in `validate.ts` (608 lines) +
`outputs.ts` and again in Rust `sema` (8.8k lines in `gripsack-ir`). Editor
types require installing `@gripsack/core` into `node_modules`, which
**changes the approved frontend** unless the user also writes a capture
exclusion in `env.toml` (`settings.md:11-19`). The newcomer who most needs
the types is the one who does not have them. Missing `target` produced
`E130: pkg("tool"): target must be an object` at runtime (variant 2) — a
message that a TypeScript compiler would have given for free, with the
field's type, before any approval dance.

**3. The approval loop.** [tedium]

- Unit of approval: the whole repository minus `.git` and explicit
  exclusions, plus the lockfile and `env.toml` (`policy.rs:11`,
  `trust inspect` listed 42 inventory objects for a 7-file repo, 20 of
  them the embedded frontend's own sources).
- Trigger: any byte. A README edit re-prompted; reverting it un-prompted.
- Non-interactive approval needs two 64-hex digests copied from the
  previous error (`trust.rs`; "path-only approval is no longer supported").
  `--yes` on `adopt` explicitly does not grant trust (`0013 D7`).
- `grip update` writes the lock → bundle digest changes → re-approve before
  `run`. The website's `grip update` / `grip run` sequence (`packages.md:32-34`)
  is therefore three commands short.

The previous review recommended approving the evaluation closure instead.
I agree and add two things: (a) the lockfile is *core output*, not user
input, and belongs outside the bundle — the core already validates it on
read; (b) when the closure changes, show the TypeScript diff since the last
approval, not a 42-line inventory.

**4. The ABI/target model is user-visible and wrong-footed.** [bug][tedium]

`admit_target` (`admit.rs:115-156`) requires a declared ABI to equal the
host libc, and admits an *undeclared* ABI only for executables "the
classifier proves carry no dynamic ABI dependency". So the right declaration
for a static musl ripgrep is to say nothing about ABI, and the docs' example
says `musl`. The ABI is a property of the artifact's bytes (which the
classifier already reads), not of the environment. Users should never type
it; the lock key (`linux-x86_64-musl` vs `-gnu` in my two runs) should be
derived.

**5. The Conda closure refusal.** [bug][tedium]

`python: "3.12.*"` from conda-forge pulls Tk, which links libX11.
Gripsack refuses to run *anything* in the prefix — including `python
--version` — until the declaration adds `xorg-libx11`. Every other Conda
consumer (conda, mamba, pixi, rattler) runs this environment on a stock
desktop; the X11 client libraries are treated as platform libraries
(conda-forge's own "CDT" policy). The intent — "never silently depend on
ambient libraries" — is right for a *shared* package and wrong as a default
for a *personal* one. Proposed: the platform library allow-list (libX11,
libGL, libasound, libxcb, fontconfig… — the CDT set) is the default
`hostRuntime`, reported in `plan` as "uses host libX11.so.6"; `strict:
true` keeps today's behaviour for publishers.

**6. The pinned-interpreter rule for shell.** [tedium]

`runBash` and `` bash`…` `` refuse any interpreter that is not a
`packageCommand` (`commands.ts:221`, `:255`), "ambient host shells are never
discovered (A1-03)". The documented consequence (`tasks.md:42-76`) is a
Conda solve of `bash` to print a string — 20 lines, a network solve and
two approvals for `printf`. Meanwhile `exec` happily accepts
`lit("/usr/bin/printf")` (`services.md:39`), tasks run on the live checkout
as cwd ("not a hermetic sandbox", `consumer/mod.rs:6`), and schedules
capture the *runner*, not the scripts it runs. The rule protects a
hermeticity the task does not have. Keep pinning as an *option* for people
who want it; make `/bin/sh` the default and print "host /bin/sh" in the
plan.

**7. Naming and imports.** [tedium]

- Three ways to name a file in the repository: `repoFile(path)` (a profile
  file origin), `inputFile(name, path)` (a producer input), `fileFetch(path)`
  (a package source) — `files.ts:32`, `bindings.ts:6`, `fetch.ts:48`. One
  concept, three words, three different shapes.
- `lit` vs `literalText`; `identity`; `trackedCopyTo`/`symlinkTo`/
  `managedBlock` (verb, verb, noun); `pkg` because `package` is reserved,
  but `environment`, `profile`, `schedule` spelled out; `conda.environment`
  vs `environment`; `lock` (mutation lock) vs `gripsack.lock` vs
  `pixi.fromLock` vs `locks/`.
- Casing: `hostRuntime`/`archiveRoot` (camel) on `pkg`, `depends_on`/
  `interval_seconds`/`base_url` (snake) on `service` and `githubRelease`,
  `mutable_paths` on `TaskContext`, `file_exists` on `probe`.
- 112 named exports from `index.ts`; the examples import 13–17 each.
  Nothing is reachable without an import, so every new idea is a new line
  at the top of the file.

**8. Host variation.** [tedium]

Four mechanisms — `when(cond, ctx)`, `hasTag(tag, ctx)`, `ctx.probe.*`,
`ctx.settings` — for one need. `when` takes the context as a second
argument because the helpers are "pure" (`conditions.ts:1-5`) — a
`ctx.host.is({ os, tags })` method would be equally pure and half the
typing. Probes need two evaluation rounds and return `false` on the first
(`probe.ts:1-8`); the docs warn against their main use ("never a tool
installed by this workspace", `probe.ts:21-24`). The number of probe uses in
the repository's own examples: zero.

**9. The three-axis file.** [tedium]

The origin/content/destination model is the best idea in the DSL and the
previous review was right to say so. But a `file({ source, content,
destination })` literal is three keys and two constructors for the
dominant case "this repo file goes there as a copy". The destination path
is the natural *key* (it is unique within a profile; the emitter already
rejects duplicates); the ownership mode is the natural *constructor*.

### 1.3 Which discipline is earning its keep

| Keep | Why |
|---|---|
| Ownership mode per file (`symlink`/`copy`/`block`) | The product. Nobody else has it. |
| Plan before apply, generations, rollback | The product. |
| Sandboxed evaluation, facts injected | Right boundary; cheap for the author once approval is fixed. |
| Lockfile pinning, `grip update` as the only mover | Right; the lock just must not be in the approval unit. |
| Provenance spans on every node | Right; capture is already automatic (`span.ts:9`) — remove `span?` from public specs so nobody can forge one. |
| Digest-pinned images, hash-verified archives | Right. |
| Deep-frozen values, falsy filter in `outputs` | Right; cheap. |
| Dedented `bash` tag with a line map (`commands.ts:142-166`) | Genuinely nice; keep. |

| Drop or default | Why |
|---|---|
| `target`, `layout`, `archiveRoot`, `abi` as authored fields | Derivable from host facts + the artifact. |
| `producer: provider()`, `defineWorkspace`, `.build()`, `lit()`, `identity()` | No information. |
| String references | Replace with handles; names stay in the IR. |
| Pinned interpreter as the only interpreter | Protects nothing the task has. |
| Conda closure refusal by default | Blocks hello-world. |
| Whole-repo approval incl. lock | Wrong unit. |
| `check`, `hook`, `schedule`, `environment` as named catalog outputs | They are attributes of a file, a profile, a task and a package list. |
| Probes | Two-stage eval for a feature the docs discourage. |

### 1.4 A redesign, with code

Design rules: a value is referenced by its handle, never its name; defaults
come from the source and the host; the destination path is the key; strings
are literals and objects are references (today it is the reverse); the
export is the workspace. The IR can stay IR10-shaped (names are still
emitted; `target`/`layout` become optional with core-side inference) or
become IR11 with the stability promise the previous review asked for.

**One dotfile — today (`crates/gripsack/template/env-repo/gripsack.ts`, 15 lines):**

```ts
import { defineWorkspace, workspace, profile, file, repoFile, identity, symlinkTo } from "@gripsack/core";

export default defineWorkspace(() => workspace({
  outputs: [
    profile("hello", {
      files: [
        file({
          source: repoFile("configs/hello/hello.toml"),
          content: identity(),
          destination: symlinkTo("~/.config/hello/hello.toml"),
        }),
      ],
    }),
  ],
}));
```

**Proposed (4 lines):**

```ts
import { profile, link } from "@gripsack/core";

export default profile("hello", {
  "~/.config/hello/hello.toml": link("configs/hello/hello.toml"),
});
```

Rules: the default export may be one output, an array of outputs, or
`workspace({...})` when `inputs`/`name` are needed. A profile's second
argument is a map from destination to a file constructor; `link`, `copy`,
`block` are the three ownership modes (`symlinkTo`/`trackedCopyTo`/
`managedBlock` keep their meaning). `copy` is the safe default the `adopt`
prompt already recommends.

**Dotfiles example (`examples/workspaces/01-dotfiles/gripsack.ts`, 43 lines → 14):**

```ts
import { profile, copy, block, tree } from "@gripsack/core";

const theme = "dark";

export default profile("dotfiles", {
  "~/.config/editor/config.toml": copy("dotfiles/editor.toml.tmpl", { vars: { theme } }),
  "~/.config/editor/assets": tree("dotfiles/editor", {
    include: ["theme.toml", "recipes"], exclude: ["recipes/drafts"],
  }),
  "~/.local/bin/editor-theme": copy("dotfiles/editor-theme.sh.tmpl", { vars: { theme } }),
  "~/.profile": block("gripsack-editor", "export EDITOR=editor\n"),
});
```

A `.tmpl` suffix or a `vars` option selects template rendering (today's
`templateFile`); everything else is identity. `tree(...)` is `repoTree` with
`copy` semantics; `tree(..., { mode: "link" })` for a symlink farm.

**CLI tool from a release (`packages.md`, 19 lines → 7, and it runs):**

```ts
import { pkg, github, env, profile } from "@gripsack/core";

const rg = pkg("ripgrep", github("BurntSushi/ripgrep",
  "ripgrep-{version.bare}-x86_64-unknown-linux-musl.tar.gz"), { bin: { rg: "rg" } });

export default [rg, env("search", [rg]), profile("personal", { bin: [rg] })];
```

`target` is the host; `abi` is read from the ELF (the classifier in
`admit.rs:104-112` already knows how); `layout` is `relocatable` for an
archive; a single top-level directory in the archive is the root unless
`root: "."` says otherwise — and the plan prints what was inferred
(`root: ripgrep-15.2.0/`, `abi: static`). `profile.bin` deploys launchers
to `~/.local/bin` by default (`launchers.directory` becomes an option).

**Conda environment (`typescript/README.md:49-63`, 20 lines → 6, and it runs):**

```ts
import { pkg, conda, env } from "@gripsack/core";

const tools = pkg("tools", conda({ python: "3.12.*", ripgrep: "*" }),
  { bin: { python: "bin/python", rg: "bin/rg" } });

export default [tools, env("dev", [tools])];
```

`channels` default to `["conda-forge"]`; `systemRequirements` default to
the host's measured baseline (already the documented fallback,
`environments.md:61`); platform libraries are host-provided unless
`strict: true`. `bin` can be omitted entirely: a Conda prefix's `bin/` is
enumerable at update time and recorded in the lock.

**Task + schedule (`examples/workspaces/04-scheduled-task`, 27 lines → 9):**

```ts
import { pkg, file, env, task, sh, daily, profile } from "@gripsack/core";

const formatter = pkg("formatter", file("downloads/formatter.tar.gz"), { bin: { format: "bin/formatter" } });
const dev = env("dev", [formatter]);
const format = task("format", sh`format docs`, { env: dev, lock: "format-documents" });

export default profile("personal", { schedules: [daily(format, "03:30")] });
```

`sh` runs under host `/bin/sh` with `env`'s bins on `PATH`; the plan says
`interpreter: host /bin/sh`. `sh.with(bash)` where `bash` is a package
handle gives today's pinned behaviour. `task("x", [sh`a`, sh`b`], {...})`
for multiple steps; `deps: [other]` and `verify: sh`test -f out`` replace
named `check` outputs. `schedule` is not an output; it is a profile entry.

**Service (`services.md`, 23 lines → 9):**

```ts
import { service, fromEnv, task, sh } from "@gripsack/core";

const pg = service("postgres", "docker.io/library/postgres@sha256:979bcc…be0e4", {
  ports: { 55432: 5432 },
  volumes: { "guide-postgres18": "/var/lib/postgresql" },
  env: { POSTGRES_USER: "app", POSTGRES_DB: "app", POSTGRES_PASSWORD: fromEnv("PG_PASSWORD") },
  health: ["pg_isready", "-U", "app", "-d", "app"],
});
export default [pg, task("db-ready", sh`echo ok`, { services: [pg] })];
```

(Whether services belong in the binary at all is §2.)

**Host variation:**

```ts
export default ({ host }) => [
  base,
  host.tags.has("work") && workProfile,
  host.hostname === "pi" && piProfile,
];
```

A function export receives the context; `when(cond, ctx)` and `hasTag`
become methods; probes go.

**What changes under the hood.** Constructors return branded handles that
carry `name` and `span` (they already do: `WorkspaceOutput<N>`,
`outputs.ts:172`). The emitter resolves handles to names at
`workspace()`/export time; the IR keeps `environment: "dev"`. Inference
(`target`, `layout`, `archiveRoot`, `abi`) is a core-side pass at `update`
time that records what it chose in the lock, so a lock is still a complete
answer and frozen consumers never re-infer. The `span?` field leaves every
public Spec; `callerSpan()` stays. The TypeScript validator shrinks to
shape + handle checks; Rust remains the semantic authority and already
returns spanned diagnostics (`E126` in my run pointed at the exact column).

Estimated size: SDK rewrite ~1.5k lines (it is 3.9k today), one `gripsack-ir`
inference module, docs regenerated from runnable examples. The 0.51 cutover
already proved the team can bump IR and migrate fixtures in one release.

---

## 2. Feature fit

The question for each area: does it strengthen the thing only gripsack
does (transactional home directory), is it something a competitor does
better, and what is its maintenance tail for one person.

| Area | Verdict | One-line reason |
|---|---|---|
| Packages from archives (`githubRelease`, `tarball`, `git`, `file`) | **keep, rework** | the "mise inside it"; infer target/abi/layout/root; fix the docs |
| Conda | **keep, relax** | the only borrowed ecosystem and the reason gripsack is more than mise; default host platform libs; consider a provisioned `rattler`/`pixi` binary over 28k vendored lines |
| Pixi lock import | **cut** | stores the solve twice; users with `pixi.lock` have `pixi` |
| Hash-pinned wheels | **cut** | a PyPI story without a resolver; `uv` exists; niche inside a niche |
| Environments | **merge** | an `env` is a package list + vars; drop `target`/`prefix`/`fixed_prefix` |
| Profiles / dotfiles / launchers | **keep, rework API** | the wedge; map form; `bin:`; `grip status` |
| Docker services | **rework or cut** | compose does it better; outside generations; a second product |
| Tasks | **keep, rework** | host shell default; drop `context`/`mutable_paths`; `verify:` inline |
| Checks (named outputs) | **merge** | an attribute of a file or a task, not a catalog node |
| Hooks | **merge** | `profile({ onActivate: sh`…` })`; keep `post_activate`, drop `post_link`/`on_remove` until asked for |
| Schedules | **keep, simplify** | natural for a home manager (home-manager has timers); make it a profile attribute; shrink the 177-line README of caveats |
| Signed bundles / sharing (`grip package`) | **cut** | a second product (OCI client, Ed25519, trust.json, namespaces, `sharing-reference.md` 294 lines) for an audience that has `git clone && grip apply` |
| Semantic review (`snapshot`, `diff`, `diff-config`, `--saved-plan`) | **cut most** | keep `plan` and `diff <gen> <gen>`; `plan --json` today returns a "redacted/unknown" review document for a dotfile (my run) |
| Adopt | **keep, promote** | the on-ramp; generate the short form; one invocation |
| Doctor | **keep, grow into status** | agrees with the previous review |
| gripfetch plugins | **cut** | undocumented on the site (grep: zero hits), broken pinning (previous review finding 6), no second plugin exists |
| griplint | **cut (or wire)** | a dead crate: in `Cargo.toml:29` as a workspace member, depended on by **no** crate (`grep -l griplint crates/*/Cargo.toml` → only itself); 1.8k lines + 20 packs bumped every release |
| `init`, `gc`, `generations`, `rollback`, `why-owns`, `store-verify`, `self-update` | **keep** | the loop |
| Probes / `when` / `hasTag` / `settings` | **merge** | `ctx.host`; drop probes |

Notes on the contested rows.

**Conda.** The previous review said freeze. I say keep and invest, because
it is the answer to "where do the 10k packages come from" — the question
that kills every mise-shaped tool. But the maintenance shape is wrong:
10.7k core + 3.5k helper + 27.9k vendored (previous review, App. A) to do
what a pinned `rattler`/`pixi` executable does. Gripsack already provisions
Deno by compiled-in hash (`host.rs`); provision a solver the same way and
delete the forks. And relax the closure rule (§1.2 item 5) or the headline
example keeps failing.

**Services.** Three facts: they are outside generations (previous review
§2.6), they are a Docker Engine client that compose already is, and the
pitch table (`README.md:33-43`) lists them as one of seven equal outputs,
which makes the product read as "three products". Two honest options: (a)
cut, and tell users to keep `docker-compose.yml` next to `gripsack.ts`; (b)
rework into *review*: `service()` emits a compose file as a profile file
(digest-pinned, loopback-only, owned, diffed in `plan`), and `grip up` is
`docker compose up` on that file. (b) is ~200 lines, keeps the one thing
gripsack adds (the reviewed, pinned declaration), and deletes
`services/engine.rs` and its state. I would do (b).

**Sharing.** The previous review said freeze; I say cut. The use case
("move a package to a machine without network") is real but tiny, and the
implementation is the largest single non-core surface: a hand-rolled OCI
client, Ed25519 trust with namespaces and revocation, a 294-line format
reference, a JSON specification file users hand-write
(`sharing.md:14-48`). Every other machine the user owns can `git clone` the
workspace and `grip apply`. If air-gap matters later, `grip store export
<pkg> > x.tar` / `grip store import x.tar` with the existing content hash
is 1/20th of this.

**Checks/hooks/schedules/environment as catalog outputs.** The catalog
model (every node named, every edge a string) is what makes `workspace({
outputs: [...] })` long: a three-file profile with a syntax check and a
nightly task is seven named outputs. Only `pkg`, `profile`, `task` and
(if kept) `service` need names users will type on the command line.
Everything else is an attribute.

**What is missing that users of this kind of tool expect.** [add]

1. `grip status`: deployed / drifted / foreign / pending / leaked, per
   destination. Today drift is invisible until the declaration changes.
2. Shell integration: `grip hook bash|fish|zsh` to put the launcher dir on
   `PATH` and, for project workspaces, a direnv-style `grip env` so `cd`
   into a repo activates its `env`. The docs say "Add the chosen directory
   to your shell … yourself" (`environments.md:101`). mise, devenv, pixi and
   devbox all do this; it is the daily-use surface.
3. `grip add github:BurntSushi/ripgrep`: inspect the release assets, pick
   the Linux x86_64 one, write the `pkg(...)` line. Finding the asset
   pattern is the hard part of every `pkg` today (and the docs example got
   it wrong).
4. A catalog. mise has a registry, Nix has nixpkgs, Homebrew has formulae.
   Gripsack has nothing between "write the asset glob" and "Conda". A
   100-entry TOML of `name → github repo + asset pattern + bin`
   (ripgrep, fd, bat, fzf, helix, zellij, …) shipped in the binary or as a
   repo would change the first-hour experience more than any proof.
5. Host-conditional dotfiles with templating: chezmoi's number-one use
   case (`{{ if eq .chezmoi.hostname "work" }}`). `templateFile` takes only
   `Record<string, string>` (`files.ts:93`); the facts are not in scope.
   `copy(path, { vars: { hostname: host.hostname } })` plus the function
   export covers it.
6. Secrets: plan 0001 §7 promised age/sops at activation; nothing landed.
   `fromEnv` for services is the only secret story.
7. `grip remove <profile>` / unadopt (previous review), `gc --keep N`.
8. aarch64 Linux. The people who manage dotfiles across machines have a
   Raspberry Pi, a Graviton box or an Asahi laptop. It is the cheapest
   platform you could add (same kernel, same ELF logic, musl build exists)
   and the one the `{arch}` placeholders already anticipate
   (`fetch.ts:11-19`).
9. macOS. Named here only to be honest: the dotfile-manager audience is
   majority Mac. The owner cut it in 0055 for good reasons (Homebrew
   bottles, launchd, BuildKit were a swamp). Without it the "devenv
   successor" vision is dead on arrival (§3); with Linux-only the vision
   must be the one in §3.3.

---

## 3. Vision

### 3.1 Does gripsack deliver a Nix/devenv successor?

No, and the gap is structural, not a matter of time.

| What devenv/Nix users get | Gripsack today |
|---|---|
| 100k+ packages, one name each | archives by asset glob, or a Conda solve |
| Build anything from source, reproducibly | recipes/BuildKit removed in 0.51 (`0055:10-14`); `git` fetch is a tree, not a build |
| Per-project `devenv.nix` / flake with hermetic shell, auto-activated | `grip shell dev` after declaring `pkg`+`env`; no activation hook |
| Languages (`languages.python.enable`), processes, services, scripts in one file | tasks + Docker services; no language modules |
| macOS + Linux, x86_64 + aarch64 | Linux x86_64 |
| Community, modules, caches | one maintainer |

Gripsack has four things Nix/devenv do not: per-file ownership modes with
drift preservation (home-manager's files are read-only links), a plan a
human can read before applying, a one-binary install with no language to
learn, and generations for *dotfiles* (home-manager has them only for the
Nix-built part). Those are real, and they are not a devenv successor; they
are a chezmoi successor with a tool store.

### 3.2 Should it be the vision?

No. Three reasons, in order of weight.

1. **Ecosystem is the whole game for a devenv-class tool, and a solo
   project cannot build one.** Borrowing Conda is the right move and the
   only one available; it covers Python, R, compilers, CLI tools — most of
   what people actually want — but it will never be "a Nix successor", it
   will be "Conda with generations".
2. **The project's demonstrated strength is somewhere else.** 0.52–0.53
   proved ownership, rollback and recovery in Lean. That is unique in the
   dotfiles space and irrelevant to the devenv space (nobody chooses devenv
   for crash safety of `~/.config`).
3. **The devenv audience is on macOS.** The owner has chosen Linux/WSL,
   which is exactly the right choice for *servers, WSL boxes and dev VMs* —
   the machines where people today run `stow` and a shell script.

I also disagree with the owner's implied reading that the frontend's
tedium is the price of discipline. The parts that are tedious are mostly
*not* the parts that are disciplined (§1.3). The one place discipline and
tedium coincide — approval — has a cheaper correct unit.

### 3.3 Honest comparison

| | gripsack | Nix + home-manager | devenv | mise | chezmoi | Homebrew bundle | pixi | devcontainers |
|---|---|---|---|---|---|---|---|---|
| Dotfiles with ownership modes + drift kept | **yes** | read-only links | no | no | templates + `modify_` scripts | no | no | no |
| Generations + rollback of home files | **yes** | Nix-built only | no | no | no | no | no | rebuild |
| Plan before apply | **yes** | dry-run | no | no | `diff` | no | no | no |
| Tools from arbitrary archives, pinned | yes | via derivations | via nixpkgs | **yes** (backends + registry) | no | formulae | conda | Dockerfile |
| Package ecosystem | Conda (borrowed) | nixpkgs | nixpkgs | registry + GitHub | — | formulae/casks | conda-forge + PyPI | apt + anything |
| Per-project shell, auto-activation | no | `nix develop` + direnv | **yes** | **yes** | — | — | **yes** | container |
| Services | Docker, outside generations | NixOS/HM | **yes** (processes) | no | — | `brew services` | no | compose |
| Config language | TypeScript | Nix | Nix | TOML | templates | Ruby DSL | TOML | JSON |
| Platforms | Linux x86_64 | all | Linux/macOS | all | all | macOS/Linux | all | all |
| Crash-safe apply, proved | **yes** | no | no | no | no | no | no | no |
| Maintainers | 1 | thousands | team | team | 1 + contributors | team | team | Microsoft |

Reading the table: gripsack's column is unique in the first three rows and
weak in the two rows that make a *development environment* tool
(per-project activation, ecosystem). Those are the rows mise, devenv and
pixi are built around. Competing there is competing on their turf with a
one-person tail. The first three rows have no competitor at all.

### 3.4 The vision I would pick

**Pitch (one sentence):** *chezmoi with undo, and mise inside it — declare
your dotfiles and the tools they configure in one typed file, read the
diff, apply as a transaction, roll back in one command.*

**Who it is for:** a developer who keeps 2–10 Linux machines in sync — a
laptop or WSL, a few servers or VMs, maybe a devcontainer — wants the same
shell, editor and CLI tools everywhere, has been burned by a dotfile script
clobbering an edit, and will not learn Nix. Secondary: a platform team
putting a blessed toolset and configs onto engineers' WSL/RHEL machines
(the 0054 reviewer on RHEL 8.10 is that user exactly), where "plan,
approve, roll back" is a compliance feature.

**What the product is, in order:** (1) dotfiles with ownership modes and
rollback; (2) user-space tools from releases and Conda, pinned, with
launchers on the path; (3) tasks and timers that use those tools; (4)
nothing else in the binary. Services via a generated compose file or not at
all. No OCI pipeline, no publisher PKI.

**What would change on the website:** the README's "What you declare"
table (`README.md:33-43`) shrinks from seven rows to three. `compare.md`
stops leading with Nix. The guarantees page stays exactly as it is — it is
the best page on the site and it is about precisely the thing this vision
sells.

**A second angle worth one paragraph:** gripsack is also unusually well
shaped for *coding agents* configuring machines — sandboxed evaluation,
approval of exact bytes, a plan the human reads, rollback when the agent
was wrong. The two shipped skills (`skills/gripsack-adopt`,
`skills/gripsack-debug`) already point there. I would not make it the
pitch, but I would make sure every CLI command has `--json` and that
`adopt` can run without a TTY, because that audience is real and growing.

---

## 4. Ranked recommendations

Effort: S = days, M = 1–3 weeks, L = a month+. Impact is on the goal in
§3.4. ★ = do first (these five fit in one or two releases and change the
first-hour experience more than anything else on the list).

| # | Recommendation | Label | Impact | Effort |
|---|---|---|---|---|
| ★1 | **Handles, inference, map-form files.** `pkg(name, source, {bin})`, `env(name, [pkgs])`, `profile(name, { "~/path": copy\|link\|block\|tree })`, handles everywhere a name is referenced, `target`/`abi`/`layout`/`archiveRoot` inferred at `update` and recorded in the lock, `defineWorkspace`/`provider`/`lit`/`.build()`/`identity` gone, one casing. Ship as IR11 with a written compatibility promise (SDK major = IR major; one year of read compatibility). | rework | very high | M |
| ★2 | **Approve the evaluation closure.** Bundle = `gripsack.ts` import graph + files the declarations name + SDK identity. `gripsack.lock` and `env.toml` validated on read, not approved. `grip check` prompts with the TS diff since the last approval; `trust add --yes` for CI after `inspect` in the same invocation. | rework | very high | M |
| ★3 | **Host shell by default.** `sh`…`` runs under `/bin/sh` with the env's bins on `PATH`; plan prints the interpreter; `sh.with(pkg)` pins. Delete the Conda-bash prerequisite from `tasks.md`. | rework | high | S |
| ★4 | **Make the two hello-worlds run.** ABI read from the ELF, never authored; static binaries admitted on any host; Conda platform libraries (CDT set) host-provided by default with a plan note, `strict: true` for publishers. Fix `packages.md:15` and `typescript/README.md:54`. Add an e2e that runs every website example on a stock glibc host. | bug/rework | high | S–M |
| ★5 | **Cut the second products.** `grip package` + `sharing-reference.md` + trust.json; Pixi import; wheels; `pluginFetch` and the `gripfetch` host; `griplint` (dead crate); `snapshot`, `diff-config`, `--saved-plan`; probes. Git keeps them. | cut | high (focus, build time, surface to document) | S each, M total |
| 6 | **`grip status`** and drift in `plan`: deployed / drifted (preserved) / foreign / pending-recovery per destination; `grip resolve <path> keep\|adopt\|restore`. | add | high | M |
| 7 | **Shell integration.** `grip hook <shell>` (PATH for launchers), `grip env <name>` for direnv-style project activation, default launcher dir `~/.local/bin`. | add | high (daily use) | S–M |
| 8 | **Services → compose.** `service()` emits a digest-pinned, loopback-only `compose.yaml` as a profile file; `grip up/down` wrap `docker compose`; delete the Engine client and service state. Or cut outright. | rework/cut | medium | S–M |
| 9 | **`grip add github:owner/repo`** writes the `pkg` line by inspecting the release's Linux x86_64 assets; plus a shipped 100-entry catalog of common CLI tools. | add | high (adoption) | M |
| 10 | **Merge `check`/`hook`/`schedule`/`environment` out of the catalog** into attributes (`verify:`, `onActivate:`, `schedules:`, `env:`); names remain only for `pkg`/`profile`/`task`/`service`. | rework | medium | S (with #1) |
| 11 | **Editor types without changing the frontend.** `grip types` writes `@gripsack/core` `.d.ts` (or `grip init` does) under a path the capture excludes by default, so newcomers get red squiggles before their first approval. | add | medium | S |
| 12 | **Conda via a provisioned solver binary** (rattler/pixi pinned by hash like Deno) instead of 28k vendored lines; same lock format. | rework | medium (maintenance) | L |
| 13 | **aarch64 Linux.** Same code paths; `{arch}` placeholders already exist. | add | medium | M |
| 14 | **Docs from runnable examples only.** Every code block on the site is an `examples/workspaces/*` directory run in e2e; delete prose that restates refusals (the scheduled-task README is 177 lines for a 27-line workspace). | rework | medium | S–M |
| 15 | **Host-conditional templating and secrets.** `copy(path, { vars: {...host} })`; `age`-encrypted files decrypted at activation (0001 §7). | add | medium | M |

### Where this review disagrees with the previous one

- **Freeze vs cut.** The previous review (§1.4, item 14) said freeze
  services, sharing and Conda. Freezing keeps the build time, the docs and
  the surface a newcomer must understand. Cut sharing, Pixi and wheels;
  keep and relax Conda; turn services into a compose generator.
- **The TS validator.** It proposed generating the TS pre-check from Rust.
  I would delete most of it: TypeScript's compiler is the pre-check if the
  types are right and the user has them (#11); Rust already returns
  spanned diagnostics and is the only authority that needs to exist.
- **Order.** It put the DSL at item 11 of 16. For the goal in §3.4, the
  DSL and the approval unit are items 1 and 2: the core is now proved
  enough that the thing stopping users is the front door.

### Where this review disagrees with the owner

- The frontend is not tedious *because* it is disciplined. Most of the
  ceremony carries no information (§1.3). Keep TypeScript; the five-times
  rejected TOML frontend (`STATUS.md`, settled rejections) would have every
  problem in §1.2 and none of the fixes in §1.4.
- The vision should not be a Nix/devenv successor (§3.2). The project's
  evidence — what it has proved, what it has cut, what platform it runs on
  — points at a different and better-defended product.

---

## Appendix A — transcripts behind the numbers

All with `grip 0.53.0`, `HOME` and `GRIPSACK_HOME` under a scratch
directory, stdin closed unless noted.

**Init → apply (6 commands, 1 approval):**

```
$ grip init demo && cd demo
$ grip check
grip: captured source/policy is not approved for …/demo
hint: inspect with `grip trust inspect --json`, then approve with `grip trust add --bundle b827fd… --policy 917b3d…`
$ grip trust add --bundle b827fd… --policy 917b3d…
$ grip check          # 0.29 s
check: ok 1 named outputs · host linux/x86_64
$ grip plan hello     # 0.23 s
  + 44cbb0…/output → ~/.config/hello/hello.toml (new)
$ grip apply hello    # 0.31 s
applied — generation 1 active
```

**README edit re-prompts:**

```
$ echo "# note" >> README.md && grip check
grip: captured source/policy is not approved … --bundle eacb48… --policy 2ce597…
```

**Release archive, docs example verbatim (`abi: "musl"`):**

```
$ grip update          # 0.9 s → ripgrep 15.2.0, 29-line lock
$ grip check
grip: captured source/policy is not approved …        # the lock changed
$ grip trust add … && grip run --env search -- rg --version
error[E128]: environment targets linux-x86_64 with ABI Some(Musl) and minimum OS None;
host linux-x86_64 has ABI Some(Gnu) and measured OS release Some(OsRelease { major: 6, minor: 18, patch: 33 })
  --> gripsack.ts:17:16  ^ incompatible target declared here
```

**Conda, docs example verbatim:**

```
$ grip update          # 3.3 s, 642-line lock
$ grip trust add … && grip run --env dev -- python --version   # 13 s cold materialize
error[E128]: Conda prefix requires "libX11.so.6" outside the explicit OS runtime;
include that library in the frozen package closure
  --> gripsack.ts:3:15  ^ consumer declaration here
$ grip build tools     # succeeds: bin/python and bin/rg exist in the store
```

**Diagnostics for five wrong declarations (each needed its own approval):**

```
packages: ["tol"]            → error[E126]: output 'dev' references unknown output 'tol'   (good)
pkg without target           → error[E130]: pkg("tool"): target must be an object
exec({ argv: ["/usr/bin/true"] }) → error[E130]: exec(...): argv[0] must be an object
packages: [tool]  (handle)   → error[E130]: environment("dev"): packages[0] must be a non-empty string
trackedCopyTo(".bashrc")     → error[E130]: destination.path: must be absolute or start with ~/ …   (good)
```

**Adopt (2 invocations, 2 approvals), then drift:**

```
$ grip adopt ~/.config/foo --name foo --mode tracked_copy --yes
  wrote configs/foo/ · profiles/foo.ts · gripsack.ts
grip: review the generated files, approve …, then resume      # exit 1 by design
$ grip trust add … && grip adopt ~/.config/foo --name foo --resume --yes
  foo ✓ took over foo → ~/.config/foo/foo.toml
applied — generation 3 active
$ echo 'a = 2' > ~/.config/foo/foo.toml && grip plan foo
  = ~/.config/foo/foo.toml (satisfied)                          # drift not reported
$ grip rollback 2 && cat ~/.config/foo/foo.toml
a = 2                                                           # drift preserved (correct)
```

Generated `profiles/foo.ts` — the shortest gripsack.ts in the repository:

```ts
import { profile, file, repoFile, identity, trackedCopyTo } from "@gripsack/core";

export default profile("foo", {
  files: [
    file({ source: repoFile("configs/foo/extra.toml"), content: identity(), destination: trackedCopyTo("~/.config/foo/extra.toml") }),
    file({ source: repoFile("configs/foo/foo.toml"), content: identity(), destination: trackedCopyTo("~/.config/foo/foo.toml") }),
  ],
});
```

## Appendix B — numbers

| Metric | Value | Source |
|---|---|---|
| Named SDK exports | 112 values/types from `index.ts` | `typescript/src/index.ts` |
| SDK size | 3,856 lines; `validate.ts` 608, `emit.ts` 380 | `wc` |
| Rust IR/sema | 10,779 lines in `gripsack-ir`, 8,797 under `workspace_model`/`sema` | `wc` |
| Example workspaces | 15–43 lines, 7–17 imports each | `examples/workspaces/*`, `e2e/fixtures/*` |
| Approval inventory for a 7-file repo | 42 objects (20 are the embedded frontend) | `grip trust inspect` |
| Lock for `python 3.12 + ripgrep` | 642 lines | `gripsack.lock` |
| State after three small workspaces | 341 MB (98 MB Deno, Conda prefix, store) | `du` |
| Dead crate | `griplint`: 1,838 lines, 20 packs, 0 dependents | `grep -l griplint crates/*/Cargo.toml` |
| Website mentions of `pluginFetch`/gripfetch | 0 | `grep` over `doc/*.md` |
| Scheduled-task example: code vs caveats | 27 lines of TS, 177 lines of README | `wc` |
| Releases | 92 `core-v*` tags in 11 weeks | `git tag` |
