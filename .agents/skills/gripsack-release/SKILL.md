---
name: gripsack-release
description: Cut gripsack releases — two artifacts, two tag namespaces, two registries
---

# Releasing

Two artifacts ship from this repo (plan/0003 §6): the `grip` binary
(crates.io `gripsack`) and the TypeScript frontend (npm
`@gripsack/core`). Versions are independent; the IR version is the
compatibility contract.

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

## TypeScript (frontend package)

```bash
# version must equal typescript/package.json
git tag ts-v0.1.0 && git push origin ts-v0.1.0
```

Builds with `npm ci && npm test` and publishes with `npm publish`. Needs
`NPM_TOKEN` in repo secrets. The npm package is for IDE types and the
deliberate-pin rule (a repo's own install shadows the embedded copy);
the grip binary embeds the frontend source, so users do not need it.

## Checklist

- [ ] Versions bumped in the right files; IR version compatibility noted
      in the release notes if it changed
- [ ] Embedded frontend changed since the last ts release? Cut the
      matching `ts-vX.Y.Z` — doctor's stale-pin advice interpolates the
      core version, so npm must carry the same major.minor (0.21.1
      review: `npm i -D @gripsack/core@^0.21.0` failed while npm had
      only 0.18.0)
- [ ] compose gates green on the tagged commit
- [ ] crates.io publish succeeded BEFORE announcing the GitHub release
- [ ] `demos/demo.gif` current (merge any open `demo/artifacts` PR first)
- [ ] Website install placeholders updated when the first real release
      ships (other repo: gripsack-dev.github.io)
