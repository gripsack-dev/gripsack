# Gripsack design review — November 2026

- Date: 2026-11-09
- Source reviewed: `foundation/release-0.52` at `5af45a6` (the 0.52.0 head)
- Method: read-only. Code, plans 0001/0002/0013/0055/0056/0057, `STATUS.md`,
  the website source (`gripsack-dev.github.io/doc/*`), `CHANGELOG.md`.
  Nothing was built or run; the installed `grip` on this host is 0.45.0 and
  was not used as evidence. Every claim below carries a `file:line` so it
  can be checked and, if wrong, struck.
- Stance: independent. Where this review disagrees with plan 0056 or the
  README it says so and why. The plans are treated as hypotheses, not facts.

Tags: **[bug]** bug or soundness issue · **[debt]** design debt ·
**[ux]** user experience · **[strategy]** direction.

---

## Executive summary

Gripsack has a real idea at its centre — *apply your home directory like a
database transaction: plan, journal, atomic flip, keep every generation,
roll back* — and the commit point in the code is genuinely sound: a single
fsynced rename, recovery classified by an exact 32-byte transaction id,
fail-closed on ambiguity (`gripsack-store/src/generations/current.rs:105-117`,
`gripsack-store/src/journal/recover.rs:108`). That is the wedge, and it is
rarer than it sounds: chezmoi, mise, devbox and pixi have none of it, and
home-manager has generations only for the Nix-built part of the world.

Everything else is out of proportion to that centre. The project is eleven
weeks old (first commit 2026-08-22), has shipped 40 minor releases and nine
breaking IR versions in that time, carries 101k lines of production Rust plus
28k lines of vendored Conda forks, 16.7k lines of plan prose, and three
verification stacks of which one (Lean, 805 lines of kernel) runs, one
(Verus, ~6.3k lines inside product crates) cannot currently build, and one
(TLA+/TLAPS, 12.8k lines) is optional and unrun. The user-visible promises
that matter most — *your edits are preserved*, *rollback reselects retained
bytes*, *evaluation is sandboxed* — are each partly false today, and none of
them is in the proved set.

### Top 10 findings, ranked by impact

1. **[strategy] The verification programme is pointed at the wrong end of
   the risk.** What is proved (34 theorems over ~500 lines: commit
   classification, GC prune/delete filters, journal publication order) is
   true and low-value; most of it restates the `match` it is about
   (`proofs/Kernel/Selection.lean:122-145`, `proofs/Kernel/Journal.lean`).
   What users actually get hurt by — ownership (K1), rollback fidelity (K2),
   the managed-block splice (K5), the TOCTOU window, the eval sandbox — is
   unproved, and F2+F3 are estimated at 11–14 weeks of solo work. Recommend
   re-ordering: K1/K5 first because they are where the open defects are,
   drop the ring-1 protocol proofs (P1–P4 under S1–S7) from the committed
   roadmap, and spend the saved time on the bugs in items 2–6. §3.

2. **[bug] Rollback does not "reselect retained bytes".** The `current`
   pointer is reselected, but every destination is *re-derived*: templates
   are re-rendered from the current source (`gripsack-exec/src/ops/plan/restore.rs:67-76`),
   modes are guessed when unrecorded (`restore.rs:80-94`), nothing checks the
   result against the target manifest's recorded hash, and an entry with
   "no safe restore plan" is a warning after which the flip still happens
   and the target manifest claims the entry deployed
   (`gripsack-exec/src/rollback.rs:305-313, 338-346`). The README sentence
   "nothing is rebuilt" (`README.md:31-33`) is false for templates and
   merges. §2.2.

3. **[bug] Owned links clobber user replacements, and any link into
   `$GRIPSACK_HOME` is treated as ours.** FND-1 is known (plan 0056 §5a);
   the second half is new: `ours` is "the symlink target starts with the
   gripsack home" (`gripsack-exec/src/ops/plan/link.rs:12-17`), so an
   unrecorded link the user made by hand is re-pointed without
   `--take-over`. The link path records no `Prior::File` for a replaced
   regular file that I could find, so there is nothing to restore from.
   §2.3.

4. **[bug] The re-check→write window is milliseconds and a loss inside it
   is silent.** Between `capture` (`gripsack-store/src/journal/mutation.rs:76-118`)
   and the effect (`gripsack-exec/src/ops/execute.rs:82,113,141,181`) sit the
   prior-blob write with its fsyncs and the journal entry's fsync+rename+fsync.
   The post-check only verifies *our* write landed; a write that races
   into the window is replaced and leaves no trace anywhere (the prior
   blob holds the *pre-race* bytes, pinned by the manifest at
   `gc.rs:131-139`, so the older content is recoverable but the racing
   edit is not). Plan 0019 calls this window "microseconds"; the code says
   "this is not CAS" (`mutation.rs:11-12`).
   The merge path diverges: it re-reads inside the effect, fails the
   post-check, and compensation keeps gripsack's block in the file with no
   generation recording it (`recover.rs:352`). §2.2.

5. **[bug] "Sandboxed Deno" is Deno permission flags plus a filesystem-only
   Landlock ruleset.** Landlock grants EXECUTE at `/` by design and has no
   network or IPC rules (`gripsack-process/src/confinement.rs:14-21`);
   `DBUS_SESSION_BUS_ADDRESS` and `XDG_RUNTIME_DIR` are inherited by the
   evaluator (`gripsack-process/src/environment.rs:176-203`). One Deno
   permission bug yields `systemd --user` and `/var/run/docker.sock`. The
   `file_exists` probe is a one-bit host-filesystem oracle with no per-round
   cap (`gripsack/src/commands/probe.rs:316-330`). The shared writable
   `deno-cache` is outside the approved bytes (`commands/prepared.rs:168-177`).
   §2.4.

6. **[bug] `gripfetch-*` plugins break "the lockfile is the sole source of
   pinning".** Verified by a same-channel `.sha256` sidecar
   (`gripsack-fetch/src/resolve.rs:95-125`; `host.rs:4-7` admits it
   "authenticates nothing"), keyed by mutable tag not recorded in
   `gripsack.lock`, one global `current` per plugin across repositories,
   run unconfined before the hash check, and an untagged `owner/repo` can
   never satisfy its receipt (`None == Some(..)`, `gripsack-fetch/src/plugins.rs:95-101`)
   so it re-downloads on every evaluation. §2.5.

