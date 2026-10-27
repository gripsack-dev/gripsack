# 0055 — One workspace surface

Owner direction: 2026-10-27. Gripsack is an alpha product with one owner;
obsolete authoring compatibility is not a product requirement. Remove it,
rather than adding another migration layer. Keep releases small and practical.

## Contract

- IR9 has exactly `ir_version`, injected `host` facts and one required
  `workspace` catalog. Old IR readers and module envelopes are removed.
- `gripsack.ts` is the only evaluated entrypoint. Ordinary TypeScript composes
  workspace packages, recipes, environments, profiles, checks, tasks and images.
- Remove `module`, `defineEnv`, the universal step pipeline, legacy destination
  helpers and legacy Pixi/Brew fetching, including callers and obsolete tests.
  Keep `conda.environment` and explicit `pixi.fromLock` interoperability.
- Retained generations, ownership receipts, priors and recovery journals are
  data, not an old authoring API. Preserve their meaning and rollback/GC safety;
  never erase state or deserialize old authoring bytes as the new contract.
- The website teaches the product: user-space environments, native packages,
  dotfiles, BuildKit source builds and OCI images. Historical migration and
  release evidence belong on GitHub, not in the main product narrative.

## Responsibility map

| Owner | Boundary |
|---|---|
| IR/schema | One current envelope, tagged admission, workspace semantic checks |
| TypeScript | Workspace-only authoring/driver, provenance, current examples |
| Executor/store | Workspace realization and one existing profile transaction; retained-state readers |
| CLI/config | Workspace init/adopt/check/plan/apply/update/run/build and current settings |
| Fetch | Native HTTP/Git/file/plugin acquisition and required helper provisioning |
| Website | Product navigation, capability guides and concise installation |

No compatibility adapter translates module declarations into workspace outputs.
Existing safety and lifecycle tests move to workspace scenarios; tests whose
only purpose was the deleted authoring surface are removed.

## Original handover

This finishes the API retirement requested by A1-09, A1-10, A1-12, A5-01,
A5-05 and A5-06 in the edition-5 frontend/ecosystems epic. Runtime completion
and handover proof closure remain distinct. The next substantive product work
is Linux scheduled-job lifecycle (E), semantic change/saved-plan review (C),
and portable artifact sharing (D). Existing workspace/Conda/BuildKit behavior
must not be mistaken for missing implementation just because old ledger rows
remain pending. Capture exclusions and focused review already shipped in 0.48.

## Verification and release

Target: coordinated core/SDK 0.50.0, IR9, Linux/WSL x86_64. Use the owner's
existing local practical qualification and manual unsigned publication style;
no new signing infrastructure or hosted duplicate build is a prerequisite.
Verify the changed CLI paths, retained-state lifecycle, actual SDK/docs examples,
BuildKit/OCI consumers and rendered desktop/mobile website. Keep public release
notes short; record actual execution results here when available.

`REL-PRACTICAL-050-2026-10-27` records the owner's request to ship 0.50 using
the same practical release style: the 17 local gates and complete 96-partition
Linux persistence campaign remain. Formal tools remain a separate programme;
this release does not manufacture proof completion or a signature. No fuzz or
saved-corpus replay is introduced. The obsolete module scheduler's Loom gate,
13 exclusively retired policy obligations and its latch mutant disappear with
their dead implementation, not as passing checks.
