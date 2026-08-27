# 0001 — Gripsack Architecture

- Status: draft
- Date: 2026-08-22
- Domain: gripsack.dev
- Org: github.com/gripsack-dev (CLI, frontends, and IR tooling live here)

## 1. What gripsack is

Gripsack is a declarative, personal environment manager. One git repository
describes your entire setup — packages sourced from anywhere (GitHub releases,
tarballs, git builds, distro packages, cargo, …) and your dotfiles — and any
machine can be reproduced from it:

```
install grip → grip apply --repo git@github.com:you/myenv → done
```

It borrows Nix's store and generations, deliberately rejects Nix's purity
enforcement, and replaces the Nix language with typed Python evaluated to a
language-neutral intermediate representation (IR).

The name: a gripsack is a 19th-century traveler's hand-bag — the bag that
holds your whole kit when you move. That is the product.

## 2. Design principles

1. **Modules are data, not scripts.** A module is a declarative description
   (typed steps over pluggable verbs) that evaluates to IR. Arbitrary shell is
   an explicit, flagged escape hatch — never the default. This buys `plan`,
   `why-owns`, generation diffs, and reliable uninstall for free.
2. **Evaluation and execution are separate.** Frontends evaluate module code
   into IR; the Rust core only ever consumes IR. The IR is the contract —
   frontends are interchangeable, the core is stable.
3. **Impure by default, reproducible by lockfile.** No build sandboxing.
   Resolution (latest releases, unpinned refs) happens at eval time and is
   pinned, with content hashes, into a per-host lockfile. Machine B matches
   machine A because the lockfile says so, not because we forced it.
4. **User-scoped.** No root, no daemon, no system mutation. Everything lives
   under the user's home. System integration is a later, explicit extension.
5. **Generations are immutable; activation is one atomic rename.** A
   generation is built completely, then a single symlink flips. Rollback is
   flipping it back. Nothing half-applied ever exists.
6. **Intents, not commands.** Modules declare activation intents
   (`service(...)`, `fonts(...)`, `desktop_entry(...)`); platform adapters
   translate. Gripsack takes no side in init-system or OS wars.

## 3. Core concepts

### 3.1 Module

The unit of description. A module declares:

- **sources** — typed fetchers: `github_release`, `tarball`, `git`, `cargo`,
  `apt`/`dnf` (later), … Platform-conditional sources are allowed (see §7).
- **build** — `none | cargo_install | make | cmake | custom_shell` (flagged).
- **install** — mapping of produced paths → destinations, ownership mode per
  path.
- **config** — files/templates → destinations, ownership mode per file.
- **dependencies** — typed edges: `runtime` or `build`. Build-only deps are
  *ephemeral modules* (e.g. a `rust` toolchain module used to
  `cargo install` something): present during the build, referenced by no
  generation, collected by GC afterward.
- **activation** — intents with trigger points (see §3.7).

### 3.2 IR (module graph)

The serialized, versioned data contract between frontends and the core. The
output of evaluation is a fully-resolved DAG of modules with all conditionals
already applied. The core never evaluates code and never branches on platform
facts at execution time — it executes exactly what the IR says.

Because IR is data, `grip plan` (diff of would-be generation vs current),
`grip why-owns <path>`, and generation diffs are mechanical.
**Provenance.** Every IR node carries optional `source: {file, line}` —
the module file that emitted it. This is nearly free to add at emit time
and very expensive to retrofit: it is what lets a validation or execution
error point back at the user's Python instead of at raw JSON. Frontends
SHOULD emit it; the core preserves and surfaces it, never interprets it.

### 3.3 Frontends

- **Python (blessed).** Modules are plain Python files using the typed
  `gripsack` Python package (dataclass/pydantic-style). Pyright gives
  autocomplete, inline errors, and refactoring for free. The core invokes the
  frontend as a subprocess (`python -m gripsack.frontend --emit-ir`) and
  validates the emitted JSON against the IR schema. No embedded interpreter,
  no PyO3, no Python-version coupling in the binary.
- **YAML (later).** For the common case — "fetch this release, symlink these
  configs" — a data-only frontend with zero logic.
- The IR is the contract, so third-party frontends are explicitly welcome.

### 3.4 Store

- Input-addressed: `/store/<input-hash>-<name>` where `input-hash` covers the
  resolved module plan (fetcher + pinned URL/rev + build recipe + dependency
  hashes).
- Store paths are immutable. Same resolved inputs → same store path, on any
  machine of the same platform → store sharing/syncing is a trivially
  addable feature later.
- No output-addressed/fixed-output machinery (a deliberate cut vs Nix).

### 3.5 Profiles and generations

- A **profile** is a forest of symlinks into the store: binaries under
  `current/bin`, configs deployed to their destinations.
- A **generation** is an immutable, complete snapshot of profile state.
  `current → generations/N` is flipped atomically after a successful
  operation. Rollback is re-flipping to any prior generation.
- The profile bin dir is a single stable PATH entry, so a generation flip
  takes effect instantly — no shell reload, ever.

### 3.6 Operations are incremental; generations are total

Every apply operation — one module, several modules, or the whole graph —
produces exactly one new generation:

```
grip apply neovim        # rebuild/redeploy only neovim; new generation = old
                         # profile with neovim's subtree replaced
grip apply               # entire graph; new generation
grip rollback [N]        # flip back; itself recorded
```

Generations are total (a full profile snapshot) while operations are
incremental (store dedup means an untouched module costs nothing — its store
paths are reused). This avoids the nix "edit one dotfile, wait for the world"
feeling: editing your neovim module and running `grip apply neovim` touches
exactly neovim's store paths and symlinks, and is still fully rollback-able.

### 3.7 Config ownership modes