7. **[debt] Three verification stacks and a 54k-line "exec" crate.**
   `gripsack-exec` holds ring 1 (apply/rollback/gc), ring 3 (Docker,
   systemd, sharing, Conda, embedded Deno sources) and ring 4 (semantic
   review, reports); ~45k of its 54k lines are not execution.
   `gripsack-policy` (Verus) sits *under* `ir`, `process`, `fetch`, `store`
   and `conda`, so retiring it touches every crate. Nine `*_model.rs`
   explorers (~4k lines) and 39 TLAPS modules remain; the 0056 "delete now"
   rows for `tlaps_*.py`/`verus_evidence.py` are not done. The kernel
   itself carries a `SelectionIdentity::Legacy` variant
   (`gripsack-kernel/src/selection.rs:57-97`). §2.8.

8. **[ux] The surface is three products wide for a one-person alpha.**
   Thirty-odd commands including an OCI bundle pipeline
   (`package pack|sign|push|pull`), three overlapping review views
   (`plan --json`, `diff`, `snapshot`+`diff-config`), Docker services,
   systemd schedules, and a Conda/Pixi/wheel subsystem larger than the
   fetch layer (10.7k core lines + 3.5k helper + 27.9k vendored). Meanwhile
   there is no `status`, no `remove`/`unadopt`, no drift `resolve`, two
   incompatible `--repo` semantics, uneven `--json`, and the trust prompt
   fires on *any* byte change in the repository because the approval digest
   covers the whole tree minus `.git` (`gripsack-store/src/source_bundle/policy.rs:475-482`)
   after copying up to 512 MiB per evaluation. §1.

9. **[ux] The DSL's typed-ness does not buy the thing TypeScript is for.**
   Every cross-reference is a string (`environment: "dev"`,
   `packages: ["formatter"]`, `schedules: ["nightly"]`,
   `packageCommand("formatter","format")`) checked at runtime in TS *and*
   again in Rust (three re-implementations of the same rules, no parity
   corpus). Required ceremony — `targetPlatform` on every package when only
   one platform exists, `producer: provider(x)` wrapping a single-variant
   enum, `exec(..).arg(lit("x")).build()` — makes a realistic workspace
   3–10× longer than mise/devbox/home-manager. Nine breaking IR versions in
   eight weeks mean every user `gripsack.ts` has broken roughly every
   minor. §1.2–1.3.

10. **[bug] Leaks and orphans the journal never sees.** Every `begin_run`
    creates `generations/.selections/<txid>/` and nothing ever removes one,
    including vacuous no-op applies (`gripsack-store/src/generations/selection.rs:33-59`;
    no release path; the inventory skips the directory at
    `generations/inventory.rs:76`). `.staging-N-<pid>` is cleared only by
    the same pid. A generation published but never flipped is listed as
    history and `rollback` defaults to "current−1 by number"
    (`gripsack/src/commands/rollback.rs:28-33`). Task mutation claims that
    survive a crash have no operator path to clear them
    (`gripsack-exec/src/workspace/consumer/task_locks.rs`). Services live
    outside generations entirely and are orphaned when a checkout moves
    (`gripsack-exec/src/services/state.rs:20-31`). §2.2, §2.6, §2.7.

---

## 1. Surface area

### 1.1 CLI

Source: `crates/gripsack/src/main.rs:37-213` (the `Command` enum), 8,471
lines across 46 files.

**Coherent core.** `init`, `check`, `plan`, `apply`, `generations`,
`rollback`, `update`, `gc`, `why-owns`, `store-verify`, `trust`, `doctor`.
This is the Nix-profile loop and it reads well. `plan` before `apply`, with
the trust gate on first contact, is the right shape.

**Overlapping or confusing.**

- Three plan/diff views: `plan` (live), `plan --json` (a different renderer
  via `review::plan`, `main.rs:447-455`), `diff proposed:<ir> current`
  (offline, `commands/review.rs:155-180`), plus `plan --ir` for frontend
  debugging. `snapshot` + `diff-config --format json|toml|bytes` add a
  fourth and fifth. One `plan` with `--json` and one `diff` with selectors
  would do; `diff-config` and `snapshot` should fold into `diff`. **[ux]**
- `--repo` means two things: `Option<String>` accepting a git URL on
  apply/check/plan/update/build/run/shell/task (`commands/repo.rs:1-33`),
  `PathBuf` defaulting to `.` and refusing URLs on up/down/services/diff/snapshot
  (`commands/services.rs:17,31,49,57`). `task` accepts a URL only for
  `task run` (`run/task_cli.rs:41-60`). **[ux]**
- `run --env <E>` vs `shell <ENV>`: flag in one, positional in the other
  (`commands/run.rs:34-35, 50`). **[ux]**
- Five "status" surfaces (`services status`, `jobs status`, `task status`,
  `trust list`, `hooks list`, `generations`) and no `grip status` — which
  plan 0001 §3.8 promised, together with a "degraded" generation state that
  the code never produces (no occurrence of `degraded` in `crates/`). **[ux]**
- `--json` on 13 commands, absent on apply/generations/gc/rollback/why-owns/
  store-verify/doctor/update/adopt/snapshot; `package` is JSON-only with no
  flag (`commands/package.rs:97`). `--timeout-seconds` defaults to 1800/180/60
  depending on the command; `apply` has none. `jobs --home` is the only
  home override anywhere (`commands/jobs.rs:16`). **[ux]**
- `hooks test --duplicate|--crash-after-start` is a developer fixture
  simulator in the user command list (`commands/hooks.rs:21-27`); the
  `GRIPSACK_CRASH_AFTER` / `GRIPSACK_FS_RECOVER_ONLY` crash switches ship in
  the production binary without a `cfg` gate (`gripsack-exec/src/util.rs:48-52`,
  `apply.rs:88`). **[debt]**
- `doctor` checks only Deno and the embedded-frontend pin
  (`commands/doctor.rs:9`): not store integrity, drift, trust, Docker,
  systemd, Landlock availability or leaked reservations. **[ux]**

