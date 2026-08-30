# Changelog

User-visible changes per release. Design archaeology lives in
`plan/`; this file is for "what's new for me".

## [0.17.3] — 2026-08-30

Platform facts, floating git, and a hardened store
([plan 0016](plan/0016-platform-facts-floating-git-readonly-store.md)).

### Added

- **Platform placeholders in fetch specs**: `{system}` (flake-style
  `x86_64-linux`), `{target}` (rust triple, musl on linux), `{arch}`,
  `{arch.go}` (goreleaser `amd64`), `{os}` — expanded by the core from
  the machine's facts in asset patterns, tarball URLs, and install and
  verify keys. One module now serves every platform; per-host locks
  keep every machine honest.
- **Floating git fetcher**: `git(url)` without a rev resolves the
  remote's default-branch HEAD at lock time, pins the sha into the
  lockfile, and `grip update` moves it — the same float-and-pin
  semantics every other fetcher already had. Inline revs still pin.
- **Read-only store payloads**: files publish with write bits dropped
  — an app rewriting an `owned` config through its symlink gets
  EACCES instead of silently corrupting the store. Directories stay
  writable, so repair/gc/rollback are unaffected.
- Linter packs: yazi theme sections ([app], [indicator], [cmp],
  [spot], [icon], [flavor], …), helix `completion-timeout` +
  `[editor.whitespace]` + `[editor.lsp]`, starship `$schema`, atuin
  `[daemon]` keys — all sourced from current upstream docs.

### Fixed

- Linter diagnostics for unknown sub-tables named the section and
  pointed at the right header line (they said "unknown key" at line 1).
- Version-skew warnings (W10) no longer fire on current versions —
  host lockfiles pin tag-style versions (`v18.20.1`) while packs
  carried bare prefixes (`18.`); packs now carry both forms.
- atuin `search.shells` accepts the documented string default (was a
  live false-positive A04).

## [0.17.2] — 2026-08-29

Dogfood fixes — every one of them caught by running gripsack against a
real, full-sized dotfiles env.

### Fixed

- `store verify` no longer false-positives on `merge` and `template`
  modules: the manifest records the *deploy-output* hash (trimmed
  block, rendered bytes), and verify now recomputes exactly that
  instead of comparing the raw store file (which could never match).