Dotfiles are where gripsack goes beyond stow (too dumb) and home-manager
(too rigid). Per config file, the module picks:

| mode | behavior |
|---|---|
| `owned` | store-owned symlink, read-only; edits go through the module |
| `tracked-copy` | copied from store; hash recorded; drift detected on next apply (`keep / adopt / restore`) — for apps that mutate their own configs |
| `merge` | managed block merged into a file other tools also write (`.bashrc`) |
| `template` | rendered at activation from module variables; each generation's manifest records the resolved vars, so rollback re-renders exactly what generation N had (amended in 0.16.1: vars-in-manifest replaces the store-keyed rendering originally described here — same guarantee, one fewer store artifact) |

Read-only-symlink-everything breaks real apps; `tracked-copy` exists from day
one for that reason.

### 3.8 Activation

- **Intents**: `service(name, user=True)`, `fonts(...)`,
  `desktop_entry(...)`, … translated by adapter traits. v1 ships
  `SystemdUser` and `Noop`; runit/launchd are community-contributed adapters.
- **Trigger points**: `post-link` (per module), `post-activate` (per
  generation), `on-remove`. All recorded in the IR, all shown by `plan`.
- **Failure semantics**: pre-flip failure → no new generation, nothing
  happened. Post-flip failure → generation marked `degraded`, surfaced by
  `grip status`. **Never auto-rollback**: a failed `fc-cache` must not bounce
  the whole system back a generation. Rollback is a user decision.

### 3.9 Garbage collection

Generations pin store paths. Anything unreferenced — including ephemeral
build-only modules — is collectable via `grip gc`.

## 4. Execution model

```
eval      python frontend evaluates hosts/<host>.py + modules → IR (JSON)
resolve   unpinned refs resolved, content hashes pinned → locks/<host>.lock
plan      diff resolved graph vs current generation → show fetch/build/link/
          activate steps (no side effects)
execute   DAG scheduler (tokio): fetch → build → install into store paths
activate  run post-link intents, flip current symlink, post-activate intents
record    generation registered (modules, store paths, hook results, status)
```

- Evaluation is the only place host facts and tags are consulted.
- The lockfile freezes eval output per host: re-runs are reproducible, and
  `plan` on machine B shows exactly what will change before anything moves.

## 5. Hosts, facts, and tags

The user's env repo:

```
myenv/
  env.toml            # tool version, external module sources (git URL + rev)
  modules/            # helix.py, neovim.py, rust.py, ...
  hosts/
    laptop.py         # tags = ["gui", "work"], module selection
    desktop.py        # tags = ["gui", "nvidia"]
  locks/
    laptop.lock
    desktop.lock
```

- **Facts** are auto-detected: os, arch, distro, libc, has-gui, gpu.
- **Tags** are user-declared per host entrypoint.
- **Conditionals** are eval-time predicates: `module("steam", when=f.has_gui)`
  or conditional sources/config inside a module.
- Host selection: `--host` flag, default from hostname.

### Platform-conditional sources (fedora vs ubuntu)

A module may select its source by platform, resolved at eval time and frozen
in the host's lockfile:

```
source = when(f.distro == "fedora", rpm_or_copr(...),
              f.distro == "ubuntu", apt_or_ppa(...),
              github_release(...))   # fallback
```

The core still sees one resolved source per host — no runtime branching. v1
ships `github_release`, `tarball`, `git`, `custom_shell`; `apt`/`dnf`
extraction fetchers are a later addition (debs/rpms assume FHS layout and
maintainer scripts; not a v1 fight).

## 6. Components

| component | lang | role |
|---|---|---|
| `grip` CLI | Rust | single static binary; git built in (git2) for `--repo` bootstrap |
| core: IR schema + validation | Rust | the contract; versioned |
| core: store + generations + GC | Rust | content layout, atomic flips, pin/ref tracking |
| core: DAG executor | Rust (tokio) | scheduling fetch/build/install, step caching |
| core: fetchers / builders | Rust | pluggable verb implementations |
| core: activation adapters | Rust | intent → platform action |
| `gripsack` python package | Python | typed module DSL; emits IR |

CLI surface (sketch): `apply [--host H] [MODULE...]`, `plan`, `rollback [N]`,
`generations`, `status`, `why-owns <path>`, `gc`, `diff N M`.

## 7. Trust and security

- Module code is **trusted code** (the PKGBUILD/ebuild model): you run code
  from module authors. Personal-manager scope, not a build farm.
- Fetched content is verified against lockfile hashes.
- Secrets: not in v1, but the config schema must not preclude them —
  age/sops-encrypted files decrypted at activation, never stored in
  plaintext in the store.

## 8. Non-goals (v1)

- Root/system scope, daemons, multi-user.
- Build sandboxing or purity enforcement.
- apt/dnf extraction fetchers.
- Module registry (module sharing = git URL + rev in `env.toml`).
- `merge` config mode, secrets, store sync between machines.
- Windows.

## 9. Invariants (hold these and everything else follows)

1. Store paths and generations are immutable once created.
2. Activation of a generation is a single atomic rename.
3. The core executes IR verbatim; all conditional logic lives in eval.
4. Every mutating operation produces exactly one generation.
5. Post-activation failure never triggers automatic rollback.
6. The lockfile is the sole source of resolution; re-eval only changes the
   plan when the user asks (`grip update`).

## 10. Open questions (next docs)

- 0002: IR schema (exact shape of modules, sources, ownership, intents).
- 0003: Python frontend API surface (what writing `helix.py` feels like).
- 0004: generation/activation lifecycle in detail (ordering, degraded states,
  GC pinning rules).
- Secrets design; store sync/cache protocol; system-scope extension.
- IR tooling (separate repos under gripsack-dev, later): schema validator,
  IR linter, provenance-aware error reporter.