**Missing for a real user.** `status` (what is deployed, drifted, pending,
degraded); `remove <profile>` / `unadopt`; a drift resolver
(`keep|adopt|restore` per file, promised in 0001 §3.7 and 0030 but the only
tool today is `apply --take-over`); `generations prune`/`gc --keep N`;
`trust add --yes` for CI (today you must copy two digests from
`trust inspect --json`, `commands/trust.rs:102-138`); shell completions; a
global `--quiet/--color`. **[ux]**

**Cut or merge.** `snapshot`, `diff-config` → `diff`. `hooks test` →
`cfg(test)`/e2e. `jobs` and `services` → subcommands of a `status`/`service`
pair or one `runtime` noun. `package *` (sharing) is a second product; see
§1.4. `__package-command`, `jobs dispatch`, `hooks fixture-*` are correctly
hidden.

### 1.2 TypeScript DSL (`typescript/src`, 3,856 lines; 50 values + ~75 types exported)

The three-axis file model — origin (`repoFile`/`repoTree`/`artifactFile`),
content (`identity`/`literalText`/`templateText`/`templateFile`),
destination (`symlinkTo`/`trackedCopyTo`/`managedBlock`) — is the best
idea in the DSL and the README table explains it well. The rest is weaker
than it should be for a project whose stated differentiator is "typed
TypeScript instead of a config language":

- **References are strings.** `environment: "development"`,
  `packages: ["formatter"]`, `schedules: ["nightly"]`, `task: "format"`,
  `packageCommand("formatter", "format")` (`examples/workspaces/*/gripsack.ts`;
  types at `typescript/src/workspace/ir.ts:394-450`). Passing the value
  (`environment: dev`) is a type error. Every name is written twice and a
  typo is a runtime E126, not a compile error. This is the single biggest
  reason the TS layer is not pulling its weight versus YAML. **[ux]**
- **Ceremony with no information.** `targetPlatform({os:"linux",arch:"x86_64",abi:"gnu"})`
  is required on every `pkg`/`environment` though only one platform is
  supported (`outputs.ts:63`); `producer: provider(x)` wraps a one-variant
  union (`ir.ts:200`); `exec(cmd).arg(lit("docs")).build()` requires
  `lit()` around every literal and an explicit `.build()`; `defineWorkspace`
  is the identity function (`outputs.ts:469`); schedule `scope` is always
  `"user"` (`outputs.ts:304`). **[ux]**
- **Four "vary by host" mechanisms** (`when`/`hasTag`, probes, `settings:
  Record<string, unknown>`, tags) that overlap; `probe.file_exists` is the
  only snake_case method; two-pass probe evaluation means any branch on a
  probe runs twice and sees `false` first. **[ux]**
- **Inconsistent casing:** `service` fields are wire snake_case
  (`depends_on`, `interval_seconds`) while `pkg` is camelCase
  (`hostRuntime`, `archiveRoot`); `githubRelease` takes `base_url`. **[ux]**
- **Overloaded words:** `lock` is a mutation lock, the lockfile, the
  `pixi.fromLock` argument and the `locks/` directory; `artifact` has four
  constructors; `input` (captured file) vs `Inputs` (the core→frontend
  envelope); `pin.ts`/`pin-selection.ts`/`pin-advanced.ts` are the driver's
  SDK-selection shim, shipped in the npm package as if public. **[ux]**
- **Leftovers:** `outputPath`/`WorkspaceProductionPath` from removed
  recipes (`commands.ts:57`, `graph.rs:244,256`), `fields.ts:1-6` documents
  the removed `module()` API, `schedule`'s doc says it is rejected until
  "E3" (`outputs.ts:290`). **[debt]**
- **Three hand-kept export lists** (`index.ts`, `pin.ts`, `workspace.ts`)
  with nothing enforcing they match. **[debt]**
- **Provenance via stack parsing.** `span` comes from a regex over
  `new Error().stack` (`span.ts:9-20`); some constructors throw if no span
  is captured (`outputs.ts:110,124,152`); `span?` is a public field on every
  Spec so users can forge it. A side table keyed by node path would be
  lighter and not need `deny_unknown_fields` on 17 schema definitions. **[debt]**

A realistic "dotfiles + 5 tools + postgres" is 75–90 lines of TypeScript
and ~20 concepts (output, profile, environment, pkg, producer/provider,
fetch kind, target, layout, commands map, three file axes, three ownership
modes, service+health, lock, trust approval, generations). mise does the
tools part in ~6 lines, devbox ~10 with a postgres plugin, home-manager
~20. Gripsack is paying for safety it has not yet made easy.

### 1.3 IR (`schema/ir/v10.json`, 2,179 lines, 45 `$defs`)

Shape is fine: `{ir_version, host, workspace}` with eight output kinds.
Problems are stability and duplication, not design:

- Nine breaking IR versions between 0.35 (2026-09-06) and 0.51 (2026-10-31).
  0.50 *promoted* BuildKit builds and OCI image exports; 0.51 removed them.
  The SDK README's advice is "use matching core and SDK versions"; the
  effective contract is one exact SDK per core, and a repo-local pin works
  only inside one IR version (`pin-selection.ts:24-44`, driver checks only
  `IR_VERSION === 10`). **[strategy]**
- Validation is implemented three times — TS `validate.ts` (608) +
  `emit.ts` + per-output rules, the JSON Schema, and Rust `sema/*`
  (`values.rs` 611, `graph.rs` 369, `service.rs` 167 …) — with duplicated
  constants (image regex `services.ts:255` vs `service.rs:76-95`; 128-entry
  cap `services.ts:224` vs `service.rs:9`) and different messages for the
  same rule. The TS↔Rust parity corpus no longer exists
  (`e2e/test_contract_fidelity.py:3`). Rust as the authority is right; the
  TS layer should be a thin pre-check generated from the same source, or
  dropped in favour of returning Rust diagnostics with spans. **[debt]**
- Stale text: the description still names "recipes … images" (`v10.json:5`),
  `hostFacts.libc` mentions `darwin`, `hostFacts` has no required fields,
  `workspaceProductionPath` survives (`v10.json:836`), and
  `crates/gripsack-ir/schema-v10.json` is a byte copy of `schema/ir/v10.json`
  that can drift (v9 is a symlink). **[debt]**
