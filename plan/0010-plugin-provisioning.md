# 0010 — Plugin provisioning through the frontend venv

- Status: draft
- Date: 2026-08-24
- Amends: 0002 §4 (fetcher discovery), 0005 §3 (evaluation order)

## 1. The problem

Plugins are prerequisites. Before `grip plan` works, the machine must
already have every `gripfetch-*` and `griplint-*` the repo declares —
installed by hand, by `uv tool install`, by some internal tarball.
That is seven things to prepare before the first command, and it
contradicts the bootstrap ethos: *`grip apply --repo <repo>` and
everything works, everything version-pinned.*

## 2. The mechanism already exists

Frontend provisioning (0005 §3) builds a venv keyed by the gripsack
package version + `[eval] deps` — one static uv binary, fetched and
sha256-verified by our own fetcher. A Python plugin is just another
dep: its console script lands in the venv's `bin/`, next to the python
that runs the frontend. No PATH mutation, no separate tool installs.

## 3. Registration in env.toml

Explicit, two-sided, committed to the env repo:

```toml
[fetchers.artifactory]
package = "gripfetch-artifactory==2.1.0"   # provisioned into the venv

[fetchers.legacy]
path = "/opt/bin/gripfetch-legacy"          # explicit override

[linters.yazi]
package = "griplint-yazi==1.2.0"
```

- `package` requires an `==` pin. Pins join the venv hash input, so a
  pin change rebuilds the venv and the same env.toml yields identical
  plugin behavior on every machine.
- Executable lookup order: env.toml `path` → provisioned venv `bin/` →
  `PATH` (0002 §4 discovery unchanged as the fallback).
- Modules reference plugins by registry name; an unregistered name is
  a hard error with provenance at plan/eval time — never a silent skip.

## 4. The language boundary

Venv provisioning covers **Python plugins only**. A Rust/Go/internal
binary plugin does not come from a package index; it keeps the
`path`/`PATH` forms, and its pinning story belongs to whatever builds
it (an internal registry's artifact versions). One registry shape,
three forms — do not force one mechanism over both worlds.

## 5. Credentials live in the environment, never in env.toml

Private indexes are reached via `UV_INDEX_URL` / `UV_EXTRA_INDEX_URL`
/ netrc — machine-local environment, never committed config. The hard
rule (the core never sees credentials) is also what keeps env.toml
commit-safe: the repo names *what* to install (`==2.1.0`); the
machine's environment says *where from, with which credentials*.

## 6. Trust

Provisioning fetches plugin *code*; wheels install without executing
anything. The 0002 §4 backstop is unchanged: bytes returned by a
fetcher are hash-verified against the lockfile before entering the
store, so a wrong or malicious plugin can fail an apply, never poison
a store. Version pins at v1; uv's `--hash` requirements are the
hardening step if we later want pip-style hash-locking in env.toml.

## 7. Failure modes

- Unregistered plugin name referenced by a module → hard error at
  plan/eval with the module-line span.
- Provisioning failure → the real cause surfaces (0.4.1 behavior:
  proxy, index, auth), plus the `GRIPSACK_PYTHON` hint. Bypassing
  provisioning with `GRIPSACK_PYTHON` means provisioned plugins are
  absent — the unregistered-name error then names exactly what is
  missing.

*env.toml becomes the manifest of the whole tooling universe: modules
pin the tools, linters pin the schema checkers, fetchers pin the
transports, the lockfile pins the bytes.*
