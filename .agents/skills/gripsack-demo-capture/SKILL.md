---
name: gripsack-demo-capture
description: Record and re-render gripsack demo GIFs with VHS
---

# Demo capture

Demos are VHS tapes of real CLI flows, rendered by the `demo` workflow
against the freshly built musl binary. `demos/demo.gif` is the README
hero; it must always show the current CLI.

## Local render

```bash
docker compose run --build --rm -e VERSION=0.0.0-demo release
docker run --rm -v "$PWD:/vhs" -w /vhs --entrypoint sh ghcr.io/charmbracelet/vhs -c \
  "install -m755 /vhs/dist/gripsack-*/grip /usr/local/bin/grip && vhs /vhs/demos/demo.tape"
```

## Tape rules

- `Type@10ms` for shell/launch lines; generous `Sleep` after commands
  that fetch or build — viewers forgive pauses, not flicker.
- Keep tapes under ~30 s. One tape = one story (apply, rollback,
  single-module sync). Split before you slow down.
- The tape runs inside the VHS container: only what setup installs is
  available. Fixture data goes in `demos/` and is referenced by absolute
  container path (`/vhs/demos/...`).
- `Set Theme "Catppuccin Mocha"` — matches the site default palette.

## Workflow behavior

The `demo` workflow renders on changes to `crates/` or `demos/` and opens
a reused, force-pushed `demo/artifacts` PR (bot PRs get no CI — merge
with admin override). Until `grip apply` lands the trigger is
`workflow_dispatch` only; enable the path triggers in the same PR that
implements apply.
