---
name: gripsack-release
description: Cut gripsack releases — two artifacts, two tag namespaces, two registries
---

# Releasing

Two artifacts ship from this repo (plan/0003 §6): the `grip` binary
(crates.io `gripsack`) and the Python frontend (PyPI `gripsack`). Versions
are independent; the IR version is the compatibility contract.

## Core (binary)

```bash
# version must equal crates/gripsack's version (Cargo.toml workspace)
git tag core-v0.1.0 && git push origin core-v0.1.0
```

The workflow builds the musl tarball, verifies it (checksum, static via
`ldd`, `--version`), then publishes **crates.io first** — it is the
irreversible artifact; a failure there must not leave a GitHub release
pointing at nothing. The tag guard fails mistags. Needs
`CARGO_REGISTRY_TOKEN` in repo secrets.

## Python (frontend)

```bash
# version must equal python/pyproject.toml
git tag py-v0.1.0 && git push origin py-v0.1.0
```

Builds the wheel with `uv build` and publishes with `uv publish`. Needs
`PYPI_API_TOKEN` in repo secrets. (Trusted publishing is the better
long-term setup — configure it on PyPI and drop the token.)

## Checklist

- [ ] Versions bumped in the right files; IR version compatibility noted
      in the release notes if it changed
- [ ] compose gates green on the tagged commit
- [ ] crates.io publish succeeded BEFORE announcing the GitHub release
- [ ] `demos/demo.gif` current (merge any open `demo/artifacts` PR first)
- [ ] Website install placeholders updated when the first real release
      ships (other repo: gripsack-dev.github.io)
