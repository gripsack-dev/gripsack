# 0052 — A1 workspace contract and read-only v4 admission

Status: **A1-01 implementation in progress; full A1 and release unverified** · Owner: implementation agent · Date: 2026-09-24
Scope: Epic A / A1 (rows A1-01…A1-12; A1-07 partial in plan/0049)
Sources: bundle edition 5 — `START_HERE.md`, master plan §§3–5, 7–8; Epic A §§3.1–3.5, §7 rows A1-01…A1-12, §§8.2–8.3; local plans 0003 §8, 0013, 0035, 0036, 0048 §§6/9/10; live checkout at the time of writing.
Coordination: plan/0050 (E0 vocabulary/scenarios), plan/0051 (B0 LLB matrix), plan/0049 (H0 inventory/ledger). Plan 0048 §9 release classes still bind every public release; this plan makes no release claim.

This document freezes the *shape* of A1: the exact wire grammar, the value boundaries, the identity model, the ownership/dependency map and the caller-migration inventory. It is written so that schema, Rust and TypeScript can be implemented in one PR series without inventing competing parallel APIs. Where a behavior is deferred to a later milestone, the contract admits the declaration and names the rejection path — it does not declare a placeholder API that pretends to work.

## 1. Live v3 facts and the version-policy reconciliation

Ground truth from the live checkout (verified by reading, not by plan prose):

| Fact | Location |
|---|---|
| `Ir` root and every struct (`HostFacts`, `Module`, `Entry`, `Dependency`, `EnvVar`, `Resource`, spans) carry `#[serde(deny_unknown_fields)]` | `crates/gripsack-ir/src/model.rs`, `span.rs` |
| Tagged unions (`FetchSpec`, `Build`, `Intent` actions, `Verify`, `Step`/`StepAction`) cannot use `deny_unknown_fields`; a pre-deserialization pass walks the JSON and rejects unknown fields per tagged node, with a hard error before serde | `crates/gripsack-ir/src/tagged.rs` (`tagged_field_check`), invoked from `parse.rs` before `serde_json::from_str` |
| The version gate is **exact equality**: `ir_version != 3` is E100 (`unsupported ir_version … this core accepts 3`) | `crates/gripsack-ir/src/tagged.rs:168` |
| The single accepted version is a constant, not a range | `crates/gripsack-ir/src/parse.rs:7` (`pub const IR_VERSION: u32 = 3`) |
| The schema mirrors this: root `additionalProperties: false`, `ir_version: { "const": 3 }`, description explicitly aligned with "deny-unknown structs plus the tagged-field pass" | `schema/ir/v3.json:5,11,14` |
| The frontend emits exactly `ir_version: 3` | `typescript/src/graph.ts:12,93` |

**Conflict.** Plan 0003 §8 and the IR skill prescribe unknown-field tolerance
for additive changes. Live v3 readers instead reject unknown structural fields.
This is a live compatibility discrepancy, not permission to silently weaken
either contract. A1 changes the meaning and shape of the module pipeline, so
it is **breaking even for a tolerant reader** and must bump the version.

**Resolution for the A1 cutover.**

1. Record and amend plan 0003 §8 and the IR skill in the same cutover PR:
   spell out which structural fields remain strict and how future additive
   changes are versioned. Until then, v3 is strict in practice; no agent may
   assume an old core ignores new workspace fields.
2. A1 bumps `ir_version` to 4 and adds `schema/ir/v4.json`, retaining v1–v3
   schemas as history. Renaming/reinterpreting the public Step/Phase and
   Module shapes is a breaking change by the existing skill definition.
3. The core accepts a declared `3..=4` range via separate typed readers
   and one tested v3-to-current admission adapter; the frontend emits only 4.
4. Unknown-field rejection remains explicit inside each version (deny-unknown
   structs plus the tagged-field pass) with source-aware diagnostics.

### 1.1 Exact impact of the v4 cutover on existing artifacts

| Artifact | Impact | Required handling |
|---|---|---|
| `Ir` literals in Rust (`model::Ir` constructions and inline IR JSON in tests, e.g. `gripsack-exec/src/source/overlay/tests.rs`, `ops/model.rs`, sema fixtures) | v3-shaped literals stop compiling/parsing against the v4 model | Migrate every literal to v4 types in the cutover PR; literals that exercise the *v3 reader* move under the compat reader's own fixtures |
| Golden corpus `e2e/fixtures/golden/kitchen-sink.ir.json` | The emitted envelope changes shape and version | Regenerate with `REGEN_GOLDEN=1 pytest e2e/test_golden.py`; the snapshot diff is PR evidence (skill checklist). Add v4 fixtures covering the four graduated examples (§5.4); the v3 fixture stays as the compat-reader fixture |
| Embedded frontend `crates/gripsack-exec/src/embedded_frontend.rs` | Vendored generated copy of the TS sources (0035 F6) embeds the old emitter | Regenerate via the existing generation script; the CI freshness check must cover it; crates.io packaged-install smoke remains the gate |
| `grip check` vs `grip plan`/`grip apply` | All three parse/admit IR; only their post-admission behavior differs (§6) | v4 admission is shared; `check` keeps its zero-effect contract (0011 §9) and must never provision a builder; a distinct command/flag owns build-check execution (master plan §6) |
| Persisted state (generations, `manifest.json`, journal priors, store receipts, ownership records, lint registrations) | IR is transient at the evaluator boundary, but plan-derived fields can be stored separately | Audit actual readers/writers and historical fixtures before asserting that the v4 wire change needs no persisted-state migration. Preserve existing ownership/hash meanings; if any persisted field changes, ship a compatible reader or explicit tested migration. No bulk rewrite of retained generations. |
| Old pinned `@gripsack/core` frontends in user repos | Emit v3 | Read through the v3 compat reader; `grip doctor` keeps flagging pin/core mismatch; the E100 help text already points at the pin |
| Future saved plans (Epic C) | A new persisted full-plan snapshot, unlike existing generation and lock records that already store IR-derived fields | Out of A1 scope; when C lands, saved plans bind `ir_version` at capture and replay through the matching versioned reader |

