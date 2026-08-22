# @gripsack/core

TypeScript frontend for [gripsack](https://gripsack.dev) — a typed module
DSL that evaluates to the IR the `grip` core consumes.

```ts
import { module, githubRelease, symlink, trackedCopy, dep } from "@gripsack/core";

module("helix", {
  fetch: githubRelease({
    repo: "helix-editor/helix",
    asset: "helix-{version}-x86_64-linux.tar.xz",
  }),
  install: { "bin/hx": symlink("~/.local/bin/hx") },
  config: { "config.toml": trackedCopy("~/.config/helix/config.toml") },
  depends: [dep("git")],
});
```

Everything is fully typed — your editor gives you autocomplete and
inline errors for free. Modules are data: evaluation emits IR (JSON);
the Rust core only ever consumes IR.

API is pre-alpha and will change with the IR schema
([plan](https://github.com/gripsack-dev/gripsack/tree/main/plan)).

## API overview

| area | exports |
|---|---|
| modules | `module`, `ModuleSpec` |
| fetchers | `githubRelease`, `tarball`, `git`, `fileFetch`, `pluginFetch` |
| destinations | `symlink`, `trackedCopy`, `merge`, `template` |
| dependencies | `dep(module, edge?)` |
| activation | `service`, `fonts`, `desktopEntry`, `customHook` |
| steps | `step`, `fetchStep`, `buildStep`, `shellStep` |
| verify | `verifyBinary`, `verifyFile`, `verifyShell` |
| graph | `emitIr`, `clearGraph` |
