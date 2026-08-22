# gripsack

Your whole environment in one bag. Packages from any source **plus** your
dotfiles, described once in typed Python, deployed by a Rust core into a
hash-addressed store — with generations and rollback.

**Status: scaffold.** The design is public and moving; nothing is
installable yet.

- Website: [gripsack.dev](https://gripsack.dev)
- Plan: [plan/0001-architecture.md](plan/0001-architecture.md) ·
  [0002-sourcing](plan/0002-sourcing.md) ·
  [0003-repo-and-tooling](plan/0003-repo-and-tooling.md)

## Layout

| path | what |
|---|---|
| `crates/gripsack` | the `grip` CLI → crates.io `gripsack` |
| `crates/gripsack-{ir,store,exec,source}` | core: IR, store, DAG, fetchers |
| `python/` | typed module DSL → PyPI `gripsack` |
| `schema/ir/` | the IR contract |
| `e2e/` | flow tests (real binary + real frontend, offline) |

## Development

Docker-first gates (host has no toolchain contract):

```sh
docker compose run --build --rm test     # fmt + clippy -D warnings + cargo test
docker compose run --build --rm pytest   # python frontend tests
docker compose run --build --rm e2e      # flow tests
```

See [AGENTS.md](AGENTS.md) for working agreements.

MIT licensed.