**Confirmed durable coupling to v3 IR types:** `crates/gripsack-store/src/generations.rs`
serializes `gripsack_ir::Ownership` in each `DeployedEntry`,
`gripsack_ir::{Action, Trigger}` in `IntentRecord`, and
`gripsack_ir::EnvVar` in `ModuleState`. `crates/gripsack-exec/src/lockfile.rs`
serializes `FetchSpec` in `LockEntry`. These are not full `Ir` envelopes,
but changing their serde representations would change retained
generations or locks. Keep their old wire variants readable via
versioned durable types/compat readers, or ship a tested explicit
migration. Historical inspect, rollback, update and GC fixtures are
mandatory before calling a wire cutover safe.

## 2. v4 cutover design

### 2.1 Envelope

v3's envelope is `{ ir_version, host?, resources?, modules: {name: Module} }` — a host-selected module map. v4 has one workspace entry, while retaining core-injected host facts (not a `hosts/<name>.ts` selection):

```jsonc
{
  "ir_version": 4,
  "workspace": {
    "span": {"file": "gripsack.ts", "line": 1},
    "name": "…",                 // optional full-A1 target
    "inputs": { /* captured source/import identities, A1-05 */ },
    "mutation_locks": [ /* reachable pure lock refs, A1-12; NOT portable pins */ ],
    "outputs": [                // array preserves duplicate names + both spans for admission
      { "name": "tool", "kind": "package", "span": …, … },
      { "name": "personal", "kind": "profile", "span": …, … }
    ]
  },
  "host": { /* core-injected OS/arch/ABI facts; NOT a hostname selector or global build target */ }
}
```

Rules:

- `outputs` names are the single catalog namespace; a collision is an admission error naming **both** declaration spans (A1-06). Local step labels and module names are *not* catalog names and carry no cache/deployment identity (Epic A §3.2).
- The A1-01 read-only admission slice initially accepts `span` and
  `outputs` only; it **rejects** (does not ignore) `name`, `inputs` and
  `mutation_locks` until A1-05/A1-12 implement their normalization
  and identity rules in this v4 cutover. No v4 public release is
  authorized with those mandatory A1 rows incomplete.
- A project needs no fake `hosts/<name>.ts` file or personal profile to
  declare a package. The core still injects host facts to admit native
  processes, environments and profiles; each recipe/output also carries
  its own execution platform and target instead of inheriting `host`.
- Every v4 IR node carries **required** `span: {file, line, col?}` from the
  originating declaration, including helpers and expanded tree entries.
  The frontend emits it, the core retains/surfaces it, and identity hashes
  exclude it (§4). Only versioned v3 legacy readers keep their old optional
  span semantics.

### 2.2 Typed wire grammar (normative sketches)

Field names are final unless the cutover PR records a mapping (Epic A §3.2 allows respelling with a recorded mapping, not semantic drift). All structs are deny-unknown; all tagged unions go through the tagged-field pass with span-carrying rejection.

**CommandSpec** — the shared command value (A1-03, A1-08). Two variants, one type:

```jsonc
{ "kind": "exec",
  "argv": ["jq", "--sort-keys", ".", {"ref": "input:settings"}],
  "env":  {"NAME": {"literal": "…"} | {"ref": "…"}},
  "cwd":  {"literal": "…"} | {"ref": "artifact:…"} | null,
  "span": … }
{ "kind": "run_bash",
  "body": "jq --sort-keys . \"$INPUT\" > \"$GRIP_OUTPUT\"",   // literal text ONLY
  "interpreter": {"tool_ref": "<package output providing bash>", "sha256": "<64 hex>"},
  "options": ["-e", "-u", "-o", "pipefail"],                   // fixed strict set
  "line_map": [[generated_line, source_line], …],              // dedent mapping (A1-06)
  "env": {…as exec…}, "span": … }
```

Boundaries (enforced at admission, with declaration spans):

- `run_bash.body` is literal text. `${…}` interpolation is **rejected** (E-code, span at the interpolation site); dynamic values enter only through typed `env`/`argv` bindings (Epic A §3, Brioche-derived contract). Dependency inference never parses the body.
- `interpreter` is a **pinned tool reference resolved through the declared toolchain** — never ambient `bash` discovery, never a host fallback (A1-03).
- Legacy POSIX-shell scripts do not silently gain Bash semantics; they migrate explicitly or run under a bounded documented compat interpretation with a diagnostic (Epic A §3.2).
- A CommandSpec is a *description only*. The enclosing recipe, task, check or hook admits the execution context; a cache flag or output filename never promotes a host task into a recipe.

**Recipe / package** (A1-02, A1-04):

```jsonc
{ "kind": "recipe",
  "steps": [ {"command": CommandSpec} | {"action": ActionSpec}, … ],  // ordered local list
  "edges": [ {"from": "step label or input", "to": "…", "role": "ordering|build_input"} ],
  "outputs": { "<name>": {"kind": "file|tree", "selector": "…", "platform": TargetSpec} },
  "execution": {"kind": "host", "policy": HostPolicy} | {"kind": "isolated_linux", "worker": "…"},
  "checks": [ {"ref": "output:…check-name"} ],                        // publication gates
  "span": … }
{ "kind": "package",
  "producer": {"recipe": "<output ref>"} | {"provider": ProviderSpec}, // resolution/acquisition separated (A1-12, §3.8)
  "commands": { "<name>": CommandSpec },        // command refs imply artifact + runtime closure
  "runtime":  [ {"ref": "output:…"} ],          // explicit runtime closure
  "layout": {"prefix": "relocatable|fixed", …}, "targets": [TargetSpec], "span": … }
```

