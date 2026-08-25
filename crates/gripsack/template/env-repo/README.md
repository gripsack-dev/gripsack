# my-env — a gripsack environment

Your whole environment, in one bag: tools, configs, per-host
variations — declarative, content-addressed, with generations and
instant rollback.

```bash
grip check        # validate everything, zero side effects
grip apply        # deploy — one atomic generation per run
grip plan         # what would apply change?
grip generations  # history
grip rollback     # undo the last apply, instantly
```

Layout:

- `env.toml` — the environment declaration (frontend, eval deps, settings)
- `hosts/` — one entrypoint per machine, tags for per-host variation
- `modules/` — one Python file per concern (a module is a function call)
- `configs/` — config payloads, deployed into the store by reference
- `grip.lock` — fetch pins; commit it (created on first fetch)

Docs: https://gripsack.dev — the `gripsack-adopt` skill teaches an
agent to migrate an existing dotfiles setup into this layout.
