---
name: gripsack-adopt
description: Interview the user about an existing tool on their machine and write a correct gripsack module for it — fetcher choice, version pinning, config ownership, verify contract
---

# Adopting a tool into gripsack

The user has a working machine and wants one tool managed by gripsack.
Your job: figure out what the tool actually IS on their system, ask the
questions only they can answer, and write a module that applies cleanly.
Never guess when the system can tell you.

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
- **Which fetcher?** Map from what you found:
  - GitHub release → `github_release(repo, asset)` (0.2; use
    `file_fetch` of a downloaded tarball today)
  - tarball URL → `tarball(url, sha256=...)`
  - git source build → `git(url, rev)` + build step
  - internal registry → a resolver in `lib/` (eval-time code, 0002 §3)
  - pixi/conda → `shell_step` running pixi with `outputs` declared and
    the `pixi-lock` resource (0007 §4)
- **Config ownership per file** (0001 §3.7) — THE question that matters:
  - "Does this app ever rewrite its own config?" (VS Code, most GUI
    apps) → `tracked_copy` (drift is kept, never clobbered)
  - Disciplined read-only tools (helix, git) → `symlink` (owned)
  - `merge`/`template` don't exist yet — `plan` rejects them (E108).
- **A verify contract**: almost always `verify_binary("bin/<tool>")`.

## 3. Write the module

Data style for most tools; class style when there's a custom step.
Rules you must hold:

- Every module: a `verify` — no exceptions.
- Destinations absolute or `~/`-prefixed (E102).
- `dep()` for real dependencies; `Edge.BUILD` for build-only toolchains.
- If the tool needs a lock (package managers!) declare
  `resource("pixi.lock")` first and require it on the step.
- Config `from` paths are repo-relative; the file must exist in the
  repo. If the user's config lives only on disk, copy it into the repo
  FIRST and tell them it's now managed there.

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