- `.outputFile()`-style sugar is recipe-construction sugar; it does not claim every Bash command produces a package.
- `execution.kind` unsupported on the current build → explicit `E… unsupported executor` diagnostic at admission; **never** silent host fallback (A1-01/A1-08). `isolated_linux` is admitted but unexecutable until B2; `host` receipts must state host filesystem/kernel access honestly and never share cache entries with isolated execution.
- Per-operation `platform`/ABI/minimum-OS/layout requirements ride on outputs and commands, not on one global `host` field (master plan §4): a Darwin laptop may request a Linux target without adopting it natively.

**Task / schedule** (A1-08):

```jsonc
{ "kind": "task",
  "steps": [ …ordered local actions… ],
  "deps":  [ {"ref": "task:…"} ],              // unordered prerequisite set: verified success within one invocation
  "checks": [ {"ref": "check:…"} ],            // per-invocation postconditions, distinct from build checks
  "context": {"kind": "host", "mutable_paths": […]},   // explicit; never inherited from ambient host task
  "span": … }
{ "kind": "schedule",
  "task": {"ref": "task:…"},              // E3 binds the durable revision at registration
  "trigger": {"kind": "daily|weekly", "local_time": "HH:MM",
              "weekday": "mon|tue|wed|thu|fri|sat|sun" | null},
  "policy": {"scope": "user", "missed": "native"}, "span": … }  // INERT
```

- `task.build(package)`-style ensure-artifact is an explicit `EnsureArtifact` action referencing the common realization service (A2/B3) — not a new producer identity and never a recursive CLI call.
- Arbitrary task outcomes/filenames cannot become recipe outputs without declared production or an explicit validated capture/import boundary.
- A schedule declaration alone activates nothing; registration is deferred to E2/E3 and rejected with an unavailable-capability diagnostic before then (A1-08: "inert schedule declaration and unavailable executor diagnostics").
- Named timezones, intervals, general cron and system/root scope are
  rejected in v1, not silently accepted as inert future promises (E §5).

**Build checks / hooks** (A1-08, A1-10): one check-construction surface over typed subjects, three distinct lifecycles that must never merge: build checks gate candidate *publication*; task postconditions gate the *invocation* and its dependents; pre-flip deployment checks retain their original stage; hooks retain lifecycle/replay rules (`post_link`/`post_activate`/`on_remove` semantics preserved from the live `Trigger` enum). A command used as a check may have effects and is never run by read-only preview.

**Environment / profile** (A1-01, A1-08):

```jsonc
{ "kind": "environment",
  "members": [ {"ref": "package:…"} | {"ref": "environment:…"} ],   // ordered; deterministic precedence
  "env_ops": [ {"var": "PATH", "op": "prepend|append|set", "value": {"ref"|"literal"}} ],
  "targets": [TargetSpec], "span": … }          // process-scoped selection; deploys no personal profile
{ "kind": "profile",
  "files": [ FileDecl ], "env": [ EnvOp ], "hooks": [ Hook ], "schedules": [ {"ref": "schedule:…"} ],
  "span": … }                                   // generations/ownership/journal/recovery stay authoritative
```

**File declarations** — three orthogonal axes, no template-ownership conflation (A1-11, Epic A §3.3):

```jsonc
{ "origin":  {"kind": "repo_file", "path": "…"}                    // typed repository/captured file
           | {"kind": "artifact_file", "ref": "output:…", "selector": "…"}
           | {"kind": "tree", "ref": "…", "include": […], "exclude": […]},  // bounded selection
  "content": {"kind": "identity"} | {"kind": "literal", "bytes": "…"}
           | {"kind": "template", "vars": {…}, "result_digest": "<bound at admission>"},
  "destination": {"path": "~/…",
                  "policy": {"kind": "symlink"} | {"kind": "tracked_copy", "drift": "…"}
                          | {"kind": "managed_block", "marker": "…"},
                  "ownership_scope": "…"},
  "checks": [ {"ref": "check:…", "subject": "source|rendered|deployed", "stage": "pre_flip|…"} ],
  "span": … }
```

- The same admitted template content composes with all three destination policies. Template source, variable binding and rendered result are stored sufficiently to inspect/restore the selected generation; rollback never re-evaluates TS or re-reads today's variables.
- `tree` expands a bounded captured inventory to explicit per-file entries with per-entry provenance, stable enumeration, containment and collision diagnostics; it never claims unrelated children.
- Managed-block merge remains an operation on the *observed destination* with foreign-byte preservation; the merged host file is never cached as a reusable content transform.
- Legacy serialized `owned` values stay readable with their original meaning (Epic A §3.2); new wire uses the action names `symlink`/`tracked_copy`/`managed_block`.

**Two different lock contracts** (A1-05 and A1-12):

```jsonc
// Pure authoring value for cooperative host mutation exclusion:
{ "kind": "mutation_lock", "scope": "<admitted user/store scope>",
  "key": "<stable logical key>", "span": … }

// Separate portable workspace lockfile, produced by explicit resolution:
{ "lock_version": 1,
  "resolutions": { "<platform>": {"pins": […], "transitive": […]} } }
```

