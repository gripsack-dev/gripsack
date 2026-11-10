# 0059 — The front door: lighter syntax, approval of what matters, a narrower product

Status: **accepted, sequenced after plan 0056 F2.** Source: the
[frontend and vision review](../docs/reviews/2026-11-frontend-and-vision-review.md)
(2026-11-09). Its measured findings were spot-checked before admission:
- `gripsack.lock` is a required approved root (`gripsack-store/src/source_bundle/policy.rs:11`);
- the `abi: "musl"` package example (`doc/packages.md:15`);
- `griplint` has no dependents.

## 1. Why

The core is now proved where users feel it: ownership, rollback and
recovery, with GC and the splice in progress. What stops someone using
gripsack is the front door:
- a one-dotfile workspace is 15 lines of ceremony;
- every `grip update` forces a second approval;
- running `printf` from a task needs a Conda solve for Bash;
- both documented package hello-worlds refuse on a stock glibc host.

The owner's verdict: "disciplined but so tedious".

## 2. Vision (admitted, owner to confirm wording)

Drop "Nix/devenv successor". Gripsack has no package set, has no builds
since 0.51, and runs on one platform. Its defensible column is:
- dotfile ownership modes with drift preservation;
- a plan you read;
- transactional apply with proved rollback.

**Pitch:** *Your dotfiles and the tools they configure, in one typed file:
read the plan, apply as a transaction, roll back in one command.* The
review's shorthand is "chezmoi with undo, and mise inside it".

**For:** developers keeping a few Linux/WSL machines in sync who won't
learn Nix. Secondary audiences: platform teams pushing a blessed toolset,
and coding agents configuring machines (sandboxed evaluation, exact-byte
approval, rollback).

**Product, in order:**
1. dotfiles;
2. user-space tools from releases and Conda;
3. tasks and timers that use them.

Nothing else ships in the binary.

## 3. Admitted work

Ordered. Each step is one or more PRs under the normal gates.

