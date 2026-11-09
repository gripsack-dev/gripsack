# gripsack

**Your environment, in one workspace.** Describe the tools, dotfiles,
environments, services and tasks you rely on in one typed TypeScript file.
`grip` shows you the exact changes, applies them as a transaction, keeps every
state it ever applied, and rolls back in one command. The decisions that make
that safe are proved in Lean, over the Rust that ships.

**Alpha · Linux x86_64 and WSL2 only.** Documentation: [gripsack.dev](https://gripsack.dev).


## Why

Setting up a machine is a pile of commands nobody can replay, and the tools
that fix it each own one slice: a dotfile manager here, a version manager
there, `docker compose` for the database. Gripsack puts the user-space layer in
one file and treats applying it like a database treats a transaction:

- **Plan first.** Evaluation runs in a sandboxed Deno with no network and no
  environment. You see every file that would change and every source you are
  about to trust before anything happens.
- **Apply as a transaction.** Prior state is journaled before any write, each
  destination is re-checked for your own edits, and one atomic rename makes
  the new generation current.
- **Keep a way back.** Every applied state is a retained generation.
  `grip rollback` reselects one and restores each file to exactly what it
  deployed, from retained bytes and checked against its record, or refuses
  before switching; nothing is rebuilt and your local edits are preserved.
- **Proved where it matters.** The commit, garbage-collection and journal
  decisions live in a dependency-free kernel crate that is translated to Lean 4
  and proved there. [What is proved, assumed and not claimed →](https://gripsack.dev/docs/guarantees.html)

## What you declare

| Output | What it gives you |
|---|---|
| **Packages** | Tools from release archives, a coherent Conda solve or an imported Pixi lock, pinned in `gripsack.lock`; no global prefix |
| **Environments** | `grip run` / `grip shell` with selected packages on the path, and launchers deployed through a profile |
| **Profiles** | Dotfiles with explicit ownership (`symlinkTo`, `trackedCopyTo`, `managedBlock`), drift protection, generations and rollback |
| **Services** | Digest-pinned Docker Engine containers on loopback ports with health checks; `grip down` keeps the volumes |
| **Tasks and checks** | Named command graphs with prerequisites, locks and postconditions; validate a file before it is written |
| **Schedules** | systemd user timers that run the same tasks on a calendar |
| **Sharing** | Signed package bundles moved to another machine or through an OCI registry |

Native packages, tasks and file-only profiles need no container runtime.
There is no daemon: Docker Engine runs services, systemd owns timers.

## Start with one profile

[Install `grip`](https://gripsack.dev/docs/install.html), then save this as
`gripsack.ts` in its own repository (or run `grip init my-env`):

```ts
import {
  defineWorkspace, file, literalText, managedBlock, profile, workspace,
} from "@gripsack/core";

const dotfiles = profile("dotfiles", {
  files: [file({
    content: literalText("export EDITOR=vi\n"),
    destination: managedBlock("~/.profile", "gripsack-editor"),
  })],
});

export default defineWorkspace(() => workspace({ outputs: [dotfiles] }));
```

```sh
grip check            # validate; approve the captured source when prompted
grip plan dotfiles    # read the exact changes
grip apply dotfiles   # generation 1
grip generations      # what you can return to
grip rollback         # return to it
```

The managed block owns only the section between its markers; the rest of
`~/.profile` stays yours. For files an application rewrites, use
`trackedCopyTo`: your later edits are preserved, never silently overwritten.

## Grow the workspace

```sh
grip update                         # resolve sources into gripsack.lock; review the diff
grip build my-package               # realize a package without deploying anything
grip run --env dev -- my-command    # run in a selected environment
grip up postgres                    # start a service and wait for health
grip task run my-task               # run a task; required services must be healthy
grip down postgres                  # stop the container, keep its volumes
```

Complete, runnable workspaces live in [`examples/workspaces/`](examples/workspaces/)
and each guide on the site is one: [packages](https://gripsack.dev/docs/packages.html),
[environments](https://gripsack.dev/docs/environments.html),
[profiles and dotfiles](https://gripsack.dev/docs/adopting.html),
[services](https://gripsack.dev/docs/services.html),
[tasks](https://gripsack.dev/docs/tasks.html),
[schedules](https://gripsack.dev/docs/schedules.html),
[review and saved plans](https://gripsack.dev/docs/review.html),
[sharing](https://gripsack.dev/docs/sharing.html).

## Safety, honestly

- [How gripsack stays safe](https://gripsack.dev/docs/guarantees.html): the
  rings, the Lean theorems, the storage assumptions, and the residual risks
  (an external writer in the window between re-check and write, effects of
  your own hooks and services, liveness).
- [Safety and recovery](https://gripsack.dev/docs/safety.html): what apply
  does step by step, what a crash leaves behind, ownership and drift.
- The design is [plan 0056](plan/0056-verified-foundation.md); the proofs are
  in [`proofs/`](proofs/) over [`crates/gripsack-kernel`](crates/gripsack-kernel).

Generations protect managed files and launchers. They do not undo a database
migration, a scheduled task's effects or anything a hook did. Keep independent
backups of anything irreplaceable.

## Reference

- [SDK reference](typescript/README.md) — every declaration and its contract
- [Sharing reference](docs/sharing-reference.md) — the signed bundle format in full
- [Agent skills](skills/) — `gripsack-adopt` and `gripsack-debug` for coding agents
- [Design records](plan/) and [changelog](CHANGELOG.md)

## Development

```sh
docker compose run --build --rm test      # fmt + clippy + cargo test
docker compose run --build --rm ts-test   # TypeScript frontend tests
docker compose run --build --rm e2e       # flow tests against the real binary
docker compose run --build --rm proofs    # Lean proofs over the kernel translation
```

See [AGENTS.md](AGENTS.md) for working agreements.

MIT licensed.