- A `mutation_lock` is an immutable typed ref, collected only when
  reachable from the returned declaration graph. Scope/key remain
  independent of task revision, generation, module path and JS object
  identity; incompatible declarations report both sites. No global
  `resource()` registration, import-order dependence or reset helper.
- The portable workspace lock records per-platform pins for imported
  recipe/frontend libraries, providers and transitive ecosystems.
  Explicit update modifies resolutions; frozen apply never re-solves.
  Local VM/socket/lease state does not belong to that portable lock.
- Mutation locks are NOT dependency pins, backend capacity budgets,
  cache-mount policy or worker leases. A user key grants no network
  entitlement or backend choice.

**Edges and identities** (A1-02): v3's `EdgeKind {runtime, build}` becomes an explicit role enum:

```jsonc
"role": "production"      // recipe output ← producer
      | "build_input"     // build-time only; must not leak into runtime closure
      | "runtime"         // required at consumption; implies artifact + closure
      | "ordering"        // sequencing only (lowered from ordered local lists)
      | "task_prereq"     // verified success within one invocation
      | "validation"      // required publication gate; never prunable dead work
      | "retention"       // root/GC protection
```

Invalid references, cycles, wrong output kinds/selectors, incompatible targets/layouts and missing runtime/tool references are admission rejections carrying declaration spans (A1-02 evidence). One admitted model exposes graph *projections* per role; relations are never forced into one untyped DAG (master plan §4).

**Explicitly not in the grammar** (A1-09, A1-12): no universal stringly `Step`/`Phase` pipeline, no public phase flag that grants effects (Rust rejects forged/unknown phase/context fields), no import registry or string lookups, no runtime JS callbacks. Local ordered lists lower to `ordering` edges with failure/cancellation propagation preserved; label-only renames preserve production semantics, reordering an ordered list does not.

### 2.3 Backward readers and retained persisted state

