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

## Integration observations

- The main cutover checkpoint `f0b7c9b` removes 38,585 lines net across code,
  schemas, fixtures and documentation. Retired native Mac/Lima implementations
  and the obsolete module-linter coordinator are also removed; data-only
  readers needed by retained records remain.
- Development CLI smoke exercised workspace init, explicit approval, preview,
  apply, no-op generation reuse, changed-source refusal, rollback, adoption
  with private permissions, and store verification. A separate published-0.48
  producer created real legacy generations; 0.50 restored both and performed
  an ordinary workspace transition without losing their bytes or 0600 modes.
- The website's actual BuildKit example built and ran natively, exported an OCI
  archive with independently checked descriptors/DiffIDs/program bytes, and ran
  in Docker. Native output still ran after owned worker/cache removal; neither
  build created a personal generation.
- Actual native, Conda and explicit Pixi-lock consumers ran. The imported Pixi
  environment stayed at its locked ripgrep 15.1.0 while the independent solved
  environment used 15.2.0; foreign lock bytes and the active profile were unchanged.
- Nine complete website/README examples passed packaged-SDK emission and real
  core admission with negative calibration; one task extension is explicitly a
  fragment. Desktop/mobile navigation, dark/light rendering, 519 internal link
  references, reduced-motion scrolling and script-free content were exercised.
- The first complete flow run passed 548 cases and found 84 failures. Fixture
  migrations are corrected rather than bypassed. It also exposed real captured
  file-alias, directory-transition routing and source-diagnostic defects; their
  existing behavioral regressions remain and the native boundaries are repaired.
- The corrected binary executed a captured absolute file alias. A published-0.48
  directory producer then exercised ordinary workspace transition, leaf-to-directory
  growth and explicit rollback; foreign sibling content and the old immutable tree
  survived. Store verification passed afterward.
- An unreceipted native file-to-directory replacement remains safely refused.
  The same isolated scenario also fails on published 0.48 without changing its
  generation; this is a pre-existing directory-ownership gap, not widened here.
- Focused native-contract and lifecycle reruns now pass all 161 selected cases
  across the two groups, including the corrected executable fixture, current
  lockfile lifecycle and nonpublishing survey. Captured-content unit tests
  passed 8/8; release-script unit tests passed 128/128; Rust formatting passed.
- Frozen candidate `02ff5ef186462f2972f4c70b5af4989274518f63ed3a25d6245fff8e141e83e7`
  (`fa35190`) passed inventory and stopped in the Rust gate at an obsolete
  positive Mach runtime test. That test is removed with the retired parser,
  rather than restoring unsupported platform code.
- Focused review found two introduced defects: typed IR errors lost JSON
  positions, and directory growth could restore a prior before failing and
  compensating. Error-only re-decoding preserves positions without parsing
  valid IR twice. Directory evolution now admits every blocker before any
  removal and requires the intended result to be absence.
- The held-inode regression fails before that repair. Actual corrected CLI
  runs refuse the prior-carrying case without changing inode, mode, bytes or
  generation; a separate two-blocker growth/rollback succeeds and ordinary
  pruning restores the private 0600 prior. Malformed IR again reports its JSON
  line/column. Updated exec/IR library tests passed 119/119 and 26/26.
- The reviewed directory/retained-state flow group passed 31/31. Candidate
  `4b7efded87489ebc241444e2f51af532e13874afe3dd89c096cb34b7ec2e77e4`
  (`0924bb6`) then exposed a stale HTTP adapter name in the practical runner,
  not a relaxed transport check. The runner now names the current tool-artifact
  TLS refusal. HTTP (13 properties/4 mutants), process bounds (35/10), and
  update survey (3/7) all passed their positive and negative calibrations.

These are development observations, not a relabelled qualification receipt.
The frozen 0.50 release must still complete its practical profile.