- The lockfile (`lock_version: 3`, pretty JSON, sorted) is deterministic but
  inlines whole Conda/Pixi closures (`workspace_model/lock/conda.rs:17-60`),
  so a modest Python env is thousands of diff lines, and `pixi.fromLock`
  stores the same solution twice. Including the frontend digest in the lock
  means an SDK upgrade changes the lock *and* requires trust re-approval
  (`typescript/README.md:39-41`). **[ux]**
- Diagnostics: codes are generated from one registry (good), but almost
  every authoring error collapses to E130 (`diagnostic.ts:133-155`), there
  is no per-code documentation and no `--explain`. **[ux]**

### 1.4 Comparison and the wedge

| | Nix/home-manager | chezmoi | mise | devbox | pixi | gripsack |
|---|---|---|---|---|---|---|
| Generations + rollback of *home files* | partial (Nix-built only) | no | no | no | no | **yes** |
| Preserve user edits per file (modes) | no (read-only links) | templates/`modify_` scripts | n/a | n/a | n/a | **yes (with the bugs in §2.3)** |
| Plan before apply | `nix build` dry-run | `chezmoi diff` | no | no | no | **yes** |
| Tools from arbitrary archives | via derivations | no | yes (backends) | nixpkgs | conda | yes |
| Config language | Nix | templates/YAML | TOML | JSON | TOML | TypeScript |
| Services | NixOS/system | no | no | plugins | no | Docker |
| Ecosystem size | enormous | large | large | medium | medium | **one person** |

The honest wedge is: **transactional dotfiles and pinned user-space
binaries with real rollback, no daemon, no Nix language.** That is a
defensible niche — chezmoi users who want undo, mise users who want their
dotfiles in the same file. Services, OCI sharing, Pixi import and wheels do
not strengthen it; they compete with docker-compose, ORAS and pixi itself
on their home turf, each with a one-person maintenance tail (28k vendored
lines, a separately locked 651-crate helper, a hand-rolled OCI client).
**[strategy]**: freeze services/sharing/Conda at "works, documented as
experimental, no new features" and spend the next quarter making the wedge
correct (§2.2–2.3) and short (§1.2).

---

## 2. Core design

### 2.1 Store and generations

Input-addressed store, immutable generations, `current → generations/N`
flipped by `symlink_replace` (temp link, rename, parent fsync;
`gripsack-fs/src/lib.rs:139-159`). Generation allocation moves a durable
high-water mark before publishing (`generations/publication.rs:42-110`),
publication stages under `.staging-N-<pid>` and renames. All good.

Issues:

- `.selections/<txid>/` reservations are created per run
  (`generations/selection.rs:33-59`) and never released. There is no
  removal function in `selection.rs`, `end_run` (`journal/marker.rs:204-240`)
  does not touch them, GC skips the directory (`generations/inventory.rs:76`).
  Vacuous applies — the `return Ok(None)` at `apply.rs:407-413` — still
  reserve one. Unbounded, slow leak. **[bug]**
- `.staging-N-<pid>` from a crashed apply is cleared only by the same pid
  (`publication.rs:129-148`). **[bug]**
- A generation published but never flipped (crash between
  `publish_generation` and `flip`) is listed as history and is what
  `rollback` with no argument targets (`commands/rollback.rs:28-33`:
  current−1 by number, not "previously selected"). Rollback should default
  to the last *selected* generation and orphans should be marked or
  collected. **[bug]**
- GC root admission is component-wise (`paths.rs:27-46`) but GC compares raw
  strings (`gc/inventory.rs:43-52`); a hand-edited manifest with `/h/./store/x`
  passes admission and its object is deleted. Latent. **[bug]**
- Legacy readers on the hot path: relative/absolute legacy `current`
  resolved via `canonicalize` (`current.rs:33-66`), `MarkerFormat::LegacyGeneration`,
  `PendingPointer::Legacy`, `Wire::Legacy` manifests, mode-less template
  receipts, legacy `env/` cleanup on every rollback (FND-2). For an alpha
  with "clean cutover" in 0.50 (plan 0055), these should be a one-shot
  `grip migrate` or deleted. **[debt]**

### 2.2 Transaction and journal

The sequence (single `apply.flock`, `util.rs:21-35`; recover → allocate →
`begin_run` marker → per-destination plan → re-check + prior blob + journal
entry → effect → post-check → prune → publish → activation record → flip →
`commit_run` → hooks) is well-ordered and the two-barrier journal
publication is correct. The weak points are at the edges:

- **The window** (finding 4). Acknowledged in code and, in one sentence,
  on the guarantees page; not in plan 0019, which still says
  "microseconds". Two cheap mitigations that are not CAS: move the prior
  blob fsyncs and the journal entry publication *before* the final
  re-observation so the window is one `stat`+`rename`; and surface a
  post-apply note when the post-check sees a different *mtime/inode* than
  the capture did even though bytes match. The merge path's
  compensation-Keep-without-generation (`recover.rs:352`) should re-plan
  rather than keep. **[bug]**
- **Rollback re-derives** (finding 2). Fix: restore from the target
  generation's *recorded* bytes (store path + recorded hash + recorded
  mode), compare to `entry.hash`, and refuse to flip if any entry could not
  be restored unless `--force`. Templates should store rendered bytes per
  generation (0001 §3.7 originally said so; 0.16.1 replaced it with
  vars-in-manifest, which is what makes re-rendering drift possible). **[bug]**
- Post-flip hook failure returns success with a warning
  (`activate.rs:44-50`, `apply.rs:268-277`) and nothing marks the generation
  degraded; interrupted `Started` attempts replay (at-least-once) for
  arbitrary user hooks (`activation/ledger.rs:358-393`). Hooks run with the
  global lock held, so a hook that calls `grip` deadlocks until timeout. **[bug]**
- The lock is per `GRIPSACK_HOME`; two homes writing the same `$HOME`
  destination are not serialised. Acceptable, but document it. **[ux]**
