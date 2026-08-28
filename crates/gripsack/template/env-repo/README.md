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

- `env.toml` — the environment declaration (name, settings)
- `hosts/` — one TypeScript entrypoint per machine; each is a function
  receiving the machine's facts and returning the environment
- `modules/` — one TypeScript file per concern (a module is a value)
- `configs/` — config payloads, deployed into the store by reference
- `grip.lock` — fetch pins; commit it (created on first fetch)

Evaluation is sandboxed (no env vars, no network, no subprocesses —
plan/0013): machine facts and probes arrive through the entrypoint's
`ctx`, so a plan tells you exactly what influenced it.

Docs: https://gripsack.dev — the `gripsack-adopt` skill teaches an
agent to migrate an existing dotfiles setup into this layout.
