# gripsack (python frontend)

Typed module DSL for [gripsack](https://gripsack.dev). Modules written
against this package evaluate to IR (JSON) that the `grip` core consumes.

```python
from gripsack import module, github_release, symlink, tracked_copy

helix = module(
    "helix",
    source=github_release(
        repo="helix-editor/helix",
        asset="helix-{version}-x86_64-linux.tar.xz",
    ),
    install={"bin/hx": symlink("~/.local/bin/hx")},
    config={"config.toml": tracked_copy("~/.config/helix/config.toml")},
)
```

API is pre-alpha and will change with the IR schema (plan/0001).