- Unjournaled durable effects: destination parent `create_dir_all`
  (`deploy/mod.rs:124`), staging `remove_dir_all` (`publication.rs:144`),
  FND-2 (`env.rs:39-40`), store repair `remove_dir_all` (`verify_store.rs:288`),
  orphan activation directories if `prepare` fails after `create_dir`
  (`ledger.rs:129`). K2's "listed, not hidden" clause in 0056 is the right
  response; list them in code, not only in the plan. **[debt]**

### 2.3 Ownership and drift

Modes: `Owned` (symlink), `TrackedCopy`, `Template`, `Merge`
(managed block). The copy/template three-way rule (`plan_copy`,
`gripsack-policy/src/ownership.rs:37-87`) and `preserved_drift` recording
(`ops/plan/write.rs:144-157`) are the right design and the part of the
product worth protecting.

- FND-1 plus the `ours` rule (finding 3). Fix in `plan_link`: `recorded`
  must mean "live object *is* the recorded link", not "a record exists"; an
  unrecorded link into the home is foreign until `--take-over`. **[bug]**
- Merge: edits *inside* a block are overwritten with a report note only
  (`managed_blocks/mod.rs:162-175`); duplicate blocks are deleted. Inside
  bytes should be `Preserve`d like a tracked copy, or the note should be a
  refusal. **[bug]**
- Rollback `continue`s silently on `preserved_drift`
  (`rollback.rs:282,304,332`) — correct behaviour, but no note, so the user
  does not learn the generation is partial. **[ux]**
- Five observers of the same destination (`deploy::observe`,
  `observe_readonly`, `journal::live_identity`, the inline read in
  `capture`, `capture_prior`), two `Prior` types, two `dest_capability`
  functions, intactness logic in three places (`deploy/remove.rs:15-77`,
  `ops/plan/remove.rs:57-116`, `deploy/restore.rs:96`). This is exactly the
  "decisions, observation and effects interleaved" problem 0056 §1 names,
  and it is why K1 is the right first theorem. **[debt]**
- Case-fold (FND-3): admission folds (`deploy/destinations.rs:26`), prune
  and journal keys do not (`apply.rs:443-486`, `journal/mod.rs:188-195`).
  S6 says the kernel's key equivalence must be no coarser than the fs's;
  today it is *finer* on WSL `/mnt/c`, which is the dangerous direction. **[bug]**

### 2.4 Evaluation boundary and trust

What is good: Deno pinned by compiled-in sha256 (`gripsack-fetch/src/host.rs:101-108`),
`--no-remote --cached-only --no-lock --no-config`, no `--allow-net/env/run/ffi/sys`
(`gripsack/src/commands/frontend.rs:132-145`), facts via a private inputs
file, Landlock with `PR_SET_NO_NEW_PRIVS` that fails closed on hosts without
it (`confinement.rs:23-25,128-130`), approval bound to bundle digest +
policy digest + runtime identity (`prepared.rs:182-205`).

What is not (finding 5):

- Landlock mediates reads only. Network, exec and IPC are Deno's job. The
  threat model in 0013 §2 says the sandbox exists because "module code is
  arbitrary"; a single layer of a JavaScript runtime's permission checks is
  not what "sandboxed" means to a reader of the README. Add Landlock
  network rules (ABI 4+), drop `DBUS_SESSION_BUS_ADDRESS`/`XDG_RUNTIME_DIR`
  for the evaluator, and consider `seccomp` for `connect`/`execve`. **[bug]**
- `file_exists` is uncapped per round (`probe.rs:300-374`): bound it. **[bug]**
- The approval unit is the whole repository. Any edit — a README, a
  `target/` directory — re-prompts, and `SourceBundle::capture` copies up
  to 512 MiB first (`source_bundle/inventory.rs:13-19`). Approve the
  *evaluation closure* (the `gripsack.ts` import graph plus declared
  `inputFile`s and the SDK) instead; that is what actually runs. **[ux]**
- Revocation is by canonical path (`trust.rs:201-223`); moving a checkout
  orphans the approval and the inventory under `inventories/`. **[ux]**
- `saved_plan`'s `existing_deno` falls back to any PATH deno with no
  version check (`frontend.rs:56-77`). **[bug]**
- Plan 0013 says the Python frontend was removed to have one eval language;
  `REQUIRED_ROOTS` still includes `env.toml` (`source_bundle/policy.rs:11`)
  and `gripsack-config` parses it, after STATUS records a TOML frontend
  "rejected five times". Either it is configuration (fine, say so) or it is
  a second authoring surface. **[debt]**

### 2.5 Sourcing and fetchers

The archive layer is the best-engineered part of the edge: hash before
extract (`tarball.rs:14-23`), path and link-graph checks independent of
member order with a 64-hop cap (`archive/paths.rs`, `archive/links.rs`),
separate passes for files/hardlinks/symlinks, setuid stripped, O_NOFOLLOW
root. Keep it; it is a fuzz target already.

- Plugins (finding 6). Fix: record `{repo, tag, resolved asset sha256}` in
  `gripsack.lock` like any other source; store per-repository or by digest
  rather than one global `current`; confine with the same Landlock profile
  as the evaluator; fix the `None == Some` receipt check. **[bug]**
- `git` shells out to the host git with the user's config, `insteadOf`
  rules and credential helpers (`git.rs:76-84`). Document it; it conflicts
  with "the core never sees credentials" as a reader would understand it. **[ux]**
- Rate limiting is ~2,056 lines including a Verus-verified 1,152-bit
  bignum (`gripsack-policy/src/rate_limit/credit_balance.rs:1-20`) for one
  built-in budget of `api.github.com 30/min` (`throttle.rs:22-25`). 0056
  already says "no f64 in the kernel, plain Rust + property tests"; do it
  now, with integer tokens per minute (~100 lines). **[debt]**
- The resolver/transport split of 0002 is sound and still holds. The
  Conda helper is a third kind — a *materializer* — that runs unconfined,
  parses untrusted repodata and archives, and is pinned to gripsack's own
  release (`conda_pins.rs`). Confine it like a plugin. **[bug]**

### 2.6 Services

Docker Engine over the Unix socket via `reqwest` (228 lines, `services/engine.rs`),
digest-pinned image, loopback-only ports, mandatory health check, labelled
resources, foreign-resource refusal, `down` keeps volumes. Reasonable.

