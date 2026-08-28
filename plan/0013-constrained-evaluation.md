# 0013 — Constrained evaluation: one frontend, injected facts, explicit effects

- Status: draft
- Date: 2026-08-28
- Amends: 0001 §3.3 (frontends), §5 (facts), §7 (trust), 0002 §3
  (resolvers), 0005 §1/§3/§5 (runtime, eval order, protocol)

## 1. The problem

Two defects in the eval boundary, both visible in the shipped code:

1. **Invisible inputs.** Facts were self-detected inside the frontend
   process (`process.platform`, `process.report` for glibc,
   `existsSync("/lib/ld-musl-…")`), and any module could read
   `process.env`, the filesystem, or the network. Same repo + same
   lockfile + same declared host did not guarantee the same graph, and
   nothing could say why. The dual self-detection already produced five
   parity bugs the golden corpus caught on debut (0.16.1).
2. **Trust.** `grip apply --repo git@…` cloned and immediately executed
   arbitrary code with the user's full credentials. `grip check` — the
   command you point CI at — did the same.

The fix is not a purity language (Starlark/CUE/Nix — rejected; "no
custom language" is the differentiator). It is: **the config is normal
typed TypeScript; gripsack controls what that code can observe;
external effects are explicit, inspectable, and locked.**

## 2. Decisions

### D1 — TypeScript is the single frontend; Python is removed

One frontend, one eval, one language. The Python package, the embedded
frontend, the venv provisioning path, uv, and the dual-frontend parity
corpus are all deleted. The IR remains the contract, so a community
Python frontend is welcome out-of-tree — it is just not ours.

The honest cost: the zero-provisioning bootstrap (0.16.2) dies with it.
Deno cannot be a directory app under a system interpreter. Mitigation,
not retention: the runtime is provisioned exactly like uv/pixi/bun were
(D2), and the frontend *source* stays embedded in the binary (D3).

The parity corpus is replaced by a golden IR snapshot corpus: fixture
envs evaluated to IR, diffed byte-exact modulo spans. It keeps the
regression value; the cross-language value dies with the second
language.

### D2 — Deno, deny-by-default, provisioned not bundled

The frontend runtime is Deno, spawned as a subprocess (0001 §3.3 stands:
the core never embeds a runtime). Deno is chosen over bun for exactly
one reason: the permission model. The spawn contract:

```
deno run --no-remote --cached-only --no-lock \
    --allow-read=<repo>,<inputs dir>,<provisioned frontend> \
    <driver.ts> <repo> --inputs <path>
```

No `--allow-env`/`--allow-net`/`--allow-run`/`--allow-ffi`/`--allow-sys`:
denied by absence. Module code can read its own repo (tree payloads,
node_modules) and nothing else; every host observation arrives through
the inputs envelope (D4). Deno's default-denied npm lifecycle scripts
match the supply-chain posture.

Provisioning: `DENO_RELEASE` in `gripsack-fetch/src/host.rs`, same
`ToolRelease` pattern as uv/pixi — version + per-platform sha256 baked
into the source (never a fetched sidecar), downloaded into
`$GRIPSACK_HOME/tools/`, `GRIPSACK_DENO` escape hatch first, a deno on
PATH second, the pinned download last.

Not bundled into grip, deliberately: the crates.io 10MB crate limit
kills `cargo install`; the musl tarball would balloon 4×; Deno's CVE
cadence would become grip's. Embedding V8 (`deno_core`/`rusty_v8`) was
considered and rejected: `deno_core` ships no permission system (that
lives in `deno_cli`, not a library), musl-static V8 is not a boring
path, and the subprocess boundary is the architecture.

Platform support: eval runtimes exist for glibc Linux x86_64/aarch64
and macOS x86_64/aarch64. Deno ships no musl build; musl hosts get a
clear `grip doctor` error. The grip binary itself stays musl-static
(the release stage is unchanged); the e2e gate's *runtime* stage moves
to a glibc base — a static musl grip runs fine there.

### D3 — The frontend source stays embedded; only the runtime provisions

The driver and `@gripsack/core` ship inside the grip binary (the same
trick as the embedded Python frontend), materialized under
`$GRIPSACK_HOME/frontend/ts-<version>/`. The DSL version always matches
the core; a repo's own `node_modules/@gripsack/core` install still wins
when it shadows the embedded copy (the deliberate-pin rule is
unchanged). What downloads is Deno alone, once, hash-verified.

### D4 — Facts and inputs are core-injected, via file, never argv/env

The core detects facts in Rust — os, arch, libc (`ldd --version` for
the glibc version, the musl loader path as the musl tell, `darwin` on
macOS), hostname — merges tags, and writes one JSON document:

```json
{
  "version": 1,
  "host": "laptop",
  "facts": {"os": "linux", "arch": "x86_64", "libc": "glibc-2.36",
            "hostname": "box"},
  "tags": ["gui", "work"],
  "probes": {},
  "settings": {}
}
```

The path travels as `--inputs <path>`. Not argv (world-visible in
`ps`), not env (leaks to children). The frontend's `facts.ts` loses all
detection code; `HostFacts` becomes a parsed type, not a probe. This
deletes the dual self-detection bug class by construction: one
detector, in the core, feeding one frontend.

### D5 — The frontend returns a value; no registration by side effect

The frontend contract becomes a function:

```ts
// hosts/laptop.ts
import { defineEnv } from "@gripsack/core";
import { helix } from "../modules/helix.js";

export default defineEnv((ctx) => ({
  tags: ["gui", "work"],
  modules: [
    helix,
    ctx.facts.os === "linux" && steam,   // falsy entries drop out
    ctx.probe.executable("nvidia-smi") && cuda,
  ],
}));
```

`module(...)` constructs a value; `defineEnv` receives
`ctx = { facts, tags, probe, settings }` and returns the environment.
The driver imports the host entrypoint, calls the function, emits the
envelope. No global registry, no import-order magic. `Inputs →
Environment` is testable, cacheable, and is what makes two-stage eval
(D6) a re-invocation rather than a special mode.

### D6 — Effects are symbolic requests; the core binds them (two-stage eval)

Sandboxed eval *cannot* run probes (no `--allow-run`, no filesystem
beyond the repo) — so a probe call can only ever be a request. That
constraint is a feature: it forces the effects into the envelope.

```
eval₁:  ctx.probe.executable("nvidia-smi") records
        {"kind": "executable", "name": "nvidia-smi", "span": …} into
        the eval envelope's probe_requests and returns the bound value
        from inputs.probes (absent → false)
bind:   the core evaluates the closed enum of probe kinds
        (executable: PATH lookup; file_exists: absolute-path stat)
eval₂:  re-run with inputs.probes populated
fixpoint: if eval₂ requests new probes, iterate; cap 4 rounds, then
        E1xx "probe set unstable" — a probe depending on a probe is an
        authoring error, not a fixpoint
```

The IR that crosses into the core stays fully concrete — invariant
0001 §9.3 ("the core executes IR verbatim; all conditional logic lives
in eval") survives, because binding happens *before* emission, on the
frontend's side of the boundary, with core-supplied data. No
conditional nodes in the schema, no `ir_version` bump.

Probe results are recorded in the run log (0009 §2 rule 7) and
summarized by `grip plan` under a host-inputs header. Probe semantics
are deliberately *not* locked: they re-evaluate every run, so plugging
in a GPU changes the next plan with zero repo changes. That is the
honest behavior for hardware; the plan header is what keeps it from
reading as nondeterminism.

### D7 — First eval of an unfamiliar repo is an explicit trust decision

Before any eval, the core checks the repo against
`$GRIPSACK_HOME/trust.toml`:

```toml
[[repos]]
path = "/home/tarek/myenv"                 # canonical; the trust key
remote = "git@github.com:tarek/myenv"      # informational
commit = "54d91a1…"                        # recorded for audit
trusted_at = "2026-08-28T12:00:00Z"
```

Untrusted → interactive prompt naming the path, remote, commit, and
the exact capability set eval will get ("sandboxed TypeScript: no
environment variables, no network, no subprocesses, read-only within
the repo"); `y` records and proceeds. Non-TTY → hard error pointing at
`grip trust add <path>` (also `list` / `remove`). `GRIPSACK_TRUST_ALL=1`
is the documented CI escape hatch, same role as `GRIPSACK_DENO`.

Trust is keyed on the canonical path, **not** the commit: a per-commit
key re-prompts on every commit to your own dotfiles repo, which trains
users to bypass the gate. A moved or re-cloned repo re-prompts — that
is the case that matters (the `git safe.directory` precedent).

The gate wraps every command that evals: `apply`, `plan`, `check`,
`update`, and the `--repo` bootstrap (after clone, before first eval).
With D2's sandbox the residual risk of eval is small; the prompt is
what makes the user *aware* a repo is code before it runs.

### D8 — Resolvers become executables (specified here, built next)

0002 rung 2 ("ordinary credentialed Python at eval") is repealed —
Python eval no longer exists. Custom resolution becomes a third plugin
kind, `gripresolve-<name>`, on the 0009 envelope (NDJSON, structured
diagnostics, codespacing):

```toml
[resolvers.artifactory]
package = "gripresolve-artifactory==2.1.0"
env = ["ARTIFACTORY_TOKEN"]        # enforced: spawned env is scrubbed
network = ["artifacts.company.com"] # declared: shown in plan, not enforced
```

Env filtering is real enforcement (the process is spawned with exactly
the declared vars). Network scoping is **declared, not enforced** —
per-process egress control is not achievable user-scoped without a
localhost filtering proxy, and a manifest that pretends to be a sandbox
is security theatre. Honesty over decoration; the declaration still
buys plan-time visibility and a future enforcement point.

Built-in resolution (0002 §8) is unaffected: it already lives in the
core at lock/update time. `grip update --dry-run` (roadmap) folds into
this work — "resolve, don't write" is the natural read mode of a
resolve phase. This is the next doc's scope, not this one's.

## 3. What dies

- `python/` frontend, PyPI package, pyright story, `GRIPSACK_PYTHON`
- the embedded Python frontend and its build.rs embed
- uv provisioning, `[eval] deps`, `GRIPSACK_UV`
- bun provisioning, `GRIPSACK_BUN`, the bun `ToolRelease`
- the dual-frontend parity corpus → golden IR snapshot corpus
- side-effect module registration in the TS frontend
- frontend-side fact detection (`facts.ts` becomes pure data)

## 4. What is untouched

- The IR schema, `ir_version`, and the core's pass pipeline (0004 §4) —
  probes bind before emission; the core never sees a conditional
- `gripfetch-*` protocol and conformance — fetchers were already the
  right shape; `gripfetch-apt` needs no change
- Store, generations, ownership modes, activation, scheduling
- The musl-static release build

## 5. Sequencing

This doc's D1–D7 land together (one PR series, one gate): runtime
swap, envelope, facts, sandbox, defineEnv, probes, trust, deletions.
D8 is the follow-up doc.