### Step 1: make the documented examples run (bug, S–M) — done on `fix/examples-run`
- Infer the ABI from the ELF, which the classifier already does, and stop authoring it. Admit static binaries on any host.
- Conda: treat the platform libraries (the CDT set, e.g. libX11) as host-provided by default and note it in the plan. `strict: true` restores the closed check.
- Fix `doc/packages.md` and `typescript/README.md`.
- Add an e2e gate that evaluates and applies every website example on a stock glibc image. This replaces prose that restates refusals (review #14).

**Landed as:**
- `admit_target` no longer compares a declared ABI with the host libc. ELF admission reads the ABI from `PT_INTERP`, checks a declared `abi` against it, and needs the host to have that loader only for a dynamic image. Static images run on any host.
- conda-forge's CDT sonames (X11 client libraries, OpenGL/EGL, ALSA; an explicit list in `consumer/admit/platform.rs`) are host-provided by default, noted at `build`/`apply` ("uses host libX11.so.6"). Plan cannot say it: runtime admission is deferred there, and the measured set exists only after materialization. `conda.environment({ strict: true })` restores the closed check. `strict` is an additive optional field on the v10 `conda_environment` source (no `ir_version` bump; an older core refuses it by name). It enters package identity, not the locked source, so switching it needs no re-solve. The CDT relaxation covers the prefix-wide platform check, not a command whose own ELF closure links a CDT library.
- Every complete workspace in the website docs and the READMEs is an `examples/workspaces/*` directory, bound by an `<!-- example: … -->` line that the website build and `e2e/test_documented_examples.py` check. The gate applies the dotfile and file-fetch examples, runs the release archive against a local GitHub-release fixture, and only evaluates/checks/plans the Conda, Pixi, Docker and systemd ones offline.
- Found on the way, not fixed: a dynamically linked glibc release binary that does not ship its own libc still needs `hostRuntime`, and with it a declared `abi: "gnu"`, because `libc.so.6` is outside a relocatable package's closure. Step 3's inference should cover it.

### Step 2: approval binds the evaluation closure (rework, M)
- The approved bundle becomes: the `gripsack.ts` import graph, the files its declarations name, and the SDK identity. A README edit no longer needs re-approval.
- **Pushback on the review:** the lockfile stays bound, because it decides which bytes run. The fix is that `grip update` re-approves the lock *it just wrote*, recording the delta in the approval. A lock that changed underneath you (`git pull`) still needs approval, with the diff shown.
- `env.toml` is validated on read. Only its capture-policy bytes stay approved.
- `grip check` shows the TypeScript diff since the last approval.

### Step 3: the syntax (rework, M; IR11)
- Handles instead of names: `pkg(name, source, { bin })` and `env(name, [pkgs])`.
- A profile's files are a map from destination to ownership mode: `link`, `copy`, `block`, `tree`.
- The default export may be one output, an array, or `workspace({...})`.
- `target`, `abi`, `layout` and `archiveRoot` are inferred at `update` and recorded in the lock.
- Gone: `defineWorkspace`, `provider`, `lit`, `.build()`, `identity`, `targetPlatform`.
- `check`, `hook`, `schedule` and `environment` become attributes (`verify:`, `onActivate:`, `schedules:`, `env:`), not catalog entries (review #10).
- Ship as IR11 with a written promise: SDK major = IR major, and one year of read compatibility. IR10 workspaces keep evaluating through a shim for that year.
- The review's §1.4 rewrites are the acceptance examples. Every repository example is rewritten and run by step 1's gate.
- `grip types` writes the SDK `.d.ts` under a capture-excluded path, so editor squiggles work before the first approval (review #11).

### Step 4: the host shell by default (rework, S)
- `` sh`…` `` runs `/bin/sh` with the environment's binaries on `PATH`, and the plan prints the interpreter. `sh.with(pkg)` pins one.
- Tasks are already unsandboxed on the live checkout, so the pinned-Bash prerequisite bought nothing.

### Step 5: cut the second products (cut, M total)
- **Remove** `grip package` with its OCI transport, Ed25519 trust and `sharing-reference.md`.
- **Remove** Pixi lock import, explicit wheels, `pluginFetch` and the `gripfetch-*` host, `griplint`, and `snapshot`, `diff-config` and `--saved-plan`.
- **Keep probes until step 6 is designed.** Host facts are how host-conditional templates will work (review #15), so verify what probes still carry before cutting.
- **Services:** cut the Docker Engine client and service state. If services are used in the owner's flow, keep a `service()` that generates a digest-pinned, loopback-only `compose.yaml` as a profile file, with `grip up`/`grip down` wrapping `docker compose`. Otherwise cut services outright. **Owner decides.**
- Git history keeps all of it. Each cut deletes code, docs, e2e, schema fields and IR types in one PR, and lands as a breaking IR11 change alongside step 3.

### Step 6: daily-use additions (add, S–M each)
- `grip status`: per destination, show deployed, drifted (preserved), foreign or pending-recovery. Make drift visible in `plan`; today a hand-edited copy shows as `(satisfied)` (review #6). Add `grip resolve <path> keep|adopt|restore` on top of the proved K1 decision.
- `grip hook <shell>` and `grip env <name>`, for direnv-style activation, with launchers in `~/.local/bin` by default (review #7).
- `--json` on every command, and `adopt` without a TTY, for the agent audience.
- Host-conditional template variables (review #15, first half).

## 4. Deferred, not admitted now

- **`grip add github:owner/repo` and a tool catalog** (#9). It's worth doing after step 3, once a `pkg` line is one line. The catalog is maintenance we don't want yet.
- **Conda via a provisioned solver binary** (#12). Large; revisit when the vendored solver next needs work.
- **aarch64** (#13). Revisit when the owner has an aarch64 machine in their own flow.
- **Encrypted secrets** (#15, second half).

## 5. Sequencing

- Plan 0056 F2 finishes first: GC (K4), splice (K5), journal, activation, apply, K6 and Verus retirement.
- Step 1 can land between F2 steps, because it's a bug fix.
- Steps 2 to 5 land together as 0.6x (IR11).
- Step 6 follows.
- The website's compare page stops leading with Nix in the same release as step 3.