- Services are outside generations: rollback never touches them, and after
  a rollback task readiness fails with "declaration differs from its
  running revision" until a manual `down`/`up` (`services/mod.rs:441-445`).
  Either put the service *declaration* into the generation (so `up` after
  rollback is one command) or say plainly in the README that services are
  not rolled back. **[ux]**
- Identity is `sha256(home, canonical repo path)` (`state.rs:20-31`): a
  moved checkout orphans running containers. **[bug]**
- `RestartPolicy: "no"` (`plan.rs:170-174`): nothing survives a reboot;
  `grip up` holds the global apply lock through image pull and health wait
  (`util.rs:29-30`). **[ux]**
- Inherited env values are visible via `docker inspect` and the revision
  hashes only the variable *name* (`plan.rs:76-91`), so a rotated secret is
  not detected. Pull progress is buffered under a 4 MiB cap
  (`engine.rs:154-185`), which large images may exceed (unverified). No
  registry auth header, so private images do not work. **[bug]**

### 2.7 Schedules and tasks

Unit files are generation-owned files in `~/.config/systemd/user`
(`schedules/lifecycle.rs:104-117`), a static `grip` copy is retained in the
store as the runner, `dispatch` takes the lifecycle lock and runs the
*captured* IR revision — good — but against the **live checkout** as cwd
(`capture.rs:156-171`, `consumer/mod.rs:6`: "not a hermetic sandbox"). So a
"captured revision" pins the tools and the IR but not the scripts or data
the task reads. Say so in the schedules guide. **[ux]**

Task locks persist a claim across crashes by design (`task_locks.rs:1-4`)
and report `LockConflict::Unknown` (exit 70) when the flock is free but
the claim exists; no CLI path clears one. **[bug]**

### 2.8 Crate boundaries and the 0056 rings

Dependency graph today (from `Cargo.toml`s): kernel ← store ← exec ← cli;
policy ← {ir, process, store, fetch, conda, exec}; fs ← {store, trace,
fetch, conda, exec}; `gripsack-trace` declares an unused dependency on
`gripsack-store` (`gripsack-trace/Cargo.toml:11`); `gripsack-store` depends
on `gripsack-process` only for `Sha256Digest` and receipts
(`store/src/activation/model.rs:7`). `check_architecture.py` allow-lists
four crates and does not enforce direction.

Against the rings:

- **Ring 0** is 785 lines; ownership, splice, activation intent, cleanup
  ordering and the planner are still in `gripsack-policy` (Verus) and
  `exec/ops/plan/*`. The kernel's `plan_prune` result is edited afterwards
  by `gc/directory_roots.rs:26-57` (safe direction, but a decision outside
  the kernel). `admit_mutation` cannot fail in practice because the caller
  sets both roles one line earlier (`mutation.rs:200`); `RecordPublication`
  re-checks an order the `gripsack-fs` typestate already enforces
  (`journal/storage.rs:52-64`). These two kernel modules are ceremony. **[debt]**
- **Ring 1** does not exist as code: the planner does IO
  (`ops/plan/restore.rs:48,63,86,104`, `ops/plan/remove.rs:13,21`), and
  there is no `Storage` trait, `MemFs` or `GRIPSACK_ENGINE` switch yet. This
  is F2 step 1 and it is the right first step. **[debt]**
- **Ring 2/3 mixing:** `gripsack-store` holds ring-1 protocol
  (`journal/recover.rs`, the 2.4k-line directory domain) and ring-2 codecs
  (`*_wire.rs`); `gripsack-exec` holds rings 1, 3 and 4 (table in finding 7)
  plus a second HTTP stack (`reqwest`) next to fetch's `ureq`, duplicate
  archive codecs, and ed25519 trust. `embedded_frontend.rs` (3,943 generated
  lines) belongs in a frontend/edge crate, not exec. **[debt]**
- **The 0056 target layout is right** (kernel / host / ir / sources /
  runtime / cli) and I would keep it, with two changes: (a) do the crate
  split (F4) *before* the kernel extraction (F2), not after — the split is
  mechanical, de-risks everything else, and makes the layering lint
  enforceable immediately; (b) add a `gripsack-frontend` (or put eval in
  the CLI) so `exec`/`runtime` receive admitted IR and never see Deno. **[strategy]**

---

## 3. Verification strategy

### 3.1 What is proved, and what it is worth

Inventory (`proofs/Kernel/*.lean`, 32 explicit theorems + 2 equation
lemmas; audit allow-list `propext`/`Classical.choice`/`Quot.sound`; no
`sorry`, no project axioms; drift check is a byte diff of `Extracted/`;
14 string-replacement mutants each must break exactly its theorem):

| Family | Substantive | Trivial / restating the definition |
|---|---|---|
| K3 classify | 3 iff theorems (finite case split) | 4 `same_spec` equality lemmas |
| K4 retention | `plan_prune_spec`, `plan_delete_spec`, `admit_gc_spec`, `strictly_ascending`, 7 loop lemmas | `names_generation_spec`, `deref_*` |
| Journal | `acknowledge_spec` (2×6×6 case split) | `new/action/finish/successor/same`, `admit_mutation_spec` |

The seven loop lemmas are real proof work (invariants over Aeneas's `loop`
combinator). Everything else follows from the definitions by `simp`. The
user-facing content is three sentences on the guarantees page, and all
three were already true and tested before the pilot.

What K4 does **not** cover is where GC actually goes wrong: the inventory
from `readdir`, the `pending` flag, and the root set (`gc.rs:117-142`) —
K4′ is explicitly "ring-3 input, tested" in 0056. K6 (fresh TxId,
high-water floor) is not started, though the TLA+ models cover the floor
and nothing in Lean does.

**Verdict:** the F1 pilot answered the question it was asked — *can
Charon/Aeneas translate shipped Rust and can a solo maintainer prove
things over it in a day* — and the answer is a credible yes. It did not
yet produce a guarantee a user can feel. That is fine for a pilot and
should be said plainly on the website, which currently leads with
"proved in Lean over the Rust that ships" above a list of three
properties that are, candidly, small. **[strategy]**

### 3.2 Is K1–K6 / P1–P4 / S1–S7 the right target set?

Mostly yes for K, no for P, and S needs one addition.

