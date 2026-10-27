# gripsack

**Your environment, in one workspace.** Describe packages, development
environments and personal profiles in typed TypeScript. Review inputs, inspect
the plan, and deploy personal files with generations and rollback.

**Alpha · Linux x86_64 and WSL2 only.** Native Windows, macOS and ARM are
outside the supported scope.

## Install

Use the [release downloads](https://github.com/gripsack-dev/gripsack/releases)
and their manual installation instructions for unsigned builds.
The existing verified installer and `grip self-update` still require signatures;
they are not the installation path for these unsigned releases.

**[0.47.0](https://github.com/gripsack-dev/gripsack/releases/tag/core-v0.47.0)
is an unsigned, unverified reviewer prerelease.** Follow that release's manual
installation instructions and use its matching SDK archive. Asset readback
and manual install/SDK smoke checks are not full qualification. This is not
a claim of npm or crates.io publication.

Capture-glob exclusions and the `--focus` view require **core/SDK 0.48**;
they are not in 0.47. Check the [release records](https://github.com/gripsack-dev/gripsack/releases)
for available artifacts and qualification evidence, not just a version string.

## A workspace

Save this complete example as `gripsack.ts` in a repository, using matching
core/SDK bytes (the 0.47 reviewer prerelease supports this declaration):

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

Review source and policy when prompted; inspect the plan before applying.
`grip generations` lists retained generations; `grip rollback` restores a
previous managed state. Packages, environments and manual tasks are separate
workspace outputs: use `grip build NAME`, `grip run --env NAME -- COMMAND`,
and `grip task run NAME`, respectively. Production does not itself create a
personal generation.

Profiles deploy files and package environments; tracked copies preserve local
drift. **Locked mutable overlays are not implemented.** For tools that rewrite
their installed tree, deploy reviewed repository content with `trackedCopyTo`.
To retain an update, copy intended changes back from the deployed tree, exclude
caches, review and approve the new source, then apply. The lockfile pins source
resolutions, never a live destination. See the
[SDK migration guide](typescript/README.md#reviewer-migration-delete-plumbing-keep-verification)
for launchers, captured trees, ownership migration and explicit recovery.

## Review before applying

Approval binds the exact captured source and policy, not a repository path.
Use `grip trust add` for interactive changed-path review and approval; use
`grip trust inspect --json` for the complete inventory. Git ignore rules do
not select captured files. Declare cache exclusions explicitly in `env.toml`.
See [source approval and migration](typescript/README.md#source-approval-and-migration)
for exclusions, unattended approval, trust boundaries and recovery.

## Documentation

- [Workspace and SDK reference](typescript/README.md) — typed outputs, profiles,
  file checks, manual tasks and migration.
- [Runnable workspaces](examples/workspaces/) — dotfiles through packages and images.
- [Adopt existing files](https://gripsack.dev/docs/adopting.html) and
  [workspace migration](https://gripsack.dev/docs/workspace-migration.html).
- [Safety and recovery](https://gripsack.dev/docs/safety.html) — ownership,
  drift, interrupted transactions and post-activation failures.
- [Architecture](https://gripsack.dev/docs/architecture.html),
  [fetchers](https://gripsack.dev/docs/fetchers.html), and
  [release scope](https://gripsack.dev/docs/roadmap.html).
- [Design records](plan/) and [changelog](CHANGELOG.md).

## Development

```bash
docker compose run --build --rm test     # fmt + clippy -D warnings + cargo test
docker compose run --build --rm ts-test  # typescript frontend tests (deno)
docker compose run --build --rm e2e uv run --locked --no-sync pytest --ignore=test_persistence_matrix.py --ignore=test_golden.py
docker compose run --build --rm model    # finite protocol models and counterexamples
docker compose run --build --rm tlaps    # transaction induction pilot (amd64)
docker compose run --build --rm verify   # production Verus kernels and mutants (amd64)
```

The [frozen local profile](.ci/pilot.json) requires17 practical release gates,
including all96 persistence partitions. The formal commands above remain
separate research; this release does not claim TLC/TLAPS/Verus qualification.
Hosted CI is manual diagnostics, not duplicate acceptance.
See [AGENTS.md](AGENTS.md) for working agreements.

MIT licensed.
