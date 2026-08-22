# 0005 — Frontends and tool configuration

- Status: draft
- Date: 2026-08-22
- Amends: 0001 §3.3 (frontends), 0001 §5 (env repo layout)

## 1. TypeScript is a first-class frontend

Python was the blessed default; TypeScript joins as an equal, in the same
monorepo, held to the same contract:

| | python | typescript |
|---|---|---|
| package | PyPI `gripsack` | npm `@gripsack/core` |
| IDE story | pyright | tsc (native) |
| runtime | user's python ≥3.10 | user's node ≥18 or bun |
| emits | IR JSON on stdout | IR JSON on stdout |

- Same DSL shape, same IR, same schema, same provenance obligation
  (spans via V8 stack frames — file:line:col for free).
- The core never embeds either runtime: frontends are subprocesses
  emitting IR (0001 §3.3). A third frontend is a package, not a fork.
- An env repo picks **one** frontend per eval — declared in `env.toml`,
  not sniffed. Mixed-python-and-TS module graphs are an anti-goal: one
  repo, one language, one eval.

## 2. Tool configuration

grip itself is configured in layers (later wins):

```
built-in defaults
  < user config   ~/.config/gripsack/config.toml   (machine-local: paths, secrets-adjacent)
  < repo env.toml                                  (travels with the dotfiles)
  < env vars      GRIPSACK_*
  < CLI flags
```

`env.toml` (repo-level, committed — the env is self-describing):

```toml
[env]
name = "tarek"
frontend = "python"            # or "typescript"

[eval]
# frontend-environment deps — resolvers/sourcerers the modules import
deps = ["gripsack-sourcerer-artifactory==1.2.0"]

[sources.artifactory]
plugin = "gripsource-artifactory"   # transport, if a resolver needs one (0002)

[settings]
keep_generations = 20
```

User config holds what must NOT be committed (machine-local overrides);
repo config holds everything that makes the env reproducible.

## 3. Evaluation / derivation order

The ordering question has one hard rule: **tool configuration is data,
read before any code runs — it can never depend on eval results.** No
cycles, by construction.

```
1  locate repo; read env.toml                 (pure TOML — safe, no eval)
2  merge config layers → effective Config
3  provision frontend env per [eval]          (python/node + deps; cached)
4  eval modules → IR                          (facts/tags resolved here)
5  core passes (0004 §4): parse→validate→resolve→lower→plan
6  execute → activate → record generation
```

Consequences:

- Sourcerer declarations and eval deps are known *before* eval, so a
  module may `import` a resolver whose package env.toml pinned — the
  env repo genuinely carries its own skills (0002 §3).
- Step 3 is content-cached: same `[eval]` spec, same venv/node_modules,
  no re-provision.
- `grip doctor` checks exactly steps 2–3 for the configured frontend:
  config parses, runtime present, frontend package importable.

## 4. Frontend protocol (v1)

```
grip → <frontend-runtime> <entrypoint> --emit-ir --host <name> \
         --tags a,b --out -            (stdout: IR JSON)
```

Exit non-zero + stderr on eval failure; stderr passes through with the
frontend's own formatting (pyright-style tracebacks are the frontend's
domain; core errors start at IR-parse time, span-labeled — 0004).