- **Retained persisted state requires an inventory before a no-migration claim.** IR is transient at the evaluation boundary, but manifests, receipts and journals store plan-derived facts independently. The cutover must examine every retained reader/writer and historical generation fixture; preserve all valid ownership/hash meanings. If v4 changes a persisted field, include the compatible reader or explicit tested migration in the same PR. Rollback of an old generation must not re-evaluate TS.
- **Versioned reader:** `gripsack-ir` keeps a `v3` module (today's `model.rs`/`tagged.rs` shape, moved, not rewritten) behind the version dispatch. v3 documents parse into the v3 model and normalize into the canonical internal model through one explicit, tested adapter — the same adapter the A5 authoring migration uses, so there is exactly one legacy→canonical mapping. Compat fixtures: the retained v3 golden file plus hostile decoded-IR cases (unknown fields, forged phase/context, invalid edge roles) that must still fail with source spans.
- **Ambiguity is never silently reclassified.** Legacy constructs without a unique destination (e.g. a script-valued hook that is both check and activation effect) produce a source-aware migration diagnostic and preserve retained state; bulk rewriting ambiguous scripts is forbidden (Epic A §§3.1, 3.5).
- The E100 message now names the accepted range (`3..=4`) and keeps the pin-update help.

## 3. Module ownership and dependency direction

Dependency direction is one-way toward the admitted model; nothing below the line imports anything above it, and store/policy never import BuildKit/Lima/solver/`systemctl`/`launchctl` (master plan §7):

| Module | Owns (A1) | Must not import / own |
|---|---|---|
| `typescript/src/` (emitter) | Authoring API, value construction, span capture, dedent line maps, emit of exactly v4 | Execution, network, host observation beyond the inputs envelope (0013 D2 sandbox stands) |
| `crates/gripsack-ir` | v4 types, parse passes, version dispatch + retained v3 reader, structural admission, canonicalization for identity hashing | Graph semantics beyond structural admission; effects |
| `crates/gripsack-ir::sema` (+ new `workspace` sema module) | Catalog collision admission, edge-role/reference/target/cycle validation, executor-capability admission, context admission | Publication, ownership planning |
| `crates/gripsack-policy` (Verus kernels) | Semantic normalization/admission kernels production actually invokes (§5.1) | Serialization, rendering, I/O |
| `crates/gripsack-exec` | Realization coordination against admitted plans; existing supervision for host execution; ownership planner/journal unchanged in meaning | A second planner, a second transaction implementation, per-step scheduling around LLB |
| `crates/gripsack-store` | Durable outputs, receipts, roots, retention; hash inputs reviewed per skill checklist | BuildKit cache as the only copy; changed stored meanings |
| `crates/gripsack/src/commands/*` | CLI surfaces (§6) | New semantics; diagnostic text owned by `gripsack-ir` diagnostics registry |

New code lands as modules in existing crates first (master plan §7); the ~400-non-generated-line split review (Epic A §8.2) applies. No new crate, no public generic backend framework, no proof-only kernel clone.

### 3.1 Callers to migrate at cutover (exhaustive at time of writing)

TypeScript: `index.ts` (27 root exports — inventory per §5.3), `module.ts` (`module()`/`ModuleSpec` → ordinary composition + new output constructors), `steps.ts` (`step`/`fetchStep`/`buildStep`/`installStep`/`configStep`/`runStep`/`shellStep`/`Phase` → retired from authoring; compat adapter only), `graph.ts` (`defineEnv`/`emitIr`/`IR_VERSION` → workspace emit, version 4), `resources.ts` (`resource`/`CORE_RESOURCES`/`clearResources` → pure lock refs), `entries.ts` (`merge`/`symlink`/`template`/`trackedCopy` → FileDecl axes), `fetch.ts` (providers lower through resolution/acquisition separation; `pixi(pkg)` naming → `conda.environment` is A3-owned, `pixi.fromLock` explicit), `verify.ts` (check constructors over typed subjects), `intents.ts` (structured intents retained), `conditions.ts` (`when`/`hasTag` stay small pure conveniences), `inputs.ts`, `pin.ts`, `probe.ts`, `deps.ts`, `cli.ts`.

Rust: `gripsack-ir` (`model.rs`, `parse.rs`, `tagged.rs`, `sema/*`, `prepared.rs`, `step.rs`, `placeholders.rs`, `dependencies.rs`), `gripsack-exec` (`resolve.rs`, `ops/model.rs`, `embedded_frontend.rs` regeneration + its generation script, source/overlay fixtures), `gripsack` commands (`eval.rs`, `check.rs`, `plan.rs`, `apply.rs`, `doctor.rs` pin messaging, `why_owns.rs` naming), `gripsack-lint` registrations (lint subject/stage mapping), `griplint` packs that reference retired authoring names.

E2E/golden: `e2e/fixtures/golden/` (regenerated + new v4 fixtures), `e2e/test_golden.py`, e2e scenarios referencing `step`/`module` authoring, TS conformance tests, installed-package export tests (A1-12). Docs/examples: `AGENTS.md` (stale IR-v2 reference — frontend emits v3 today, v4 at cutover), user docs and the four graduated examples (§5.4). Out of scope here (integration owner): `verification/delivery.json`, `scripts/check_delivery.py`, `plan/STATUS.md`.

### 3.2 Executable entry paths and unsupported-capability errors

| Entry (live path) | v4 behavior |
|---|---|
| Internal frontend evaluation (`commands/eval.rs`, used by `check`/`plan`/`apply`) | Sandboxed Deno eval (0013 D2 unchanged) emits v4; declaration evaluation runs no build or host effects and needs no fake `hosts/<name>.ts` for a project workspace |
| `grip check` (`commands/check.rs`) | Sandboxed eval + v4 admission + sema, then list named workspace outputs; no profile mutation or builder startup (frontend runtime may be provisioned) |
| `grip plan` / `grip apply` | A1-01 rejects workspace execution with E124 before effects; A2 will extend the common ownership planner to workspace profiles. Legacy v4 module flows retain their existing behavior during migration |
| Build checks | Distinct command or explicit execution flag (exact spelling an A1/A2-P CLI decision); `check` does not suddenly run workloads |
| `grip run --env …` / `grip shell …` | A2-P-owned (provisional spellings); A1 defines only the admitted environment/task shapes |
| Unavailable executor (`isolated_linux` before B2, schedule registration before E3, host runBash without declared toolchain pin) | Explicit E-code diagnostic at admission naming the capability, the declaring span and the owning milestone; **no silent fallback** |

Diagnostic contract (A1-06): codes allocated through the repository registry; terminal and `--json` outputs agree on facts; both collision sources are shown; dedented script locations map to original source lines via `line_map`.

## 4. Identity model (A1-04, A1-05)

Distinct identities, deliberately never interchangeable (0036/0048 §10 typing rules apply: semantic values get semantic types):

| Identity | Covers | Excludes | Invalidates when |
|---|---|---|---|
| Recipe semantic identity | Script text, declared source bytes, toolchain pin, enforcement policy, output kinds/selectors, build-input edges | Provenance spans, labels, consumer wiring | Source/script/toolchain/policy change |
| Provenance | file/line/col, module/factory location | — (never hashed) | Never — diagnostic-only changes preserve identity (A1 acceptance) |
| Consumer identity | Deployment/invocation wiring: profile files, env selection, schedule binding, task arguments | Recipe semantics | Changing consumer deployment must **not** rebuild its package |
| Archive / tree / lock digests | Downloaded bytes, realized tree, per-platform lock resolution | VM/socket/lease state | Changed transitive/import pins change identity; locked resolution never floats or re-solves |
| Admitted-plan identity | Captured semantic plan + source/import snapshot + injected facts + pins, captured **before** effects | Ambient clock/randomness assumptions (documented TS determinism limit, master plan §5) | Any admitted input change |
| Realization receipt | Realized output + execution policy + host-access honesty for host mode | — | Per execution; receipts describe host access accurately |

Source snapshots vs writable checkouts are distinct: a cached build consumes a captured tree with inclusion rules and recorded dirty-content identity; a development task consumes the writable checkout; merely editing application source does not rebuild an unchanged tool environment (A1-05).

## 5. Verification design (claim evidence ≠ implementation)

Everything in this section is a **target**, not a result. Plan 0048 §6/§9 binds: no proptest-for-proof substitution, calibrated negatives must fail with attribution, blocked lanes stay visible.

### 5.1 Verus proof targets (production-used kernels only)

| Target (new guarantee-ledger IDs at implementation) | Production mapping | Properties | Calibration (must fail with attribution) |
|---|---|---|---|
| Semantic normalization kernel | `gripsack-policy` normalization used by admission | Fluent ≡ object form normalization; label/module/import-order invariance; reordering sensitivity of ordered lists | Mutant: treating a label or destination spelling as identity |
| Context/executor admission kernel | `gripsack-ir::sema` admission | Forged phase/context rejection; typed command refs imply artifacts; unavailable-capability rejection is total | Mutant: allowing a phase flag to grant effects |
| Identity/invalidation kernel | `gripsack-store` hash inputs | Provenance exclusion; source/toolchain/policy invalidation; consumer-independence | Mutant: conflating lock and cache identities; diagnostic-only invalidation |
| Graph/closure completeness (extend 0047 kernels) | Name/index/schema adapters | Required outputs and runtime/build refs retained under distinct edge semantics; missing/invalid names rejected at admission | Mutant: dropping a required validation edge |
| Rendered-content mapping (with A2 ownership/merge kernels) | FileDecl admission | Source origin preserved through rendering and all three destination policies | Mutants: losing the source origin; treating a managed-block merge as a cached transform |

TLA+/TLC extension (rendered-candidate validation, deployment observation, retained recovery) and the focused TLAPS protected-publication invariant over the new content mapping are **A2-owned** extensions of existing models; A1 names them and supplies the admitted representations. No proof-only clone; one production implementation shared by production, proofs and explorers.

### 5.2 Delivery checker calibration (A1-07 — **partial in this slice**)

The integration slice (plan/0049 lineage) owns `verification/delivery.json`, `scripts/check_delivery.py`, runner-evidence admission in `scripts/delivery_evidence.py`, and their calibrated CI gate. Its `scripts/check_architecture.py` enforces current protected crate dependency direction. The Rust gate now exercises both the v3 schema/parser parity and v4 schema admission tests, while TypeScript and real CLI tests cover the first v4 workspace corpus. **A1-07 remains partial:** full cross-language golden coverage across output kinds, complete lane/case inventory and module/API cutover are not yet implemented. The §3 module map names the target ownership, not passing evidence for that cutover.

### 5.3 Export/migration inventory (A1-09, A1-10, A1-12)

At the v3 baseline, `typescript/src/index.ts` exported **40 runtime
values and 29 types** (not 27 exports). The table inventories those
baseline symbols and their cutover destinations. The current partial
v4 slice adds workspace exports but has **not** retired legacy root
constructors; A1-09/A1-10/A1-12 migration remains open. `advanced`
is an explicit stability tier, never a second execution path.
V3 serialized values keep their versioned reader.

| v3 baseline runtime exports | Cutover destination and owner |
|---|---|
| `dep` | Typed artifact/runtime/ordering refs replace module-name dependencies; retire root constructor (A1-02, A5-05). |
| `merge`, `symlink`, `template`, `trackedCopy` | Orthogonal content vs destination policy; `symlink`/tracked copy remain conveniences, `merge` becomes `managedBlock`, `template` renders content only; retain v3 stored tags' meanings (A1-11, A2-06, A5-06). |
| `brew`, `fileFetch`, `git`, `githubRelease`, `pixi`, `pluginFetch`, `tarball` | Provider helpers over resolution/acquisition; `brew` closure belongs A4; coherent `conda.environment` and explicit `pixi.fromLock` belong A3; native releases/file/git/plugin/tarball remain A2 (A1-12). |
| `hasTag`, `when` | Retain pure convenience over injected facts, never a global fact registry (A1-12). |
| `defineEnv`, `emitIr`, `IR_VERSION`, `mergeTags` | Workspace entry replaces `defineEnv` in ordinary authoring; emitter/version/tag merger are internal or documented advanced; v3 reader retained (A1-01, A1-12, A5-05). |
| `tree` | Retain bounded tree-expansion helper producing explicit owned per-file entries (A1-11, A2-06). |
| `module` | Retire runtime `module()` identity/registry; ordinary TypeScript factories/namespaces group values, v3 reader preserves old inputs (A1-09, A5-05). |
| `customHook`, `desktopEntry`, `fonts`, `service` | Preserve structured host intents and lifecycle timing; custom script uses shared command construction, not an ordinary task or build validator (A1-08, A5-05). |
| `parseInputs`, `createProbeBuilder` | Internal driver entry points; injected probe context remains a supported authoring value, not a user-managed registry (A1-12). |
| `CORE_RESOURCES`, `clearResources`, `resource` | Retire mutable global resource registry/reset; reserved core names stay internal; new `lock()` yields pure scoped mutation refs reachable from returned declarations (A1-12, E1-07). |
| `buildStep`, `configStep`, `fetchStep`, `installStep`, `runStep`, `shellStep`, `step` | Retire public universal phase pipeline and duplicated script constructors; local ordered command/action lists in recipes/tasks, typed profile files and shared `exec`/`runBash`; v3 compat reader only (A1-03, A1-09, A5-05, B2-07). |
| `verifyBinary`, `verifyDeployed`, `verifyFile`, `verifyShell` | Retain typed check-construction conveniences with explicit subject/stage; build validation, invocation postconditions and deployment pre-flip checks remain different lifecycles (A1-08, A1-10). |

| v3 baseline type exports | Cutover destination |
|---|---|
| `Dependency`, `Edge` | Typed graph roles/references, advanced IR projection; no stringly module dependency as ordinary authoring (A1-02). |
| `Dest`, `Ownership` | Typed file destination-policy authoring; old serialized `Ownership` variants still readable (A1-11). |
| `HostFacts`, `Condition`, `FactView` | Retain typed injected facts + small pure conditions (A1-01). |
| `Fetch` | Advanced provider description; ordinary users select sources/providers (A1-12, A3/A4). |
| `Env`, `EnvContext`, `EnvFn` | Workspace context/environment value types; v3 `defineEnv` types move to compat/advanced (A1-01, A2-P). |
| `IrEntry`, `IrModule`, `ModuleSpec`, `ModuleValue`, `Span` | Compiler DTOs internal; `Span` exposed only through the documented advanced diagnostics API; v3 module types remain in the compat reader (A1-06, A1-09, A1-12). |
| `Intent`, `Trigger` | Retain typed structured activation intents/triggers; schedule calendar trigger is separate (A1-08, E2). |
| `Inputs`, `ProbeBuilder`, `ProbeKind`, `ProbeRequest` | Inputs DTO/request machinery internal; the authoring context exposes a typed `probe` interface without wire fields (A1-12). |
| `Resource` | Replace with pure `MutationLockRef`, explicit scope/key, no registry identity (A1-12). |
| `Build`, `Phase`, `Step`, `StepAction`, `StepOpts` | Retire universal phase/step DTOs from ordinary authoring; v3 reader only (A1-09, A5-05). |
| `Verify` | Typed check value with lifecycle-specific owner, advanced type where necessary (A1-08/A1-10). |

### 5.4 Graduated examples (A1-10 admission only)

Four admitted, type-checked authoring examples land with the cutover: (1) dotfiles-only profile with rendered content — no synthetic package/recipe/task/schedule; (2) downloaded tool with native environment and owned config; (3) source-built package with a compatible command consumer and an alternate producer selection; (4) scheduled task using that tool — plus a small manual task. A1 admits and type-checks them; **execution is owned elsewhere**: A2/A5 run the native examples, A2-P/E1 the manual task, B3/B5 the source example, E3/E5 the scheduled example. Syntax alone satisfies no runtime gate.

### 5.5 Native no-builder guarantee

A1 evaluation, admission, `grip check` and preview never provision BuildKit/Lima and never require a builder or OS scheduler for the native examples (master plan §6, Epic A §3.1). `runBash` host execution uses existing `gripsack-process` supervision with an honest host receipt, or rejects. The blanket no-daemon promise is preserved for package/dotfile/manual flows.

## 6. Deferred executor lanes — owner milestones

A1 admits declarations; the row below owns execution. Types existing ≠ behavior claimed (Epic A §3.1):

| Capability declared in v4 | Executor owner | Until then |
|---|---|---|
| Host-mode `runBash` under declared toolchain | A1/A2 via existing supervision | Admission-validated only where toolchain pin unresolved |
| Native acquisition/composition/publication, file composition | A2 | Outputs admitted, unrealized |
| `grip run`/`grip shell`, first single-command Task | A2-P | Unavailable-capability diagnostic |
| Task prerequisite graphs, invocation verification/concurrency | E1 | `deps` admitted and checked statically; no runtime claim |
| systemd/launchd translation and registration | E2/E3 | Schedule declaration inert; registration rejected with diagnostic |
| Conda/Pixi ecosystems | A3 (`conda.environment`, `pixi.fromLock`) | Provider refs admitted; materialization deferred |
| Homebrew bottle closures | A4 | Same |
| Isolated Linux builds, checked LLB | B1/B2 (matrix: plan/0051) | `isolated_linux` execution rejected with diagnostic |
| Source packages/images | B3/B4 | Same |
| Authoring cutover, persisted-state acceptance, legacy path removal | A5 (A5-05) | Compat reader + adapter retained; old ownership behavior preserved |
| Scheduler cutover | B5 | Existing outer scheduler retained; no per-step LLB wrapper ever |

## 7. A1 row mapping — case and proof per row

All rows: **design frozen here; implementation pending; no evidence claimed.** "Case" is the representative acceptance test the implementation must run; "proof" is the §5.1 target.

| Row | Contract sections | Representative case (evidence to produce) | Proof/calibration target |
|---|---|---|---|
| A1-01 workspace/output contract | §2.1, §3.2 | `grip check` passes on a workspace with no fake hostname entry or `env.toml` and only a package output; unknown field rejected with span; unavailable executor errors explicitly | Schema/Rust/TS conformance; admission kernel |
| A1-02 typed outputs/edges | §2.2 edges, §4 | Cycle, invalid ref, wrong output kind, incompatible target/layout each rejected naming declaration spans; graph adapters exercise production policy | Graph/closure kernel + dropped-validation-edge mutant |
| A1-03 immutable authoring, runBash | §2.2 CommandSpec | Fluent ≡ object IR byte-identical modulo spans; `${}` interpolation rejected at its span; pinned interpreter required; argv/env boundaries preserved; dedent line map points at original lines | Normalization kernel (fluent/object equivalence) |
| A1-04 separate identities | §4 | Script/toolchain/policy change invalidates; file/line/label-only change preserves recipe identity; consumer rewiring does not rebuild package | Identity kernel + provenance-exclusion property + mutants |
| A1-05 per-platform locks | §2.2 locks, §4 | Locked resolution does not re-solve; changed transitive pin changes identity; VM/socket/lease excluded; snapshot vs writable-checkout distinction tested; TS determinism limits documented | Identity kernel; determinism documentation is a stated limit, not a guarantee |
| A1-06 diagnostics | §2.3, §3.2 | Real CLI failures for typo/type/collision/target/missing capability; both collision sources shown; JSON agrees with terminal; `check` provisions no builder | Diagnostic registry conformance; span-carrying rejection |
| A1-07 architecture rules + gates (**partial**) | §3, §5.2 | AGENTS/architecture updated to v4; dependency/schema checks mechanically enforced | Ledger/checker + negative calibration owned by integration slice (plan/0049/0051 lineage) — **out of this slice** |
| A1-08 shared commands, context grammar | §2.2 CommandSpec/Task/Schedule | Hostile decoded IR: forged context/edge rejected; typed command ref implies artifact; recipe/consumer/invocation identities separate; schedule inert; executor diagnostic explicit | Context-admission kernel + phase-grant mutant |
| A1-09 ordinary modules, context steps | §2.1, §2.2, §5.3 | Equivalent factory/object forms; moving a declaration between factories preserves recipe identity; no import registration; documented legacy field/verify mapping | Normalization kernel (location invariance) |
| A1-10 compact API + examples | §5.3, §5.4 | Exhaustive export inventory; four examples admitted and type-checked; ordered list vs `deps` distinction; shared command construction preserves check/hook/recipe/task lifecycles and source maps | Conformance suite; lifecycle separation cases |
| A1-11 file contract | §2.2 FileDecl | Origin/content/destination composed independently; equal relative filenames with different origins coexist; template across all three policies; old source roots and lint stages mapped; tree add/remove/collision/escape cases | Rendered-content mapping target named; ownership proof targets named (A2 executes) |
| A1-12 pure locks, SDK split | §2.2 locks, §5.3 | Repeat evaluation and import reordering in one process emit identical IR with no registry reset; scope/key equivalence and conflict diagnostics naming both sites; installed-package exports hide driver/compiler/reset utilities | Lock-identity property; export-surface tests |

## 8. Implementation checklist (when the cutover PR lands)

Per the IR skill, one PR series touching all three parties:

- [ ] `schema/ir/v4.json` added per §2; `v1`–`v3` retained; `ir_version` bump recorded as breaking
- [ ] `gripsack-ir`: v4 model + parse/tagged passes, version dispatch `3..=4`, retained v3 reader + single normalization adapter, sema admission, unit tests
- [ ] `gripsack-policy`: §5.1 kernels wired into production admission (no proof-only clone), guarantee-ledger IDs added with assumptions/bounds/calibration
- [ ] `typescript/`: emitter per §2.2, export inventory per §5.3, examples per §5.4, TS tests
- [ ] `gripsack-store` hashing inputs reviewed — identity changes intended and enumerated (§4)
- [ ] Every §3.1 caller migrated; compat adapter bounded; no competing live phase runtime
- [ ] Golden corpus regenerated (`REGEN_GOLDEN=1 pytest e2e/test_golden.py`); v3 compat fixture retained; snapshot diff reviewed in PR evidence
- [ ] `embedded_frontend.rs` regenerated; CI freshness check green; packaged-install smoke intact
- [ ] E2E: hostile decoded-IR cases, graduated-example admission, diagnostics parity, no-builder `check` proof
- [ ] `AGENTS.md`/docs updated (IR-v2 staleness fixed); ledger/STATUS/checker updated by the **integration owner in the same series**, not this slice

## 9. A1-01 read-only implementation packet (not A1 closure)

The schema/Rust/TypeScript slice adds `schema/ir/v4.json`, versioned
`gripsack-ir` admission (strict v3 reader + v4 workspace or legacy
module envelope), pure workspace output constructors, an embedded Deno
driver preferring `gripsack.ts`, and `grip check` catalog inspection
without `hosts/<host>.ts` or `env.toml`. Rust sema checks duplicate
names with both spans, the current named references/kinds, task
prerequisite cycles, source spans and illegal command contexts. Full
cross-role cycle/kind/selector and target/layout graph admission remains
A1-02; the read-only v4 workspace does not execute. `grip plan`,
`apply`, `update` and `adopt` reject workspace execution with E124
before host effects; no builder or OS scheduler is provisioned. The
v4 corpus includes a literal dotfile profile and a provider-backed
package without a synthetic recipe.

Focused observed evidence: container-built real CLI workspace e2e
**5/5 passed**; golden corpus regenerated in the e2e container
**2/2 passed**, with the prior kitchen-sink output changing only
`ir_version: 3 → 4`. Draft 2020-12 schema admission accepts both a
literal-only profile and provider-only package. `npm run build` +
`npm pack` served the compiled SDK to a separate Node 24 process;
its installed root export emitted a v4 profile workspace without
legacy `modules`. Integrated compose gates on 2026-09-24
(14:07–14:26 UTC) **all passed**: Rust fmt/clippy/tests (40
`gripsack-ir` unit, 5 v4 schema acceptance, 3 workspace admission
cases), TypeScript Deno tests **53 passed**, full real e2e **251
passed**, TLC image `RUN` **CACHED**, and the existing Verus policy
kernels executed **56 verified / 0 errors** plus four calibrated
mutants. A second complete compose pass after the Rust tagged-walker
and workspace-model module splits and `cargo fmt --all` also passed
(Rust fmt/clippy/tests, Deno tests, **251/251** e2e, TLC gate and
Verus **56 verified / 0 errors** with four mutants). The existing
policy proofs do **not** prove the new A1 workspace admission kernel;
new production-connected Verus obligations remain mandatory. Native
macOS, TLAPS and exact source-bound delivery evidence are not supplied
by these Linux worktree observations.

**Not delivered by this packet:** A1-02–A1-06 and A1-08–A1-12
identities, per-platform pins, pure mutation locks, ownership
materialization, source-built artifacts, task/schedule executors,
compact root-export retirement, four executed examples and the
production Verus A1 normalization/admission obligations. A1-07
checker/architecture work is partial (plan/0049). The existing v3
stored `Ownership`/`Action`/`Trigger`/`EnvVar`/`FetchSpec` wire meanings
remain unchanged. A1-01 is not `verified` in the delivery ledger until
its full source-bound reports and Mac/proof/CI lanes are complete.

## 10. Honesty register

- This document began as a design candidate; §9 records one read-only
  admission packet, not a completion or release claim for A1.
- Plan 0048 §9 NEXT gates bind any public release claiming A1 behavior;
  this plan authorizes no publication.
- A1-07's checker/ledger and protected dependency gate remain partial
  in plan/0049 and their own delivery rows.
- Where older plan 0003/skill unknown-field tolerance conflicted with
  strict v3/v4 readers, this plan and their amended rules record the
  live version boundary (§1).
