# 0052 — A1 workspace contract: v4 history and v5 typed admission

Status: **A1-01 implemented_unverified; A1-02 in progress; full A1/release unverified** · Owner: implementation agent · Date: 2026-09-24
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
2. A1-01 introduced `ir_version: 4` and `schema/ir/v4.json`,
   retaining v1–v3 as history. Its strict reader and read-only workspace
   remain available for old pinned frontends and saved IR.
3. A1-02 changed required execution/layout/target field shapes. The
   strict v4 reader would reject these fields, so the writer bumps to
   `ir_version: 5` and adds `schema/ir/v5.json`. The core accepts
   `3..=5`: v3 modules, historical read-only v4 workspaces/modules
   and current v5 workspaces/modules. The frontend emits only v5;
   no v4 `native` recipe or prefixless fixed layout is silently
   reinterpreted for execution.
4. Unknown fields are rejected within each declared version (strict
   plain structs plus a versioned tagged-field pass).

### 1.1 Exact impact of the versioned workspace cutovers

| Artifact | Impact | Required handling |
|---|---|---|
| `Ir` literals in Rust (`model::Ir` constructions and inline IR JSON in tests) | A1-01 added v4; A1-02 retained a separately typed historical v4 reader alongside current v5 | Migrate every literal to an explicit version; v3/v4 fixtures exercise their own retained readers, and direct executor APIs refuse both workspace versions before effects |
| Golden corpus `e2e/fixtures/golden/` | The emitted envelope changes to v5; old v4 bytes remain historical input | Regenerate under the current writer, review version/layout diff, and retain dedicated v4 schema/parser/round-trip fixtures; four graduated examples remain A1-10 |
| Embedded frontend `crates/gripsack-exec/src/embedded_frontend.rs` | Vendored generated copy of the TS sources (0035 F6) embeds the old emitter | Regenerate via the existing generation script; the CI freshness check must cover it; crates.io packaged-install smoke remains the gate |
| `grip check` vs `plan`/`apply`/`update` | All parse/admit versioned workspaces, but no A1 workspace executor exists | `check` lists named outputs without builder bootstrap; CLI and direct executor APIs reject historical v4/current v5 with E124 before home/store effects |
| Persisted state (generations, `manifest.json`, journal priors, store receipts, ownership records, lint registrations) | IR is transient but plan-derived fields are durable independently | Audit retained readers/writers and historical fixtures before any no-migration claim; preserve ownership/hash wire meanings and test inspect/rollback/update/GC |
| Old pinned `@gripsack/core` frontends | Emit v3 module maps or v4 workspaces | Read with the corresponding strict versioned reader; `grip doctor` continues reporting mismatched pins |
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

### 1.2 v4→v5 compatibility invariant

The A1-02 v5 schema changed execution from
`native|host|isolated_linux` strings to explicit host/isolated objects,
typed target ABI/minimum-OS values, and package layout/prefix
representations. v4's strict
schema is preserved byte-for-byte. A dedicated v4 typed reader stores
historical output values and round-trips them without promoting
`native` to host execution or inventing an install prefix. Shared
read-only graph admission uses a temporary validation projection that
is discarded; v4 target strings retain exact-match admission. Both
CLI and direct executor entrypoints reject either workspace version
with E124 before mutation. This is a versioned reader, not a second
live execution path.

## 2. Current v5 contract and retained workspace readers

### 2.1 Envelope

v3's envelope is `{ ir_version, host?, resources?, modules: {name: Module} }`; historical v4 added a read-only workspace. Current v5 retains one workspace entry with core-injected host facts (not a `hosts/<name>.ts` selector):

