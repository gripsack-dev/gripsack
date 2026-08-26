# 0012 — Linters in the core, plugins with a lifecycle, and the end of griplint-py

Status: **in progress** (moves 1 and 2 landed, 0.13.0 / 0.14.0)

## The problem

Two plugin kinds, two architectures:

- **Fetchers** (`gripfetch-*`) are language-free executables the Rust
  core spawns over NDJSON/stdio — conformance-gated, throttle-aware,
  provisionable.
- **Linters** (`griplint-*`) are Python packages the *frontend* drives
  at eval time, installed by `uv pip install` into the provisioned
  venv. They silently vanish under `GRIPSACK_PYTHON`, inherit every
  Python-distribution wound (creation caps, index shadowing,
  transitive resolution, proxy-filtered indexes), and can't run
  anywhere the venv story doesn't reach.

The asymmetry is the bug. A linter is an executable speaking a
protocol — the core should drive it, exactly like a fetcher.

## The shape

```text
move 1   the core drives linters (protocol host in Rust; Python
         linters work unchanged; zero behavior change)
move 2   grip manages the lifecycle of BOTH plugin kinds —
         declarative `package = "owner/repo@tag"`, versioned store,
         sha256 sidecars, receipts, lockfile pins (rootle 0010's
         surveyed design, fetch machinery reused)
move 3   crates/griplint — the engine as a crate linked INTO grip:
         first-party linters are data packs (versioned key tables) +
         format parsers, run in-process. No binary, no provisioning,
         no lifecycle. Exotic formats stay external plugins forever.
move 4   archive griplint-py. /simple freezes for back-compat.
         upstream-watch moves to gripsack CI scripts.
```

### Move 1 — core-driven lint (this release)

- `lint = "name"` travels in the IR (it was eval-time-only before —
  that was the frontend owning the drive). Module spans come along, so
  a label-less plugin diagnostic still points at the callsite.
- `crates/gripsack-lint` hosts the protocol: one-shot exchange,
  `{op: lint, paths, tool_version}`, death-is-never-silent (E02),
  spawn failure (E01), crash-class codes (E99/E02) reclassified to
  warning by the CORE — a plugin's self-report is never evidence.
- Registration semantics unchanged: `path` wins; `package` resolves
  the console script next to the frontend python (same venv rule,
  including the GRIPSACK_PYTHON bypass note). Codes unchanged:
  E501/E502/E503.
- `tool_version` from the host lockfile, paths from the module's
  config payload files — same as the frontend did.

### Move 2 — lifecycle (grip manages both kinds)

Declarative-first, rootle 0010's mechanics (surveyed: gh, krew, helm,
mise, asdf, cargo — the ecosystem converged):

```toml
[fetchers.artifactory]
package = "acme-corp/gripfetch-artifactory@1.4.0"

[linters.myfmt]
package = "acme-corp/griplint-myfmt@0.3.0"
```

- A plugin install IS a fetch: release tarball + sha256 sidecar
  (mandatory — a missing checksum is a failed install, not a warning)
  → versioned store dir `plugins/<name>/<version>/` + `current`
  symlink → receipt written LAST → lockfile pin. Never overwrite a
  running binary; trust notice on first install (names owner/repo).
- Bare `package = "artifactory"` resolves by repo-prefix convention
  (`gripsack-dev/gripfetch-artifactory`); no central index at
  single-digit plugin counts.
- External linter plugins are managed identically to fetcher plugins.

### Move 3 — the engine in the crate

First-party linters are ~95% data: a per-tool declaration is ~30
lines; the value is the versioned key tables. So: ONE engine
(`crates/griplint`), linked into grip, embedding parsers with span
tracking for toml/json/jsonc/yaml/ini/kdl/ron. Per-tool tables become
**data packs** (TOML data, versioned by tool version, in-repo). A new
structured-format linter is a data PR — no code, no package, no PyPI.

- The ported fixture corpus becomes golden tests in the crate.
- `griplint-conformance` stays the gate for EXTERNAL plugins,
  unchanged. The engine's crate tests reuse its fixtures.
- First-party lint needs no lifecycle at all (it ships with grip).

### Move 4 — the kill

griplint-py is the reference implementation until the engine hits
conformance parity, then it is **archived** — a real delete, not a
deprecation zombie. griplint-common is fully absorbed.
`gripsack.dev/simple` freezes (published packages stay immutable for
0.10–0.12-era users). The upstream-watch (table freshness vs tool
releases) moves to scripts + CI in the gripsack repo, filing the same
freshness issues against pack versions.

## Fetcher frontend sugar

Fetchers own their syntax: `gripfetch-apt` the repo may ship a thin
`apt(version=...)` helper per frontend. One rule — sugar emits the
SAME plugin IR as `plugin_fetch("apt", ...)`; no side channels, so
`grip check`, docs, and the lockfile see one truth. The core never
learns the word "apt".

## The reproducibility ceiling (verdict, from review)

Can gripsack keep the strong word "reproducible"? Decomposed:

- **Pre-built artifacts** (github_release, tarball, brew, pixi — the
  dominant case): YES, fully, today. Bytes are sha256-pinned in the
  lockfile, the store is content-addressed, deploy is deterministic.
  Same lockfile → same bytes → same machine state, bit-for-bit.
- **Resolution/eval**: the frontend is arbitrary host-reading Python,
  but its output is pinned by the lockfile; re-resolution is explicit
  (`grip update`). Pinned, not live-reproducible — the standard
  lockfile contract (uv.lock, package-lock), and honest.
- **`shell_step` builds**: the leak. Arbitrary shell observes ambient
  env, system toolchains, /etc, network, timestamps. Pinning inputs
  does not pin the build environment. Full hermeticity requires
  store-provided toolchains + a sandbox — the Nix architecture, which
  we deliberately reject (daemon, /nix, the language).

So: the strong claim holds where the bytes are pinned; it is an
INHERENT design boundary for shell builds — not a bug, the price of
"no daemon, no /nix". Two tightenings raise the bar without becoming
Nix, both opt-in and deferred to a later plan:

1. env-scrubbed build steps (declared `[eval] env` + minimal PATH, not
   the ambient environment) — kills the commonest nondeterminism;
   needs a compatibility story for steps that legitimately use system
   tools, so it is a declared step option, not a default flip.
2. `grip build --check` (rebuild and diff store hashes) — MEASURE
   reproducibility instead of claiming it, reproducible-builds.org
   style. The honesty feature: pure builds prove themselves; impure
   ones tell you exactly that.

Site wording (shipped): "pinned inputs, reproducible resolution and
deployment — not hermetic builds; that guarantee is Nix's, honestly
theirs."

## Progress UX (the snake)

Fetch/provision/install progress adopts the retro snake loader rootle
shipped (indicatif custom tick set, palette-green) with per-module
MultiProgress lines during parallel applies. Eye candy with a job:
every line names the module and its current verb.