- **K1 (ownership) and K5 (splice) first.** They are where the open
  defects are (FND-1, the `ours` rule, in-block overwrite), they are pure
  functions of (desired, live, previous record, consent) already, and a
  theorem of the form "Preserve/Refuse never claim the desired state;
  anything not Fresh/consented/last-managed is never overwritten" is the
  one that backs the README's promise. Each is a day or two at the pilot's
  measured rate (1–2 h per loop function).
- **K2 (fidelity) as stated includes "executing it to `Landed` yields the
  intended identity"** — that is a ring-1 claim about effects and will
  drag the storage model in. Split it: the pure part (decision is a
  function of four inputs; Preserve/Refuse never claim desired) is ring 0
  and cheap; the `Landed` part moves to P.
- **K6 should be done early and is cheap**: "numbers exceed the floor" and
  "TxId never reused" are small, and the high-water mark is the thing that
  makes orphan generations (finding 10) survivable.
- **P1–P4 under S1–S7: push back.** This is where 0056 estimates 3–4 weeks
  and I estimate 2–3 months for a solo maintainer, because it requires
  (a) the sans-IO machines to exist (F2, 8–10 weeks itself), (b) a
  `Storage.lean` crash semantics with *two* crash kinds and no prefix
  closure (S3), and (c) proofs by induction over traces of a loop machine
  with at most one outstanding effect. The 2026-11-08 design study already
  found the first draft's P2 and P4 "false or unprovable as stated". Nobody
  has yet done a crash-safety proof over an Aeneas translation of
  production code; the Aeneas `loop`/`Result` model was the hardest part
  of a 500-line pilot. The payoff is a theorem whose assumptions (S1–S3)
  are exactly the things that fail in practice (ext4 `data=writeback`,
  WSL2 9p/DrvFs, an fsync that lies). The *same* confidence is obtainable
  for a fraction of the cost by making `MemFs` crash-injecting and
  enumerating every cut point — which 0056 F2 step 1 already requires and
  which found the 0.19.1 resurrection bug once. Keep P1–P4 as
  *aspirational* TLC-checked models (≤5, as planned); do not make them a
  release claim or a scheduled phase. **[strategy]**
- **S5 must grow.** "External writers are only detected" is the honest
  statement, but the storage model should also say *what evidence survives
  a race*: the pre-race prior blob does (pinned by the manifest), the
  racing write does not. Say both.
- **S6 is written backwards for the real hazard.** It requires the fs to be
  no *coarser* than the kernel's key equivalence; on `/mnt/c` the fs is
  coarser and the code is finer (FND-3). The invariant should be "the
  kernel folds at least as much as the fs", with the fold a kernel
  function.

### 3.3 Is Charon/Aeneas → Lean sustainable for one person?

Honest assessment: **yes for ring 0 at its current shape; doubtful for
ring 1; and the surrounding estate must shrink first.**

For:
- The pin set is explicit (`proofs/Dockerfile:12-16`), the drift check is
  byte-exact, the audit rejects every non-standard axiom, and the mutant
  calibration gives a cheap statement-strength check. This is better
  hygiene than most research projects.
- Translation is ~1 s; `lake build` ~13 s warm. The style rules (no
  derives, no nested borrows, no casts, explicit `same`) are learnable
  and are documented in `gripsack-kernel/src/lib.rs`.
- Proof effort per function is hours, not days.

Against:
- Five pins move together, two of them nightlies (Aeneas
  `nightly-2026.11.07`, Charon built from an un-released commit on rustc
  `nightly-2026-09-17`). Each bump regenerates `Extracted/` and may break
  every proof. There is no upgrade history yet — all seven kernel/proof
  commits are from one day (2026-11-08) — so the real maintenance cost is
  unknown. Budget one day per month for it and measure.
- Images are 9.4 GB + 19.7 GB and the cold build is ~11 min; a cache miss
  on the qualification host (already storage-constrained per
  `AGENTS.md` and 0056 §F0b) blocks releases. 0057 is addressing this.
- The nightly-rustc MIR vs stable-musl binary gap is part of the trusted
  base and is not written on the guarantees page. Add it.
- `convenience.rs` (242 lines) re-implements `PartialEq`/`Ord`/`Hash`
  by hand outside the translation; `Ord` is what `BTreeSet` in `gc.rs` uses
  and its agreement with `same` is "by construction", not proved. Add a
  property test or move `Ord` into the kernel as a function.
- Three stacks coexist. Verus cannot build (pin sha changed upstream,
  0056 §13.7) yet 33 files of `verus!` still ship in product crates;
  12.8k lines of TLA+ are optional. The survival rule in 0056 §2.5 says
  delete; the trim register says "after K4/K5", "after P1–P3". **Delete
  now.** Unrun proofs are negative value: they cost reading time, they
  mislead reviewers (the TLA+ models cover the high-water floor that Lean
  does not), and they anchor the code shape to Verus's constraints. Git
  keeps them. **[strategy]**

---

## 4. What I would do next

Sizes: S = a day or two, M = a week, L = a month. Ordered by impact per
unit of effort for a single maintainer. Items 1–6 are what I would ship
as 0.53 before any further F2 work.

