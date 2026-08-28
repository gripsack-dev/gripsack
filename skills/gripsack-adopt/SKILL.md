---
name: gripsack-adopt
description: Interview the user about an existing tool on their machine and write a correct gripsack module for it — fetcher choice, version pinning, config ownership, verify contract
---

# Adopting a tool into gripsack

The user has a working machine and wants one tool managed by gripsack.
Your job: figure out what the tool actually IS on their system, ask the
questions only they can answer, and write a module that applies cleanly.
Never guess when the system can tell you.

The frontend is typed TypeScript (plan/0013 D5): `modules/<tool>.ts`
default-exports its module value — `module()` constructs, it never
registers — and `hosts/<host>.ts` imports the modules it wants and
returns them from `defineEnv`. Falsy entries drop out; that is where
per-host gating belongs.

## 1. Inventory before asking

```bash
which <tool>                       # where does the binary live?
<tool> --version                   # what version are they on?
ls ~/.config/<tool>/               # what config exists?
readlink -f $(which <tool>)        # is the binary a symlink? to where?
```

Also: how was it installed? (`apt list --installed`, `brew list`,
`pixi global list`, `ls ~/.cargo/bin`, GitHub release? an internal
registry?) The install method decides the fetcher.

## 2. Grill the user (only what you can't see)

- **Pin or float?** Pinned version (reproducible) or `update`-tracked
  latest? Pinned is the default recommendation; the lockfile makes it
  painless either way.
- **Which fetcher?** Map from what you found (all exported from
  `@gripsack/core`):
  - GitHub release → `githubRelease({ repo, asset })`
  - tarball URL → `tarball(url, sha256)`
  - git source build → `git(url, rev)` + build step
  - internal registry / mTLS / non-HTTP → a `gripfetch-*` plugin
    (transport runs in the core; eval is sandboxed and has no network,
    so repo code cannot do credentialed resolution — 0002, 0013 D8)
  - pixi/conda → `shellStep` running pixi with `outputs` declared and
    the `pixi-lock` resource (0007 §4)
- **Config ownership per file** (0001 §3.7) — THE question that matters:
  - "Does this app ever rewrite its own config?" (VS Code, most GUI
    apps) → `trackedCopy` (drift is kept, never clobbered)
  - Disciplined read-only tools (helix, git) → `symlink` (owned)
  - Files other tools also write (`.bashrc`) → `merge` (gripsack owns
    one managed block; everything outside the markers is never touched)
  - Per-host content differences → `template` with `vars` computed from
    `ctx.facts` inside the host entrypoint (undefined variables fail
    loudly at apply)
- **A verify contract**: almost always `verifyBinary("bin/<tool>")`.
- **Payload layouts per fetcher** (what `install` keys look like):
  - `tarball`/`githubRelease`: the archive's contents, verbatim.
  - `fileFetch`: the directory's contents; a bare file stages under its
    basename.
  - `pixi`: the conda env root — `bin/<tool>` works directly.
  - `brew`: the RAW bottle layout — binaries live at
    `{formula}/{version}/bin/<tool>`, so write
    `install: { "{formula}/{version}/bin/jq": symlink(...) }`;
    `{version}` is substituted from the lock. `brew()` floats to the
    current formula (the API only serves stable) — the `version` arg is
    a tripwire that fails at resolve with `grip update` to move, not a
    range.

## 3. Write the module

Data style for most tools; class style when there's a custom step.

```ts
// modules/helix.ts
import { module, symlink, trackedCopy, verifyBinary } from "@gripsack/core";

export default module("helix", {
  fetch: githubRelease({ repo: "helix-editor/helix", asset: "helix-…-x86_64-linux.tar.xz" }),
  install: { "hx": symlink("~/.local/bin/hx") },
  config: { "configs/helix": trackedCopy("~/.config/helix") },
  verify: verifyBinary("hx", ["--version"]),
});
```

```ts
// hosts/laptop.ts — the module is inert until a host returns it
import { defineEnv } from "@gripsack/core";
import helix from "../modules/helix.ts";

export default defineEnv((ctx) => ({
  tags: ["gui"],
  modules: [helix],
}));
```

Rules you must hold:

- Every module: a `verify` — no exceptions.
- Destinations absolute or `~/`-prefixed (E102).
- `dep()` for real dependencies; `Edge.BUILD` for build-only toolchains.
- If the tool needs a lock (package managers!) declare
  `resource("pixi.lock")` first and require it on the step.
- Config `from` paths are repo-relative; the file must exist in the
  repo. If the user's config lives only on disk, copy it into the repo
  FIRST and tell them it's now managed there.
- Machine differences (OS, arch, an optional dependency on some binary
  existing) are host-entrypoint decisions: `ctx.facts`,
  `hasTag("gui", ctx)`, `ctx.probe.executable("nvidia-smi")` — never
  environment reads or filesystem probes inside module files (eval has
  none of those anyway).

## 4. Prove it

```bash
grip plan                    # must be clean — no E1xx
grip apply <tool>            # subset apply, just this module
<tool> --version             # the deployed binary works
grip apply <tool>            # must report already satisfied
```

If the second apply isn't satisfied, the module has an unstable input —
find it (lockfile hash, repo file hash) and fix it, don't ship a
flickering module.

## 5. The honesty bit

Tell the user what changed: which files are now managed, where the
original config was backed up (suggest one), and that `grip rollback`
undoes it. gripsack never deletes their old config — but make them hear
that.
