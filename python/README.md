# gripsack (python frontend)

Typed module DSL for [gripsack](https://gripsack.dev). Modules written
against this package evaluate to IR (JSON) that the `grip` core
consumes. Fully annotated — ships `py.typed` (PEP 561), so pyright
gives you autocomplete, inline errors, and refactors.

```python
from gripsack import module, github_release, symlink, tracked_copy, dep

helix = module(
    "helix",
    fetch=github_release(
        repo="helix-editor/helix",
        asset="helix-{version}-x86_64-linux.tar.xz",
    ),
    install={"bin/hx": symlink("~/.local/bin/hx")},
    config={"config.toml": tracked_copy("~/.config/helix/config.toml")},
    depends=[dep("git")],
)
```

## API overview

| area | exports |
|---|---|
| modules | `module`, `Module` |
| fetchers | `github_release`, `tarball`, `git`, `file_fetch`, `plugin_fetch` |
| destinations | `symlink`, `tracked_copy`, `merge`, `template` |
| dependencies | `dep(module, edge="runtime")` |
| activation | `service`, `fonts`, `desktop_entry`, `custom_hook` |
| steps | `step`, `fetch_step`, `build_step`, `shell_step` |
| verify | `verify_binary`, `verify_file`, `verify_shell` |
| graph | `emit_ir`, `clear_graph`, `current_facts` |

Modules are data: evaluation emits IR; the Rust core only ever consumes
IR. Every module captures a source span so core errors point back at
the exact line of your code.

API is pre-alpha and will change with the IR schema
([plan](https://github.com/gripsack-dev/gripsack/tree/main/plan)).
