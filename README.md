# gripsack

**Your user-space environment, in one workspace.** Declare packages, development
environments, source builds, OCI images and personal profiles in typed TypeScript.
Use the same named packages for project commands, managed launchers and images.

**Alpha · Linux x86_64 and WSL2 only.** Native Windows, macOS and ARM are outside
the supported scope.

## What you can build

- **Packages** from release archives, coherent native Conda environments or an
  explicitly imported Pixi lock. Pin inputs before using them.
- **Environments** for `grip run` and `grip shell`, without installing into a
  global prefix or deploying your personal files.
- **Source builds** using digest-pinned toolchains and isolated BuildKit workers.
  Retain the result as a package or export it in an **OCI image archive**.
- **Profiles** for dotfiles and environment launchers, with explicit ownership,
  drift protection, generations and rollback.
- **Tasks and checks** for deliberate actions and validation, not an implicit
  background scheduler.

Native providers and file-only profiles need no Docker worker. BuildKit source
builds and OCI exports require Docker; the core provisions its matching helper
and owned worker when needed.

## Install

Follow the [installation guide](https://gripsack.dev/docs/install.html) and
[release downloads](https://github.com/gripsack-dev/gripsack/releases). Use a
matching core and SDK archive, or the core's embedded frontend. Unsigned releases
require manual installation: the verified installer and `grip self-update`
require signatures.

## Start with one profile

Save this complete example as the required root `gripsack.ts` in a repository:

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
grip check
grip plan dotfiles
grip apply dotfiles
```

Review source and policy when prompted, then inspect the plan before applying.
`grip generations` lists retained generations; `grip rollback` restores previous
managed state. To deploy files an application rewrites, choose `trackedCopyTo`:
local drift is preserved by default rather than silently overwritten.

## Grow the workspace

Add named outputs to the same catalog and select what you need:

```sh
grip update                         # refresh external source pins; review the lock
grip build my-package               # produce without personal activation
grip run --env dev -- my-command    # execute in a selected environment
grip build my-image                 # export a retained OCI archive
grip task run my-task               # explicitly invoke an action
```

These commands name outputs you declare; see the complete
[runnable workspaces](examples/workspaces/) for packages, environments and images.
Approval binds the exact captured source and policy. Git ignore rules do not
select captured files; declare cache exclusions explicitly in `env.toml` when
needed. Keep backups and recovery state until you have exercised the real tools.

## Documentation

- [Workspaces](https://gripsack.dev/docs/workspaces.html) and
  [packages](https://gripsack.dev/docs/packages.html)
- [Environments and launchers](https://gripsack.dev/docs/environments.html)
- [BuildKit and OCI images](https://gripsack.dev/docs/builds.html)
- [Profiles and dotfiles](https://gripsack.dev/docs/adopting.html)
- [Tasks and checks](https://gripsack.dev/docs/tasks.html)
- [SDK reference](typescript/README.md) and [runnable examples](examples/workspaces/)
- [Safety and recovery](https://gripsack.dev/docs/safety.html),
  [source capture](https://gripsack.dev/docs/settings.html) and
  [architecture](https://gripsack.dev/docs/architecture.html)
- [Design records](plan/) and [changelog](CHANGELOG.md)

## Development

```sh
docker compose run --build --rm test
docker compose run --build --rm ts-test
docker compose run --build --rm e2e uv run --locked --no-sync pytest --ignore=test_persistence_matrix.py --ignore=test_golden.py
```

See [AGENTS.md](AGENTS.md) for working agreements and the
[local qualification profile](.ci/pilot.json) for release checks. Formal research
commands and their evidence are separate from practical product qualification.

MIT licensed.
