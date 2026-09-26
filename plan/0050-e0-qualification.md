# 0050 — E0 qualification: task/schedule scope, vocabulary and proof targets

Status: E0 record · Owner: implementation agent · Date: 2026-09-24
Scope: Epic E (tasks_schedules closure scope) · Source: bundle edition 5,
`gripsack-tasks-scheduling-epic.md`

## E0-01 — API/scope/ownership reconciliation

### Current source → target model mapping (at 176eaec + plan/0049)

| Epic E concept | Current repository surface | Gap to A1 contract |
|---|---|---|
| `CommandSpec` (`exec`/`runBash`) | `typescript/src/steps.ts` argv/shell steps; `gripsack-exec/src/verify.rs` runs `/bin/sh -c` | A1-03/A1-08: shared immutable command values, pinned Bash, strict options, source maps; strict Bash is a new semantic choice, POSIX scripts migrate explicitly |
| `Recipe<Outputs>` / `ArtifactRef` / `Package` | `gripsack-exec/src/module/produce.rs` sequential staging; `identity.rs` payload/recipe identities | A1-02/A1-04: typed outputs, distinct digests, production separate from deployment |
| `TaskSpec`/`TaskInvocation` | none (closest: `DevelopmentTask` naming reserved) | A2-P first single-command task; E1 finite graphs |
| `EnsureArtifact` (`task.build`) | none | common realization service (A2/B3), never a recursive CLI call |
| Local step | `step`/phase pipeline (fetch/build/install/config wrappers) | A1-09/A1-10: retired universal pipeline; ordered lists normalize to sequencing edges |
| `EnvironmentSpec` | `defineEnv` host-selected modules (`typescript/src/graph.ts`) | A2-P: process-scoped selection, no personal activation |
| `ScheduleSpec` | none | E2/E3: inert declaration; systemd/launchd translation |
| Locks (`lock("key")`) | `resource()` global registry + string lookup | A1-12: pure typed refs, scope/key normalization, no import registration |
| Scheduler | `PureScheduler` proved kernel wired into `run_all` (plan/0047) | retained as outer lifecycle coordination; BuildKit receives whole subgraphs (B5-01) — no per-vertex scheduler around LLB |

### Eight scenario families (Epic E §10) — registered owners

1. One source-produced tool, two invocations, two postcondition executions → E4-01/E1-03
2. Task prerequisite + inferred artifact requirements → E1-02
3. One compatible package through project/profile/image/task consumers → A2-01, B3-03, B4-01 (full closure at E5 after B5)
4. Timing/argument/dotfile edits without unrelated producer invalidation → E4-03, A1-04
5. Retained scheduled tools after worker/cache loss → E3-04, B3-02
6. Incompatible execution target rejected before registration → E4-02, E2-01
7. Shared command authoring, build vs host context, invalid cross-context refs rejected → A1-08, B2-06
8. Ordinary module composition without import side effects or cache boundaries → A1-09, E1-07

Required failure journeys (also §10): lost manager, malformed translation,
failed prerequisite/postcondition, overlapping requests, supervisor death
with surviving child, disabled/retired callback, stale profile plan,
foreign registration, interruption between publication and registration,
corrupt task record, missing credentials, bounded log flood.

### Result/exit vocabulary (frozen for E1)

`failed`, `blocked`, `cancelled`, `skipped-busy`, `indeterminate` —
skip is never successful prerequisite completion; indeterminate covers
unknown-outcome effects (no auto-retry, no exactly-once claims).
Success is `succeeded` only with declared postconditions passed.

### Budgets and boundaries (v1)

- Finite static graphs only; cycles rejected with causal path + both declarations.
- Locks: complete statically declared set, canonical order, all-or-release, held through verification; `onBusy: skip` → `skipped-busy`.
- Scheduled invocations: closed stdin, noninteractive, bounded stdout/stderr/logs, declared timeout covering verification.
- No resident timer daemon; no BuildKit as task runtime; no second store; no root/system scope, no implicit linger; daily/weekly local-calendar only; `missed: native` with Linux persistence explicitly false; named timezones/cron syntax/intervals rejected in v1.
- Prohibition: never alter the developer's real schedules or clock in tests; disposable fixtures only.

### Production proof targets (E1/E3 rows, from the master plan's table)

- Verus: context/edge admission, readiness/conflict, active-revision decisions (production adapters, not clones).
- TLC: manual/timer/update/disable/GC interleavings on the shared lifetime model; focused TLAPS active-revision and protected-invocation-root invariants (TLAPS lane: repo has TLC+Verus runners; TLAPS is new infrastructure to stand up — recorded as such, see guarantees ledger).
- Calibration mutants: stale-launch, early-root-release, omitted-edge, false-success, conflict-admission.

## E0-02 — OS scheduling qualification

The **systemd-linux lane alone is verified** at source `0be1eaa`.
`verification/reports/2026-09-26-e0-systemd-0be1eaa.log`
(SHA-256 `932a36f8a76dee63bda0adbe85b9f8825ec41c3754d9c712aa3a61fa6a84b323`)
embeds the disposable fixture source and actual systemd 255
user-manager outputs: UID 1000, Linger=no; `daily` normalized to
local `*-*-* 00:00:00`; a unique transient user `.timer` with
`AccuracySec=1s`/`Persistent=no` bound a finite `.service` with
`Restart=no`, fired **1.007 seconds** after the requested local
calendar second and exited successfully. `--collect` left both
units `not-found` and no timer-listing residue; no persistent
user unit, other schedule or clock was changed. Three named Linux
checks pass. The previous 2026-09-24 Markdown-only observation
remains historical and cannot qualify this newer result. This is
an OS-manager capability fixture, **not** the future E3 Gripsack
registration/runtime or a sleep/reboot/DST claim.

launchd/macOS lane: **blocked** — no Mac hardware or runner on this
workstation. The lane stays open; Linux results never qualify it.

## Record

| Item | Status |
|---|---|
| E0-01 inventory (this document) | implemented_unverified (record complete; awaits owner review + A1 contract landing for the mapping to bind) |
| E0-02 systemd lane | verified at `0be1eaa`: real transient user-manager calendar trigger, bounded observation and zero residue in SHA-256 runner report |
| E0-02 launchd lane | blocked (no Mac) |
| Next | Obtain actual native Mac launchd user-agent evidence for E0-02; E0/E1 stay open until A1/A2/A2-P and other requirements. B0 Linux harness is separately source-bound but B0 required CI/Mac-VM lanes remain open |
