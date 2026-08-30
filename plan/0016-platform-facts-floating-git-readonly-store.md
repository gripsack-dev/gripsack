# 0016 — Platform-parameterized fetches, floating git, read-only store

- Status: draft
- Date: 2026-08-29
- Amends: 0002 §5 (fetcher specs gain platform placeholders), 0014 §5
  (store mutability), schema/ir/v1.json (git.rev optional)

## 1. D1 — `{system}` in fetch specs: the flake fact, our way

Module authors hardcode `x86_64-unknown-linux-musl` into asset
patterns today — the string only matches the machine it was written
on. The host's platform is already a fact (injected into eval as
`ctx.facts`, detected in the core at fetch — one source of truth by
construction). Fetch specs gain placeholders, expanded by the **core**
at resolve/fetch/verify time from the machine's facts:

| placeholder | linux x86_64 | linux aarch64 | macOS arm64 | macOS x86_64 |
|---|---|---|---|---|
| `{system}` (flake-style) | `x86_64-linux` | `aarch64-linux` | `aarch64-darwin` | `x86_64-darwin` |
| `{target}` (rust triple, musl preferred) | `x86_64-unknown-linux-musl` | `aarch64-unknown-linux-musl` | `aarch64-apple-darwin` | `x86_64-apple-darwin` |
| `{arch}` | `x86_64` | `aarch64` | `aarch64` | `x86_64` |
| `{arch.go}` (goreleaser/go) | `amd64` | `arm64` | `arm64` | `amd64` |
| `{os}` | `linux` | `linux` | `darwin` | `darwin` |

- Naming conventions are a swamp (`amd64` vs `x86_64` vs `x64`,
  `linux-amd64` vs full triples) — a small explicit set, not a
  pretend-universal one. `{system}` follows flakes; `{target}` follows
  rustup; `{arch.go}` follows goreleaser. That's the honest three.
- Expansion happens at resolve (asset URL → locked, per-host lockfile
  as always) and at deploy/verify (install/verify keys, same
  substitution as `{version}`).
- `github_release` locks record the EXPANDED asset url as always; a
  direct `tarball()` spec stays symbolic in the lock and expands per
  host at fetch — either way the content hash is the pin, and per-host
  locks (0001 §5) keep machine B honest with a different `{system}`.
- Content-addressed store identity (0014) is unaffected: the path
  names bytes, not patterns.

## 2. D2 — `git(url)` floats; the lockfile pins the rev

`git(url, rev)` required the rev inline — the one remaining inline pin
in a real env repo. API consistency with every other fetcher:

- `rev` becomes optional end to end (schema drops it from `required`;
  the model becomes `Option<String>`; the frontend accepts
  `git(url, rev?)`). Alpha license: we are not holding backward
  compatibility for its own sake; rev-present specs are unchanged in
  behavior.
- Rev absent: the core resolves the remote's default-branch HEAD
  (`git ls-remote <url> HEAD`) at lock/update time, pins the sha into
  the lockfile, and every apply fetches exactly that rev (TOFU +
  hash-drift rules unchanged — 0002 §3, invariant 6).
- `grip update <module>` re-resolves HEAD, same as every other
  fetcher's update semantics.

## 3. D3 — Read-only store payloads

The store is user-writable today: an app rewriting an `owned` config
writes through the symlink and silently corrupts a store path (verify
catches it after the fact, maybe). Nix's answer is a read-only store;
ours is the user-scoped version:

- `publish_dir` chmods every payload **file** `a-w` after staging
  (exec bits preserved: `mode & !0o222`). Directories stay writable —
  `store verify --repair` and `gc` unlink files, which needs a
  writable parent, not a writable file.
- Effect: an app writing through an owned symlink gets EACCES; the
  store hash still verifies; deliberate edits go through the repo, as
  intended.
- Nothing in the pipeline legitimately writes into a store path
  post-publish (builds run in staging; deploy reads). The change is
  one walk at publish plus e2e proving the EACCES and that
  repair/gc/rollback are unaffected.

## 3a. D4 — Path validation is a sema pass, not a language construct

Asked during review: should paths get a typed `path()` construct, and
must placeholders hydrate before validation?

**Validate the symbolic form; never hydrate to validate.** A typed
`path()` in the IR buys nothing — the IR is strings either way, and
frontend-side constructors duplicate a check every other frontend
would miss. The answer is a sema pass over the strings, with spans:

- `from` (payload-relative): reject absolute paths, `~`, `..`/`.`
  segments, empty segments, trailing slashes, backslashes.
- `to`: `~/…` or absolute only (E102's territory — extended with
  `~/../` escapes and bare-home rejections).
- `{placeholders}` validate as opaque single-segment atoms. This is
  sound by construction: platform placeholder values come from a
  static table (never contain separators), and `{version}` values are
  git refnames (git forbids `..`). Documented here, defended at
  expansion.
- Precedents: Bazel labels (strict segment validation at parse),
  chezmoi (targets must not escape home), zip-slip defenses in every
  archive extractor.

**Archive extraction audit result**: our tar crate (0.4.46) already
routes `Archive::unpack` through per-entry `unpack_in` (traversal-safe
— hostile entries are skipped), and zip 8.x sanitizes by construction.
Remaining hardening: a silent skip is a partial payload that fails
later with an obscure "no payload at …" — extraction now pre-scans
entry names and hard-errors on a traversal attempt, naming the entry.

## 4. Chores (same batch, no design content)

- **Linter audit against a real corpus**: the dotfiles env deploys
  configs for six packed tools (atuin, gh-dash, helix, starship,
  tuicr, yazi). Declare `lint:` on those modules, verify packs fire at
  `grip check` — including a deliberately-broken config producing a
  span-labeled diagnostic. (Linters are opt-in per module today; the
  showcase repo should opt in.)
- **Website linters page**: drop the help-wanted framing; the page is
  shipped/planned only.

## 5. Acceptance

- The dotfiles env's asset patterns use `{system}`/`{arch}`/`{arch.go}`
  and apply identically on linux-x86_64 (host run) — and the patterns
  demonstrably resolve per-fact (unit tests for the expansion table).
- `git(url)` without a rev: first apply pins HEAD's sha into the lock;
  second apply fetches the pinned rev, not HEAD (e2e with a local
  remote that gains a commit between applies).
- Write through an owned symlink → EACCES; `store verify` stays ok;
  `--repair`, `gc`, rollback all still work (e2e).
- `grip check` on the dotfiles repo lints all six declared tools; a
  deliberately broken key produces a span-labeled error (e2e or
  fixture run).
