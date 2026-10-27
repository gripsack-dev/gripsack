---
name: gripsack-adopt
description: Inspect an existing tool and declare its packages, configuration and commands with explicit ownership, verification and recovery
---

# Adopting a tool into gripsack

Preserve the working installation. Inspect what exists, ask only about choices
that the machine cannot answer, and write a small declaration with a tested
recovery path. Use matching core and SDK release assets or the embedded frontend.

The repository's root `gripsack.ts` is the sole authoring entrypoint. Read the
[SDK reference](../../typescript/README.md) for current workspace declarations.

## 1. Inventory before asking

Find the actual executable, its resolved target, version and installation
method. Inspect its configuration tree, modes, internal aliases and any runtime
resources loaded relative to the executable. Determine whether the application
rewrites its config or installed package tree. Check existing ownership with
`grip why-owns PATH` before proposing a destination.

Use read-only observations first. Do not execute an unknown binary merely to
identify it, install a solver, mutate an existing environment, or replace a
working command as an inventory step.

## 2. Grill the user (only what you can't see)

- **Source and upgrade policy:** inspect the actual release asset/archive layout.
  Use a package provider such as `githubRelease`, `tarball`, `git`,
  `conda.environment` or captured `pixi.fromLock` as appropriate. A Conda solve is
  coherent, not a set of independent package solves or an unpinned shell install.
  Declare an explicit libc/kernel baseline for cross-host solves. A frozen Pixi
  import does not upgrade its external lock.
- **Configuration ownership:** use `trackedCopyTo` when tools rewrite files or
  require physical siblings; `symlinkTo` for immutable individual files; and
  `managedBlock` for a shared shell file. Captured executable `repoFile` plus
  `templateFile` avoids duplicated template bodies. Use `repoTree` for captured
  trees and safe internal aliases, not a generated manifest.
- **Command exposure:** select an environment and explicit profile launcher
  directory/commands for editor or SSH PATH. Activation alone does not create
  a launcher. Match the actual target ABI; host library authority is explicit,
  not inferred from direct host success.
- **Payload selection:** workspace selectors are literal. For a release with one
  versioned root directory, opt into `archiveRoot: { kind: "single_directory" }`
  and use root-relative selectors. Ambiguous roots must refuse. Release trees
  with self-location or extension-sensitive consumers need physical copies.
- **Verification:** use sealed package checks only with a complete admitted
  runtime closure. A deliberately deployed-host check proves host behavior,
  not hermetic closure. Keep functional checks relevant to the actual consumer.

Pure typed functions can compose declarations. Do not add ambient filesystem,
network or environment reads to evaluation, a custom walker, or a second
configuration framework.

## 3. Write the declaration

A complete configuration-only workspace can adopt a captured Helix config tree
without moving its package management:

```ts
import {
  defineWorkspace, file, identity, profile, repoTree, trackedCopyTo, workspace,
} from "@gripsack/core";

const helix = profile("helix", {
  files: [file({
    source: repoTree("configs/helix"),
    content: identity(),
    destination: trackedCopyTo("~/.config/helix"),
  })],
});

export default defineWorkspace(() => workspace({ outputs: [helix] }));
```

Copy the intended configuration into `configs/helix` before capture, preserving
executable bits and reviewing secrets. Add package/environment/launcher declarations only
when the user wants Gripsack to manage the tool too; derive them from observed
asset paths rather than a guessed archive name. See the runnable
[workspace examples](../../examples/workspaces/).

To generate the declarations, run:

```sh
grip adopt ~/.config/helix --name helix --mode tracked_copy
```

The interview captures files under `configs/helix/`, writes a default-exported
profile in `profiles/helix.ts`, and adds it to a simple multiline root
`gripsack.ts` outputs list. For more complex roots it prints the import/output
snippet to integrate manually. Modes are `owned` (symlinks), `tracked_copy`,
and `merge` (managed blocks). Inspect generated declarations and captured files.

Generation deliberately stops with exit status 1 for source review, including
with `--yes`; it does not apply unapproved source. After reviewing, approve and
resume the same target and profile:

```sh
grip trust add .
grip adopt ~/.config/helix --name helix --resume
```

If the first invocation used `--host`, keep that identity on resume. Resume
selects the evaluated profile and rejects destinations outside the adopted
target; it applies only that profile. `--yes` skips confirmation, never source
approval.

## 4. Prove it

Inspect `grip trust inspect --json`, review changed source and policy, then
approve that exact snapshot. Interactive `grip trust add` supports review;
automation supplies reviewed `--bundle` and `--policy` digests. Never approve
whatever an unreviewed script captures or use `GRIPSACK_TRUST_ALL`.

When source/frontend pins change, run `grip update`, review the resulting lock,
and approve the changed snapshot before execution. Declare cache exclusions in
`env.toml`; Git ignore files do not define captured authority. Global globs must
not carve an admitted SDK pin. An editor-only SDK can be excluded explicitly to
select the embedded frontend.

```sh
grip check
grip plan helix
grip apply helix
```

Resolve reported destination conflicts before applying. Use `--take-over` only
for explicitly reviewed foreign/drifted tracked copies. Intact legacy ownership
instead uses generation-bound `--migrate-from N`; prepare replacements first and
retain that predecessor until functional checks pass.

Exercise the deployed tool, not just the declaration: executable mode, PATH,
sibling resources, application loading and meaningful error behavior. Reapply
unchanged source and inspect the result. Complete unchanged workspace state can
reuse its generation, but checks/hooks may still execute and external effects
are not necessarily idempotent. Test recovery in isolation before a live cutover.

## 5. The honesty bit

Report which destinations changed, how originals and prior generations are
retained, the verification actually performed, and the recovery command. Keep
backups independent of generation retention. `grip rollback N` restores retained
managed state, not arbitrary external side effects; late activation failure does
not automatically roll back. Never delete journals, discard generations, or
claim an unexercised recovery path works.