- Package harvest copies symlinks **as symlinks** instead of following
  them — a symlink to a directory (conda's `lib/terminfo →
  share/terminfo`) crashed pixi-module fetches with a pathless io
  error. Note: pixi payload identity changes with this fix; a
  `grip update <module>` re-pins deliberately.
- API auth tokens now survive **same-host redirects** — a transferred
  GitHub repo (`owner/x` → `new-owner/x`) previously re-requested
  anonymously into the rate-limited pool and 403'd. Cross-host
  redirects still strip credentials (the no-leak rule stands).
- Fetch/copy io errors carry the offending path instead of a bare
  "the source path is neither a regular file…".

## [0.17.1] — 2026-08-29

The adopt audit ([plan 0015 §7](plan/0015-grip-adopt.md)): ask, don't
guess; say exactly what you wrote.

### Changed

- **`grip adopt` no longer guesses ownership.** The hardcoded
  app-behavior tables are deleted — they presented folk knowledge as
  detection. Adopt now *asks* with the semantics laid out (arrow-key
  select): `owned` (read-only link, repo is the only editor),
  `tracked_copy` (real file, drift kept — the safe default), `merge`
  (one managed block in a shared file). Non-interactive runs take
  `tracked_copy` with a loud note; `--mode` is the preseed.
- Adopt's plan labels destinations it will absorb as
  "will be adopted (prior recorded)" instead of demanding
  `--take-over`.

### Fixed

- Adopt no longer follows directory symlinks inside the adopted tree —
  a link could pull an arbitrary directory tree into your repo. Such
  links (and broken ones) are skipped and reported.
- Adopt refuses paths outside `$HOME` (plan compliance) and refuses to
  overwrite existing `modules/<name>.ts` / `configs/<name>/` — the
  never-clobber rule covers the repo too.
- Eval-failure and abort messages list exactly what was written and
  how to abandon it (the old message claimed the repo was untouched).
- Host-entrypoint edits are a pure, unit-tested function; a
  single-line `modules: [a]` insertion produced `[ab, ]` before.
- Large adoptions (>25MB) warn and name the largest entries.

### Refactored

- `commands/adopt.rs` (500 lines) split into
  `adopt/{mod,inspect,generate,prompt}.rs` — pure functions at the
  edges, side effects named. The ownership menu uses dialoguer.

## [0.17.0] — 2026-08-28

Constrained evaluation ([plan 0013](plan/0013-constrained-evaluation.md)).
Breaking: the Python frontend, bun, and uv are removed.

### Added

- **Sandboxed eval**: the TypeScript frontend runs under a pinned,
  hash-verified Deno (2.9.6) with deny-by-default capabilities — no
  environment variables, no network, no subprocesses, read-only within
  the repo. First eval downloads the runtime once; `GRIPSACK_DENO`
  overrides. Eval platforms: glibc Linux and macOS (Deno ships no musl
  build; `grip doctor` says so plainly there).
- **Injected facts**: os/arch/libc/hostname are detected in the Rust
  core and passed to eval in a JSON inputs document — no more
  frontend-side self-detection.
- **Probes**: `ctx.probe.executable("nvidia-smi")` /
  `ctx.probe.file_exists(...)` are symbolic requests the core binds in
  a two-stage eval (fixpoint-capped, E112 on instability), recorded in
  the run log and summarized in `grip plan`'s host-inputs header.
- **Trust gate**: the first eval of an unfamiliar repo prompts before
  running its code, naming the exact sandbox capabilities. `grip trust
  list/add/remove` manages the store; `GRIPSACK_TRUST_ALL=1` is the CI
  bypass.
- **`defineEnv`**: host entrypoints are functions —
  `export default defineEnv((ctx) => ({ tags, modules }) )`. `module()`
  is a pure constructor; falsy module entries drop out. Side-effect
  registration is gone.
- The dual-frontend parity corpus is replaced by a golden IR snapshot
  corpus (fixture envs → IR, byte-exact modulo spans).
- **Content-addressed store identity** ([plan 0014](plan/0014-content-addressed-fetches.md)):
  fetch-only and config-only modules name their store path by the
  canonical hash of their content; builds stay input-addressed (their
  output can't be named before it exists — plan-time naming is what
  keeps `grip plan` complete). A mirror swap or URL edit with identical
  bytes re-proves once, then dedups to the same path: no store churn,
  no new generation. Editing an install mapping no longer refetches.
  The lockfile records both hashes: `sha256` (transport integrity of
  the download) and `tree256` (store identity).
- **`grip store verify` is now correct and host-independent**: the
  whole-tree check previously compared a tree hash against the
  transport hash (could never match) under a hostname-keyed lock
  lookup (usually skipped) — the generation manifest now carries
  `tree256`, and a tampered fetched payload fails verify anywhere.
- **`grip adopt <path>`** ([plan 0015](plan/0015-grip-adopt.md)) — the
  adoption flow as a first-class command: inspects the path,
  recommends an ownership mode with the reason stated, generates the
  payload + module + host entry, shows the plan, and touches nothing
  until confirmed. The apply uses **scoped take-over** (absorbs
  exactly the adopted destinations; unrelated drift is never
  clobbered) and records **prior state** for every taken-over
  destination. Rollback — and undeclaring the module — restores your
  original files, bytes and permission bits, drift-guarded against
  your post-adopt edits. On a fresh machine, adopt records an empty
  **generation 0** first, so adoption is always reversible.
- Prior blobs live content-addressed under `$GRIPSACK_HOME/prior/`;
  `gc` collects them with the same reachability rule as store paths.
- Provisioning is serialized across concurrent `grip` runs — two
  racing applies could corrupt each other's Deno download or embedded
  frontend materialization (os error 26/2 under concurrency).
- Symlink deploys report "unchanged" when the link already points at
  the right store path — a re-proved fetch no longer looks like a
  redeploy.

  Migration note: content-addressed paths differ in shape, so the
  first apply after upgrading re-stages fetch/config modules once
  (identical bytes, new names); `grip gc` collects the old paths.

### Removed

- The Python frontend (`pip install gripsack`), the embedded
  zero-provisioning path, `GRIPSACK_PYTHON`, `[eval] deps`, and uv
  provisioning. `frontend = "python"` in env.toml is an E400 with a
  migration hint.
- bun provisioning and `GRIPSACK_BUN`.

## [0.16.4] — 2026-08-28

### Fixed

- `grip self-update` finds the binary inside the real release layout
  (`gripsack-<version>-<triple>/grip`, not a bare root) — 0.16.3's
  self-update reported "no grip binary" against actual releases; the
  e2e fixture now nests identically so the gate catches the shape.
  (0.16.3 self-updaters: run `curl -fsSL https://gripsack.dev/install.sh | sh` once.)

## [0.16.3] — 2026-08-28

### Added

- **`grip self-update`** — a package manager that can update itself.
  Tarball/install.sh installs fetch the newest `core-v` release, verify
  the mandatory sha256 sidecar, and atomically swap the running binary
  (takes effect next launch). brew/cargo/mise installs get their
  manager's command instead. `--check` reports only.
- This changelog, mirrored at
  [gripsack.dev/docs/changelog](https://gripsack.dev/docs/changelog.html).

## [0.16.2] — 2026-08-28

### Added

- **Zero-provisioning bootstrap**: the Python frontend is embedded in
  the binary. A config-only repo applies with zero network and zero
  provisioning, first apply included — `grip` + any `python3` is the
  whole requirement. Repos declaring `[eval] deps` or wheel linters
  still provision a pinned venv on demand.

### Fixed

- Repo-ref linters (`owner/repo@tag`) no longer force frontend
  provisioning; they resolve from the plugin store, not pip.
- `grip doctor` recognizes the embedded frontend.
- The crates.io publish loop publishes `griplint` before
  `gripsack-lint` (retries absorb index lag, not ordering).

## [0.16.1] — 2026-08-27

### Added

- **`grip store verify [--repair]`** — re-hashes every deployed entry
  against the manifest and every store payload against the lockfile's
  pins; `--repair` removes corrupt paths so the next apply re-fetches.
- The dual-frontend golden corpus: one full-surface fixture env
  evaluated through both frontends, IR diffed modulo spans — running in
  the required CI gate.

### Fixed

- Four parity bugs the corpus caught on debut: TypeScript dropped
  `brew(version=)`, env contributions were missing from the TS spec,
  host arch drifted (`x64` vs `x86_64`), and a missing `--host` file
  silently yielded empty tags instead of erroring. A fifth (musl libc
  detection) fell to the docker gate.
- Rollback of template/merge entries restores rendered bytes from the
  manifest's recorded vars; a foreign file's non-managed content
  survives.
- `keep_generations` resolves from the repo, never cwd-sniffing.

## [0.16.0] — 2026-08-27

### Added

- One deploy engine for apply and rollback: destinations are restored
  through the same code path either way.
- `grip plan` diffs against the live generation — modified, pruned, and
  take-over rows, not just the new manifest.

### Fixed

- Identity projection: spans and absolute paths no longer leak into the
  store-path hash, so two machines with identical repos get identical
  paths.
- IR rejects unknown fields; GC fails closed on corrupt manifests;
  `tracked_copy` drift is e2e-pinned.

## [0.15.5] — 2026-08-26

### Added

- Activation adapters: `fonts` (fontconfig cache) and `desktop_entry`
  (desktop database) intents run post-link/post-activate.

## [0.15.0] — 2026-08-25

### Added

- The griplint engine moved in-crate: all 22 config linters run
  in-process from embedded data packs — no venv, no provisioning for
  first-party linters.
- Plugin lifecycle: `package = "owner/repo@tag"` provisions fetcher and
  linter binaries from releases, sha256-verified and receipted.
- TypeScript frontend evaluates through a pinned, provisioned bun.

---

Earlier releases (≤ 0.14.x) predate this file — see
[the release history](https://github.com/gripsack-dev/gripsack/releases).