| # | Item | Size | Tag |
|---|---|---|---|
| 1 | Fix `plan_link`: `recorded` ⇒ live object is the recorded link; unrecorded link into home is foreign (FND-1 + `ours`). Add the failing-before e2e. | S | bug |
| 2 | Rollback restores recorded bytes and compares to `entry.hash`; refuse the flip on any unrestorable entry without `--force`; persist rendered template bytes per generation. Correct the README sentence until then. | M | bug |
| 3 | Fix untagged plugin receipt (`plugins.rs:96`); record plugin asset digest in `gripsack.lock`; confine plugins and the Conda helper with the evaluator's Landlock profile. | M | bug |
| 4 | Evaluator: drop `DBUS_SESSION_BUS_ADDRESS`/`XDG_RUNTIME_DIR`; Landlock net rules where ABI ≥ 4; cap `file_exists` probes per round; version-check the saved-plan Deno fallback. | S–M | bug |
| 5 | Release `.selections/<txid>` in `end_run`/`commit_run` and collect strays in `gc`; mark or collect never-flipped generations; `rollback` defaults to last *selected*. | S | bug |
| 6 | Shrink the window (re-observe after the journal entry is durable, immediately before the effect); fix merge compensation to re-plan; correct plan 0019's "microseconds". | S | bug |
| 7 | Delete Verus annotations, `check_verus*`, `verify` service, TLAPS modules, `tlaps_*.py`, `verus_evidence.py`, all nine `*_model.rs` explorers; keep ≤5 TLC models. Replace the bignum rate limiter with integer tokens + proptest. | M | debt |
| 8 | Crate split (0056 F4) *before* F2: `gripsack-host` (fs+process+store codecs), `gripsack-sources`, `gripsack-runtime`, `gripsack-frontend`, CLI; move `Sha256Digest` and `terminal` out of `process`; drop trace→store; `cargo-deny` direction rule. | L | debt |
| 9 | K1 + K5 + K6 in Lean (ownership rule, splice preserves-outside/idempotent, fresh TxId + floor). Rewrite the guarantees page from these. Mark P1–P4 as TLC-only, not roadmap. | M | strategy |
| 10 | F2 step 1 (`Storage` trait, `RealFs`/`MemFs`, every-cut crash enumeration) as a *test harness*, not as a proof prerequisite. | M | strategy |
| 11 | DSL: object references (`environment: dev`), infer `targetPlatform`, drop `provider()`, `exec("...")` overload without `lit()`/`.build()`, one casing, `service` env helpers. Ship as IR11 together with a *stability promise* (IR major = SDK major, one year of read compat). | M | ux |
| 12 | Approve the evaluation closure instead of the repository; `trust add --yes`; approval identity by repo digest not path. | M | ux |
| 13 | `grip status` (deployed/drift/pending/degraded/leaked) and `grip resolve <path> keep|adopt|restore`; fold `snapshot`/`diff-config` into `diff`; one `--repo` type; `--json` everywhere. | M | ux |
| 14 | Freeze services, sharing (`package *`) and Conda/Pixi/wheels: "experimental, no new features" banner; no new vendored forks. Decide in Q1 whether to keep or extract to a plugin binary. | S (decision) | strategy |
| 15 | Replace the three validators with Rust-as-authority + a generated TS pre-check (constants from `schema/`), restore a TS↔Rust golden parity corpus. | M | debt |
| 16 | Honest website edits: "sandboxed" → "Deno permissions + Landlock read confinement"; "nothing is rebuilt" → true statement; add the nightly-rustc/stable gap to the trusted base. | S | ux |

### Where this review disagrees with plan 0056

- **Order of phases.** 0056 does kernel extraction (F2) then protocol
  proofs (F3) then the crate split (F4). I would do F4 → the K1/K5/K6
  subset of F2 → the `MemFs` harness, and stop. The split is the only
  phase with no research risk and it makes every later phase smaller.
- **P1–P4 as a release claim.** Not within a year for one person;
  keep them as TLC bug-finders and say so.
- **"Delete after X."** The survival rule is correct; the register's
  deferrals undercut it. Delete the unrun stacks now.
- **Trim scope.** 0056 trims verification artefacts but not product
  surface. The product has the same problem: three products and one
  wedge. Trim the surface too (item 14).
- **0001 §3.7 template rendering.** The 0.16.1 amendment (vars in manifest,
  render on restore) is what makes rollback non-faithful. Revert to
  storing rendered bytes; it costs one small store artefact per template
  per generation and buys K2 for free.

### What I agree with, strongly

The rings as a mental model; "proofs are about the shipped code"; sans-IO
machines with one outstanding effect; `MemFs` with every-cut enumeration;
one proof language; the release rule; the survival rule; keeping TLC as a
counterexample finder; the three FND defects being first in the queue;
"code moves inward to gain guarantees, models are never added outward".
Those are the right principles. The disagreement is only about how much of
the mountain to climb this year, and in which order.

---

## Appendix A — numbers used above

| Metric | Value | Source |
|---|---|---|
| Age | 11 weeks (2026-08-22 → 2026-11-08) | `git log` |
| Commits | 606 (245 Aug, 213 Sep, 110 Oct, 38 Nov) | `git log` |
| Minor releases ≥0.16 | 40 | `CHANGELOG.md` headings |
| Breaking IR versions | v2→v10, 2026-09-06 → 2026-10-31 | `CHANGELOG.md` |
| Rust, production | ~101k lines; 19.6k in test paths; 791 `#[test]` | `wc`, `grep` |
| `gripsack-exec` | 54,376 lines (32.9k in `workspace/`, 6.1k `semantic/`, 3.9k generated frontend) | `wc` |
| `gripsack-kernel` | 785 lines, 0 deps; 516 under proof | `wc` |
| Lean | 742 lines proofs, 822 extracted, 34 theorems, 14 mutants | `proofs/` |
| Verus | 33 files `verus!`, ~6.3k lines, cannot build (pin drift) | `grep`, 0056 §13.7 |
| TLA+ | 90 modules, 12,825 lines, 39 TLAPS, optional | `specs/` |
| Rust explorers | 9 files, ~4k lines | `find *_model.rs` |
| Conda | 10.7k core + 3.5k helper + 27.9k vendored forks; 651 locked crates | `wc`, `tools/conda-helper` |
| Rate limiting | ~2,056 lines for one 30/min budget | `wc` |
| Plan prose | 16,753 lines across 57 plans | `wc plan/*.md` |
| e2e | 57 files, ~457 tests, ~25 min serial | `e2e/`, 0057 §2 |
| CLI | 8,471 lines, ~30 commands, 4 hidden | `crates/gripsack/src` |
| TS SDK | 3,856 lines, 50 values + ~75 types, 3 examples | `typescript/src` |

## Appendix B — known-defect cross-check

| 0056 §5a | Independently confirmed | Where |
|---|---|---|
| FND-1 owned link re-pointed | yes, plus the `ours` widening | `ops/plan/link.rs:12-20`, `gripsack-policy/src/ownership.rs:103-121` |
| FND-2 env backfill outside journal | yes | `exec/src/env.rs:27-49` from `rollback.rs:72` |
| FND-3 case-fold prune aliasing | consistent with code, unreproduced | `deploy/destinations.rs:26` vs `apply.rs:443-486` |