```jsonc
{
  "ir_version": 5,
  "workspace": {
    "span": {"file": "gripsack.ts", "line": 1},
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
- The initial A1-01 v4 slice accepted `span` and `outputs` only; the
  current v5 writer still rejects (does not ignore) `name`, `inputs`
  and `mutation_locks` until A1-05/A1-12 implement their normalization
  and identity rules. No public release is authorized with those
  mandatory rows incomplete.
- A project needs no fake `hosts/<name>.ts` file or personal profile to
  declare a package. The core still injects host facts to admit native
  processes, environments and profiles; each recipe/output also carries
  its own execution platform and target instead of inheriting `host`.
- Every v4/v5 semantic declaration carries a required `span: {file, line, col?}` from the
  originating declaration, including helpers and expanded tree entries.
  The frontend emits it, the core retains/surfaces it, and identity hashes
  exclude it (§4). Only versioned v3 legacy readers keep their old optional
  span semantics.

### 2.2 Typed wire grammar (normative sketches)

Field names are final unless the cutover PR records a mapping (Epic A §3.2 allows respelling with a recorded mapping, not semantic drift). All structs are deny-unknown; all tagged unions go through the tagged-field pass with span-carrying rejection.

**CommandSpec** — the shared command value (A1-03, A1-08). Two variants, one type:

The following is the A1-03 **target**, not a claim that all fields exist
in the read-only v5 wire. v5 currently represents the interpreter as
`{kind:"package_command",package,command}` and does not serialize the
strict Bash `options` or a resolved tool `sha256`; those fields require
an explicit versioned schema/Rust/TypeScript cutover before execution.
The package reference alone does not prove its bytes are already pinned.

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
- The A1-02 v5 grammar cutover makes execution explicit: a recipe
  declares `{kind:"host",access:"unconfined"}` (host filesystem,
  kernel and network access; no remote isolated cache claim) or
  `{kind:"isolated_linux",worker:"buildkit"}` (unavailable until B2).
  Native acquisition uses a provider-backed package, never a fake
  recipe execution mode. Both are descriptions, not A1 executors.
- A target's optional ABI is `gnu`/`musl` for Linux or `darwin` for
  macOS; a missing ABI means *unspecified*, not compatible with a
  declared ABI. `minimum_os` is a bounded `{major,minor,patch?}`
  version value: the producer's floor must not exceed the consumer's
  floor; a consumer with no floor cannot claim compatibility with a
  producer that has one. OS and architecture must agree; host facts
  never silently narrow cross-target authoring.
- A package layout is `{kind:"relocatable"}` or
  `{kind:"fixed_prefix",prefix:"/absolute/install/path"}`. An
  environment may select a fixed-prefix package only with an exactly
  matching declared installation `prefix`; image selection stays
  rejected until B4 can declare/materialize a compatible location.
  Relative, escaping and NUL-containing prefixes are admission errors.

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

- **Retained state:** IR is transient at evaluation, but manifests,
  receipts and journals persist plan-derived values independently.
  Inventory those readers/writers before a no-migration claim; do not
  re-evaluate TS to roll back old generations.
- **Versioned readers:** v3 module maps retain their historical types;
  strict v4 workspaces retain a dedicated typed read-only reader and
  their own schema. v4's `native` recipe and opaque target strings are
  never promoted to v5 execution policy. Current v5 is the only
  frontend writer. The E100 diagnostic names the `3..=5` range.
- **Ambiguity:** legacy scripts with no unique lifecycle classification
  require a source-aware migration diagnostic; A5 owns that cutover,
  not an implicit workspace executor in A1.

## 3. Module ownership and dependency direction

Dependency direction is one-way toward the admitted model; nothing below the line imports anything above it, and store/policy never import BuildKit/Lima/solver/`systemctl`/`launchctl` (master plan §7):

| Module | Owns (A1) | Must not import / own |
|---|---|---|
| `typescript/src/` (emitter) | Authoring API, value construction, span capture, dedent line maps, emit exactly v5 | Execution, network, host observation beyond the injected inputs envelope |
| `crates/gripsack-ir` | v5 typed model and admission, v4 read-only typed reader and v3 module reader, strict version dispatch | Effects, store mutation, or reinterpretation of prior wire meanings |
| `crates/gripsack-ir::sema` (+ new `workspace` sema module) | Catalog collision admission, edge-role/reference/target/cycle validation, executor-capability admission, context admission | Publication, ownership planning |
| `crates/gripsack-policy` (Verus kernels) | Semantic normalization/admission kernels production actually invokes (§5.1) | Serialization, rendering, I/O |
| `crates/gripsack-exec` | Realization coordination against admitted plans; existing supervision for host execution; ownership planner/journal unchanged in meaning | A second planner, a second transaction implementation, per-step scheduling around LLB |
| `crates/gripsack-store` | Durable outputs, receipts, roots, retention; hash inputs reviewed per skill checklist | BuildKit cache as the only copy; changed stored meanings |
| `crates/gripsack/src/commands/*` | CLI surfaces (§6) | New semantics; diagnostic text owned by `gripsack-ir` diagnostics registry |

New code lands as modules in existing crates first (master plan §7); the ~400-non-generated-line split review (Epic A §8.2) applies. No new crate, no public generic backend framework, no proof-only kernel clone.

### 3.1 Callers to migrate at cutover (exhaustive at time of writing)

TypeScript: `index.ts` (40 runtime + 29 type baseline exports; §5.3 inventory), `workspace/{ir,target,validate,commands,files,outputs,emit}.ts` (v5 authoring and emission), `module.ts` (old `module()` → ordinary composition), `steps.ts` (`step`/phase constructors → retired from ordinary authoring; versioned module compatibility only), `graph.ts` (`defineEnv`/`emitIr` emits v5 legacy modules), `resources.ts` (global registry → pure locks), `entries.ts` (file axes), `fetch.ts` (provider resolution/acquisition; `conda.environment` is A3), `verify.ts` (check constructors), `intents.ts`, `conditions.ts`, `inputs.ts`, `pin.ts`, `probe.ts`, `deps.ts` and `cli.ts`. Root authoring API retirement remains A1-09/A1-10/A1-12.

Rust: `gripsack-ir` (`model.rs`, `legacy_v4.rs`, `parse.rs`, versioned `tagged.rs`, `workspace/{catalog,platform,layout,command,file}.rs`, sema, and old module types), `gripsack-policy::graph` (production-used role/closure proofs), `gripsack-exec` (direct apply/update/preview/order E124 guards, `embedded_frontend.rs` regeneration, existing store/lifecycle), `gripsack` CLI (`eval`, `check`, `plan`, `apply`, `update`, `adopt`, `doctor` pin messaging), `gripsack-lint` registrations, `griplint` packs referencing retired authoring names.

E2E/golden: v5 regenerated golden corpus, strict historical v4 schema/parser/serialization fixture, v3 module fixtures, real binary flow tests and installed SDK smoke. Docs/examples: `AGENTS.md`, this plan and four graduated examples (§5.4). The integration owner updates `verification/delivery.json`, checker, `plan/STATUS.md` and CI rules in this same series; no Markdown-only verification claim closes a row.

### 3.2 Executable entry paths and unsupported-capability errors

| Entry (live path) | v5 writer and versioned-reader behavior |
|---|---|
| Internal frontend evaluation (`commands/eval.rs`, used by `check`/`plan`/`apply`) | Sandboxed Deno emits v5; declaration eval runs no build or host effects and needs no fake host file for a workspace |
| `grip check` (`commands/check.rs`) | v4/v5 workspace sema and named-output listing; no personal-profile mutation or builder startup |
| `grip plan` / `grip apply` / `grip update` | E124 rejects v4/v5 workspace execution before effects, in the CLI and direct executor APIs. The first unavailable output is labeled with its capability owner and source span (v4 is historical A5 migration), plus the workspace origin; legacy versioned module maps retain their executor |
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

The integration slice (plan/0049 lineage) owns `verification/delivery.json`, `scripts/check_delivery.py`, runner-evidence admission in `scripts/delivery_evidence.py`, and their calibrated CI gate. `scripts/check_architecture.py` enforces protected crate dependency direction. The Rust gate exercises strict v3 modules, historical v4 workspace and current v5 schema/parser/semantics corpora; TypeScript and real CLI tests cover v5 authoring and versioned rejection. **A1-07 remains partial:** full cross-language golden coverage for every output kind, source-bound lane/case evidence, and the compact module/API cutover remain open. The §3 module map names ownership, not passing evidence for that cutover.

### 5.3 Export/migration inventory (A1-09, A1-10, A1-12)

At the v3 baseline, `typescript/src/index.ts` exported **40 runtime
values and 29 types** (not 27 exports). The table inventories those
baseline symbols and their cutover destinations. The current partial
v5 slice adds workspace exports but has **not** retired legacy root
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
| A1-07 architecture rules + gates (**partial**) | §3, §5.2 | AGENTS/architecture updated to v5 with v4 historical reader; dependency/schema checks mechanically enforced | Ledger/checker + negative calibration owned by integration slice (plan/0049/0051 lineage) — **out of this slice** |
| A1-08 shared commands, context grammar | §2.2 CommandSpec/Task/Schedule | Hostile decoded IR: forged context/edge rejected; typed command ref implies artifact; recipe/consumer/invocation identities separate; schedule inert; executor diagnostic explicit | Context-admission kernel + phase-grant mutant |
| A1-09 ordinary modules, context steps | §2.1, §2.2, §5.3 | Equivalent factory/object forms; moving a declaration between factories preserves recipe identity; no import registration; documented legacy field/verify mapping | Normalization kernel (location invariance) |
| A1-10 compact API + examples | §5.3, §5.4 | Exhaustive export inventory; four examples admitted and type-checked; ordered list vs `deps` distinction; shared command construction preserves check/hook/recipe/task lifecycles and source maps | Conformance suite; lifecycle separation cases |
| A1-11 file contract | §2.2 FileDecl | Origin/content/destination composed independently; equal relative filenames with different origins coexist; template across all three policies; old source roots and lint stages mapped; tree add/remove/collision/escape cases | Rendered-content mapping target named; ownership proof targets named (A2 executes) |
| A1-12 pure locks, SDK split | §2.2 locks, §5.3 | Repeat evaluation and import reordering in one process emit identical IR with no registry reset; scope/key equivalence and conflict diagnostics naming both sites; installed-package exports hide driver/compiler/reset utilities | Lock-identity property; export-surface tests |

## 8. Implementation checklist (when the cutover PR lands)

Per the IR skill, one PR series touching all three parties:

- [ ] `schema/ir/v5.json` added per §2, strict v4/v3 schemas/readers retained; v4→v5 breaking version bump recorded
- [ ] `gripsack-ir`: current v5 model + version dispatch `3..=5`, historical v4 reader round trips its own wire, sema admission and source-aware tests
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
without `hosts/<host>.ts` or `env.toml`. At the A1-01 packet boundary,
Rust sema checked duplicate names with both spans, named
references/kinds, task prerequisite cycles, source spans and illegal
command contexts. §10 records the subsequent A1-02 static-graph
admission packet; the v4 workspace remains read-only. `grip plan`,
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

## 10. A1-02 static graph admission packet (not A1-02 closure)

No wire shape or version changed in this packet: `schema/ir/v4.json`
retains its structural grammar. The Rust decoded-IR reader now
projects named catalog edges into distinct production, build-input,
runtime, task-prerequisite, validation and retention roles. TypeScript
emission uses the corresponding typed roles. Ordered recipe commands
remain local list order; no false build or retention edge is invented.
Both readers reject missing/wrong-kind artifact and check-subject
references, missing exported tool commands, dependency cycles
(including a recipe building with the package it produces), unsafe
artifact selectors and command-environment invocations. Core
diagnostics label declaration spans; emitter graph errors name their
sites. Constructor-time guards are not all source-labeled yet
(A1-06). Publication checks and consumer wiring do not make spurious
production cycles. Only recipes and packages are addressable as file
artifacts; selectors are `.` or normalized relative POSIX paths
(no NUL, absolute path, empty/`.`/`..` segment).

At this first static-graph packet boundary target requirements matched
exactly, and fixed-prefix selection lacked a declared consumer
destination. §11 records the subsequent typed v5 target/layout cutover;
these earlier conservative limitations are not the current wire.

Observed on the Linux worktree after this change: Rust
`cargo test -p gripsack-ir --offline` **59 passed** across five suites;
Deno container **59 passed**; container-built real CLI
`test_workspace_contract.py` **11 passed** (including six new decoded
graph rejections before E124). The final post-edit compose chain
passed Rust fmt/clippy/tests, Deno tests and full real e2e
**257/257**. TLC and Verus final image layers were **CACHED**;
the preceding fresh verify image ran the existing policy kernels
**56 verified / 0 errors** and rejected four named mutants. None of
those existing proofs establishes the new v4 production graph.
At this historical packet boundary, source-bound CI, production
policy adaptation, correspondence and a dropped-validation-edge
mutant were open. §11 records the subsequently connected adapter,
role proof and mutant; full refinement and source-bound evidence
remain open. Neither packet authorizes workspace execution or A1-02
closure.

## 11. A1-02 typed targets and production graph policy (not A1-02 closure)

The strict v4 reader forced a breaking bump to **v5**. Schema,
Rust and TypeScript migrated together while retaining v3 and
historical v4 readers. Recipe execution is now
explicit `{kind:"host",access:"unconfined"}` or
`{kind:"isolated_linux",worker:"buildkit"}`; native acquisition stays a
provider-backed package, not an invented recipe executor. A target's
ABI is `gnu`/`musl` on Linux or `darwin` on macOS, with a bounded
`{major,minor,patch?}` minimum OS. OS/arch/ABI agree and the
provider's minimum floor must not exceed the consumer's; missing ABI
is not a wildcard. A fixed-prefix package declares its absolute,
normalized install prefix; an environment must declare that exact
prefix to select it. Fixed-prefix image selection remains rejected
until B4 supplies a compatible image-location/materialization
contract. Cross-target declarations are checked against their
**declared** requirements, not the checking machine.
Historical v4 package/recipe values round-trip under their own strict
schema/reader (§1.2). Neither the CLI nor direct executor APIs can
mistake v4/v5 workspaces for empty module applies: E124 is returned
before opening the home, lockfile or store. The old v3 module reader
and versioned v4 module envelope remain available.

The typed graph projection now lowers adjacent recipe commands into
local `ordering` edges; those are not catalog names, cache nodes or
build inputs. The same production Rust sema pass indexes named
production/build-input edges and calls the existing verified
`build_closure`, while the new `gripsack-policy::graph::roles` kernel
classifies every edge into build and required-validation decisions.
An omitted producer, publication check or local sequencing edge
fails admission with E131 and source labels. No proof-only
implementation substitutes for that production adapter.

`verification/guarantees.md` records this narrow kernel as
WORKSPACE-ROLES-001; the unproved schema/name-index bridge is an
explicit exclusion, not a completed A1-02 proof family.

Fresh post-edit Linux compose gates passed Rust fmt/clippy/tests,
Deno **61/61**, real CLI e2e **262/262** (workspace cases
**16/16**, goldens **2/2**) and Verus **61 verified / 0 errors**
with **five** calibrated mutants. The model gate passed using a
**CACHED** TLC image layer; this run does not establish a fresh model
check. Rust `gripsack-ir` independently passed **67** tests across
six suites and the direct executor E124 guard passed **1**. The golden
diff changes only `ir_version: 4 → 5` in both fixtures and the
project-workspace package's tagged layout. `npm run build` passed;
an installed Node 24 consumer loaded the packed root SDK, emitted a
v5 workspace and admitted ABI/fixed-prefix selection.
The dropped-validation mutant failed its intended
`required_validation` assertion at `graph/roles.rs`, not a missing
tool or unrelated lemma. Worktree gates do not supply source-bound
CI reports or prove the entire schema→name-index adapter refinement.
No recipe, task, schedule, image or OS worker is executed by A1.

## 12. A1-03 command authoring packet (not A1-03 closure)

The v5 authoring SDK now accepts `exec({argv,…})` or
`exec(program).arg(…).env(…).cwd(…).build()`, and
`runBash({interpreter,body,…})` or
`bash(packageCommand(…)).body(…).env(…).build()`. Builder branches
are immutable; both forms use the same existing v5 IR normalization.
Environment keys are emitted in a stable order, including own
`__proto__` entries, while argv values retain their individual
literal/artifact/package-command boundaries.

`bashBody` captures the template-opening source span. Dedent maps
generated lines to those original script lines, not to the later
`runBash` invocation. Single-line strings remain valid; an unlocated
multiline string is rejected rather than assigned a false map.

Decoded `--ir` callers cannot bypass literal-body admission: Rust
sema rejects `${` with E130, labeling the original script line when
the emitted `line_map` supplies one, else the command declaration.

The interpreter must name an exported command of a declared package;
the command remains a *description*, so hook trigger stages and
explicit recipe host-access policy are not inferred from its body.

The v5 schema and Rust command types are unchanged by this authoring
packet: no new IR field was silently added to a strict reader. The
v5 package-command reference is not a resolved byte pin and the wire
has no strict Bash options field. Both require a versioned cutover
before execution; a JavaScript `${…}` expression is evaluated before
a template tag can reject it, so static pre-evaluation rejection is
also still open. Production normalization proof/calibration and
source-bound CI evidence remain required.

Observed on this Linux worktree: Deno **63/63**, `npm run build`
passed; the real sandboxed CLI exercised fluent exec/Bash
declarations and rejected both frontend and forged decoded Bash
interpolation in **18/18** focused workspace cases. The decoded
Rust regression failed before the E130 fix (accepted the hostile
script) and passed afterward with original-line attribution.
Workspace `plan` still refuses execution with E124 before effects.

The post-correction Linux compose chain passed Rust fmt/clippy/tests,
real CLI e2e **264/264**, and fresh Verus **61 verified / 0 errors**
with **five** calibrated mutants. TypeScript **63/63** ran fresh in
the focused container gate; the final compose TS image reused that
test layer. The model gate also reused a **CACHED** TLC layer, not a
fresh model run. None is source-bound CI evidence or the missing
normalization proof. This packet does not close A1-03 or authorize
execution/release.

## 13. A1 graph-coverage, diagnostics and gate packet (not closure)

The production workspace policy adapter now compares per-output
role cardinalities derived independently from decoded output fields
against the projected edge roles before accepting a closure. A dropped
build input, runtime reference, prerequisite, validation or retention
edge fails E131 with the referencing output span; local ordering and
producer/check presence still have their own source-aware checks.
This count guard cannot prove a same-role name substitution and is
**not** the missing schema→name-index refinement theorem.

The frontend sends authoring failures as the core's existing structured
`Diagnostic` wire. `grip check --json` and the terminal consume the
same codes/messages/labels: typo, wrong output kind, duplicate name,
target mismatch, ambient Bash and dedented-script failures are source
labeled. Snippets are optional and limited to 1 MiB beneath the
evaluated repo's pinned filesystem capability; out-of-repo spans and
invalid coordinates retain their labels without reading file bytes.
JSON errors exit nonzero; operational failures still print stderr and
may have no structured diagnostic. Read-only checks do not provision
BuildKit or a scheduler.

The protected architecture checker now enumerates actual Cargo
workspace members rather than only `crates/`: the `fuzz` member and
future non-crates members cannot silently import OpenSSL. Its self-check
includes a clean control and negative cases for renamed TLS deps,
target-specific protected dependencies and missing protected crates.
CI already runs the checker in the required `test` job; this does not
attest to a live PR or branch-protection setting.

Focused Linux checks passed Rust policy adapter **4/4**, renderer
**5/5**, fresh Deno **62/62** (one wording-only test retired),
`npm run build`, real CLI diagnostic/workspace cases **29/29**, and
the calibrated architecture/delivery checker negatives. The final
post-integration compose chain passed Rust fmt/clippy/tests, real CLI
e2e **275/275**, and fresh Verus **61 verified / 0 errors** with
**five** mutants. Its TS image reused the fresh 62-test layer and
its TLC model image was **CACHED**, not a fresh model check.
Source-bound CI/Mac evidence, remaining capability-owner combinations,
cross-language goldens for every output kind and a production
schema/name-index proof remain open. A1-02, A1-06 and A1-07 stay
**in_progress**; no release.

## 14. A1-06 capability-specific E124 packet (not closure)

`gripsack-ir/src/workspace/execution_gate.rs` now owns the same
pre-effect E124 decision used by the CLI and direct executor APIs.
It names the first declared output's unavailable capability, owner
milestone and declaration span; the workspace origin remains a second
label. An isolated Linux recipe names B2, schedule registration
E2/E3, task prerequisites E1, and historical v4 workspaces are
explicitly read-only A5 migration inputs, never silently promoted
to v5 execution. `grip check` still admits and lists outputs with no
builder; neither version can apply as an empty module map.

The real isolated-worker CLI case **failed before** this change with
only a generic workspace-root E124 and **passed after** with B2 and
the recipe's source line. Rust IR **69** tests and the direct-executor
E124 guard **1** passed; the focused CLI cases for isolated recipes,
inert schedules/task prerequisites and historical v4 **3/3** passed.

The final Linux compose chain passed Rust fmt/clippy/tests, fresh
Deno **62/62**, real CLI e2e **276/276**, and fresh Verus
**61 verified / 0 errors** with **five** calibrated mutants.
The TLC model gate reused a **CACHED** layer. Other capability-owner
combinations, source-bound CI/Mac evidence and A1-06's diagnostic
registry proof remain open. No workspace execution or release.

## 15. Honesty register

- This document began as a design candidate; §§9–14 record read-only
  admission, graph, typed-target, command-authoring and diagnostic
  packets, not a completed A1 milestone or release.
- Plan 0048 §9 NEXT gates bind any public release claiming A1 behavior;
  this plan authorizes no publication.
- A1-07's checker/ledger and protected dependency gate remain partial
  in plan/0049 and their own delivery rows.
- Where older plan 0003/skill unknown-field tolerance conflicted with
  strict v3/v4 readers, this plan and their amended rules record the
  versioned v5 writer boundary (§1).
