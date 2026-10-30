# 0055 — One workspace surface

## Active continuation — 2026-10-28

Owner direction supersedes the earlier product and qualification scope below:

- Finish literal Bash/task authoring first, then **managed container services
  before calendar scheduling**. The owner selected “Services first” after
  distinguishing finite tasks, persistent services and systemd timers.
- Linux/WSL x86_64 remains the only supported platform. Remove Homebrew and
  BuildKit, including source-recipe and application-image authoring/execution,
  helper distribution and dead platform branches. No implicit host-build
  fallback replaces them. OCI artifact transport and running existing images
  are distinct capabilities, not the retired builder.
- A service uses Docker Engine's structured API, an explicitly pinned image,
  workspace-scoped ownership, declared ports/volumes and readiness. Docker
  supervises the process; Gripsack has no resident daemon. Start/stop/status/log
  and task readiness use the same service identity. Foreign resources refuse.
  Stopping, profile rollback and GC never delete database volumes or pretend to
  reverse database migrations. Credentials stay outside captured declarations.
- Continue native scheduled-job lifecycle (E), semantic review/saved plans (C)
  and portable package/environment sharing (D), excluding the retired
  Mac/Homebrew/BuildKit cases. Preserve stored generations, ownership, journal
  authority and artifact integrity. Removed authoring is not removed data.
- Essential gates are Rust formatting/clippy/unit tests, frontend tests and
  examples, ordinary CLI flows, dependency audits, changed-path real runtime
  scenarios and exact release/package/installed-consumer checks. Formal/model,
  exhaustive persistence, broad mutation/fuzz campaigns, duplicate platform
  builds and two-build reproducibility are **optional, explicitly requested**
  checks for substantial releases. Existing proof results remain historical;
  unrun gates are never reported as passed. This standing owner amendment
  supersedes the earlier 17-gate/96-partition release requirement, not the
  runtime safety contracts those checks exercise.
- Release remains coordinated core/SDK/helper, local practical qualification
  and manual unsigned publication; no assumed HP-mini access or new signer.

### Execution ownership and evidence

| Slice | Owner / boundary | Acceptance still to record |
|---|---|---|
| Bash/QoL | SDK command normalization and source mapping | Actual tagged-template task/check/hook execution and invalid interpolation |
| Service runtime | Engine API, resource ownership and native task readiness | PostgreSQL readiness, task use, stop/restart data retention, foreign-resource refusal |
| Backend cutover | IR/schema/SDK, realization and retained-data readers | Native consumers and old-generation rollback/integrity without backend provisioning |
| Scheduled jobs | Profile transaction, captured task revisions, systemd adapter | Install/update/remove/rollback, stale callback and retained invocation roots |
| Semantic review | Admitted snapshots, impact explanations and saved execution binding | Offline comparison and actual unchanged/stale-plan executions |
| Portable sharing | Signed manifests, acquisition/materialization and current trust | Independent recipient, registry/offline transport, tamper/revocation and runtime closure |
| Gate policy | Existing profile/runner and release admission | Essential-only default and truthful optional-gate selection |
| Release/site | Integration owner | Exact artifacts, download/install readback and rendered website |

Implementation is in the isolated `feat/linux-product-handover` worktree from
`799676d`; the older checkout's uncommitted plan edits are preserved. No new
completion or release evidence is claimed by this execution map.

### Development observations — not release qualification

- The current frontend container gate passed 59 tests and checked examples.
  Integration corrected an IR9 driver capability check left behind by the IR10
  emitter cutover; retired string-producer fixtures now exercise native providers.
- A real literal-Bash journey ran nine CLI commands and two task/postcondition
  invocations with the actual interpreter bytes pinned to
  `bc5945feb8bd26203ebfafea5ce1878bb2e32cb8fb50ab7ae395cfb1e1aaaef1`.
  Strict options, spaces in cwd and shell-looking typed environment values were
  exercised without creating a personal generation. Private development evidence:
  `/tmp/gripsack051-bash-0sl0c5yz/`.
- The PostgreSQL guide ran 27 actual CLI/Engine commands against image digest
  `979bccfde35445db618c65107d27d08c6c21339d8dc3ad4c830c6633947be0e4`.
  Healthy startup, unchanged-container reuse, task readiness, SQL data surviving
  down/up into a different container, missing-checkout cleanup and foreign
  container refusal passed. Gripsack created no personal generation and never
  removed the data volume; the fixture explicitly removed its own resources
  afterward. Private development evidence: `/tmp/gripsack051-services-gpfg0vh4/`.
- Actual execution reproduced a missing rustls-provider panic in the new native
  HTTP clients and a nested lifecycle-lock deadlock in package export. Explicit
  shared ring initialization repaired startup; export now reaches its normal
  missing-import refusal rather than deadlocking. The tests do not convert a
  publisher signature into host execution or registration authority.
- The first real systemd fixture found the obsolete “schedule registration
  unavailable” authoring barrier; it is removed. The subsequent real-manager
  journey passed on systemd 255 under fresh passwd account UID 1098, including
  calendar delivery, capture, old in-flight revision completion, replacement,
  disable, rollback, drift refusal and GC. Its five effects and postconditions
  matched (`first` three times, `admitted-old`, `replacement`). Development core
  SHA-256: `02630d5d7d73c24ad99f7abc07e4f906a87ab46c1ea58438ca2ea0db39b56ab5`.
  The owner's schedules were untouched; no sleep/reboot/DST claim follows.
- Booting an unmodified privileged systemd fixture ran its binfmt service and
  cleared WSL's Windows-executable registration. The fixture was killed without
  shutdown hooks; the host's generated `:WSLInterop:M::MZ::/init:P` registration
  was restored, and Windows-backed CI storage observation resumed. The corrected
  fixture masks binfmt service/mount/automount, sysctl, module-loading and other
  global-mutating services; the host WSLInterop bytes stayed unchanged across its
  boot. This is a container isolation limit, not a Gripsack service requirement
  or evidence for sleep/reboot/DST behavior.
- The owner explicitly approved verified scratch reclamation after the 64-GiB
  state-storage floor blocked qualification. Independent hash/inventory checks
  against retained controller archives preceded deletion of 236 duplicate worker
  attempts across 23 superseded candidates: 73,047,932,928 bytes reclaimed.
  Published candidates, all archive/inbox data, active work and four unbacked
  attempts were preserved. Storage admission passed with 102.05 GiB free.
  Receipt: `release-evidence/gripsack/0.51.0/verified-ci-scratch-reclamation-20261029T192018Z/`.
  Retained copies are on the same filesystem, not an independent disaster backup.
- Required private-state and runtime regressions remain intact. Unit execution
  exposed tempfile's permissive directory default in semantic snapshot
  publication; staging now explicitly requests 0700 rather than weakening the
  reader. A changed-executable fixture now changes actual bytes because Alpine's
  `echo` and `false` are the same BusyBox image. No failure is relabeled a pass.
- Protocol4 helper measurement succeeded in one pinned Rust 1.98/musl build:
  `gripsack-conda-0.51.0-x86_64-unknown-linux-musl`, 25,132,360 bytes, SHA-256
  `c547b6a33655be716b6153a0acc468e4de78a932c6ad57f6ab38d5929db5f545`.
  Its source-input digest is
  `a05ced31ffd87b935379236293b594a086225459a970ad58116ec46ef3ddbb67`;
  static admission, protocol smoke and canonical notice checks passed. The core
  pin records these measured bytes. This is not a two-build reproducibility claim.
- Removed the unused IR9 authoring reader and the obsolete B0 Go/BuildKit
  qualification harness, worker-lease/session models, their configurations and
  active TLC/TLAPS dispatcher/catalog entries. Historical reports and actual
  persisted-state readers remain. Retained recovery catalog structure admits
  four roots, 45 proof units and 63 source modules; no prover was run.
- Delivery inventory validation, 19 positive/798 negative CI admission
  calibrations and 141 release-tooling unit tests passed. These unit fixtures
  publish nothing and do not stand in for actual candidate qualification.
- The official example journey exposed an explicit-update bug: a retained v2
  lock kept its old writer version. The writer now promotes to v3 without
  discarding unselected pins and reports format-only changes in `--check`.
  Its regression passed; all three official example locks migrated through the
  corrected CLI with unchanged external selections. Source pin:
  `ed90e26a106782b563b7a392e1bef69c2da96ce2` (feature branch, not yet merged).
- The same preserved development core passed a 26-command semantic/saved-plan
  journey: exact saved apply, stale-source refusal, provenance-only no-op,
  private value omission and source-independent rollback. Evidence:
  `/tmp/gripsack051-semantic-imkwfpep/`. Focused real-CLI regression execution
  also passed 53 task, schedule, service, review and saved-plan cases.
- Real Conda/Python and ABI3 wheel fixtures were prepared through the pinned
  helper. Offline execution passed the two original-archive/Python journeys and
  six extension-wheel cases, including actual native extension execution,
  Python/ABI/architecture/libc rejection and the upstream binary-prefix bound.
  The original overlong-prefix case is retained as an explicit refusal test;
  successful long/spaced-prefix coverage stays within that declared capacity.
- A genuinely separate-filesystem native transfer found `pack` moving a tar
  from the default temporary filesystem into destination-local blobs. The writer
  now creates the temporary archive beside its destination and publishes it
  without clobbering. The rebuilt core passed the real transfer at distinct
  filesystem devices, after deleting both producer and first recipient. Static,
  bundled-loader dynamic, transitive library, dlopen/data, shell/child, argv,
  exit/signal, symlink-entry and moved-root cases passed. Core SHA-256:
  `5ac5818ca4a67c9ffd2fc478df992f7defe4dd62348bb76adc96e586632e0334`.
- The combined imported-tool/scheduler journey passed under a fresh real
  systemd255 account, with only the signed bundle, independent public policy and
  core copied into the recipient container. Producer paths were not mounted and
  the transfer bundle was deleted before use. Calendar/postcondition execution,
  in-flight update/GC, disable, rollback and three current-trust revocation checks
  passed without killing the admitted old run. Package manifest:
  `379d13a229dae7574ed3849a4f18272cb07a728741567fe066bf308d7a57262a`.
- Authenticated HTTPS Distribution 3.1.2 transfer passed with a private fixture
  CA, ordinary TLS verification, wrong/missing credential refusals, independent
  `urllib`/hash verification of the OCI manifest and every blob, exact portable
  manifest/DSSE payload bytes, and actual native execution after sender/cache
  removal. Credentials stayed out of captured output. Private evidence:
  `release-evidence/gripsack/0.51.0/private-registry-ng7za8q5/evidence/attempt-3/`.
  The setup's CA-as-server-certificate mistake was refused, not worked around;
  the fixture also follows the existing read-only-files/writable-directories
  repair/GC policy rather than inventing a stronger permission contract.
- The full container `test` gate passed core/helper formatting, strict clippy,
  Rust tests, architecture and generated-source checks; helper results were
  24 passed and one explicitly prepared-input test ignored. That exposed and
  repaired ordinary PEP508 string-marker warning handling without discarding
  parse-time ambiguity or incomplete-extra refusals. Fresh helper measurement:
  SHA-256 `63beb313012c50a06af562d184124bd8143f148a52af549ab63431a17c55ce3e`,
  25,148,744 bytes, source inputs
  `ea78d708be97efc5573372dec48c328ba373fa136b084894eb6932491f940269`.
  Earlier development Conda journeys used the preceding measured helper;
  final-byte consumer qualification remains separate.

These are uncommitted development observations. Exact-source selected-gate,
artifact, independent-consumer and website evidence must be recorded separately
before publication.



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
- Candidate `983e74561975264a1efd1d1694f68b89dd991b98951ce52c8755e1b77e4058c1`
  (`10110d8`) passed inventory, Rust, GNU, TypeScript, runtime images, dependency
  audits, bridge reproduction and Linux B0. Its helper check correctly rejected
  the pre-review pin after measuring identical new bytes in both builds.
  The updated pin `9c072cc8efeaa6d71aad8ba2647acec336f0cf792a032d5482388911613761b2`
  then passed two independent builds, static/protocol admission and exact pin
  checking from `4144c73`; the helper lock remained unchanged.
- The complete `10110d8` flow image passed 642/644 cases. The remaining wording
  assertion is removed; the other failure exposed an actual native store-audit
  omission, not a stray cache fixture: generation closures retained package
  sources that the deployment-only verifier never hashed.
- Native auditing now reads the existing bounded receipts and retention roots,
  without evaluation or provisioning. Full payload claims cover packages,
  recipes, Conda prefixes/archives, image materializations and OCI exports.
  Unsupported versions, escaped references, conflicting claims and receipt
  digests that disagree with self-identifying bytes block native repair.
  Repairs use direct-child store capabilities rather than ambient deletion.
- Capability tree hashing preserves the existing fixed digest vector and passes
  a held-directory/name-replacement regression. Store/exec library tests passed
  110/110 and 119/119; clippy passed. The corrected integrity/repair/preview
  flow group passed 29/29, including failing-before cases for generation and
  standalone native payloads and ambiguous receipt preservation.
- Actual CLI corruption/repair smokes restored a fetched native payload, the
  real ripgrep Conda prefix and the website's OCI archive. The Conda executable
  and OCI archive returned to their exact previous SHA256 values; the active
  generation stayed unchanged and the image flow created none. Healthy native
  and image inventories audited with deliberately unavailable runtimes/helpers,
  confirming that verification does not evaluate or provision.
- The capability-hashing source `c591f37` produced identical native helper bytes
  in two independent builds: `1026d2f1c7761c072beae615e78d0f9c7fea983979855431c942ec1ce6044e89`.
  Static/protocol checks passed and the helper lock remained unchanged; the core
  pin is updated to these measured bytes before the next qualification snapshot.
- Candidate `e418add7bc519e1ddc9b17a1d8c0e4c3060ea3dd3019224f4f8f5071415e5aa2`
  (`54c1c29`) passed the first 13 practical gates, including static packaging,
  SDK packaging, publication packages and the final helper pin check. Installed
  consumers then refused a stale duplicated site pin before consuming artifacts.
  Consumer and release-evidence readers now take external identities from the
  frozen profile; release-script unit tests passed 128/128 after that cutover.
  Installed-release flows also include native integrity and preview scenarios.
- The normal assertion-enabled `54c1c29` flow image passed all 648 cases.
  Candidate `f94a9ea2021abd7c6a9dfbc327d2204400b881bcd31ffbfb92923dd968b9a14d`
  (`83e4922`) additionally passed packaged documentation examples and 227
  installed-release flows before the root example correctly refused incomplete
  frontend/import pins: the runner had copied the SDK but omitted its locked
  development dependencies. Preparation now seeds those exact archives and uses
  offline `npm ci`, preserving both package manifests and gripsack.lock.
- A continued real-artifact smoke completed the corrected root consumer,
  frozen Conda build/run/task/apply and retained reuse, plus both `.conda` and
  `.tar.bz2` archive consumers and their negative controls (62 commands).
  The archive fixture now uses `pixi.fromLock`; no retired flat alias remains
  in active consumer declarations. No lock refresh or public SDK substitution
  was used to turn the refusal into success.

The observations above are development history. The exact-source release result
is recorded separately below.

## Published 0.50.0 — 2026-10-28

[Release](https://github.com/gripsack-dev/gripsack/releases/tag/core-v0.50.0) is
public/latest, with short product-first notes and 15 immutable manual-install
assets. Core and SDK are 0.50.0; the authoring contract is workspace-only IR9.
The qualified source is `d060e1fef80f83212210e66bf65a8f1d62631bd0`, merged in
[PR198](https://github.com/gripsack-dev/gripsack/pull/198). It removes **37,496
lines net across 470 files** relative to its main base.

- Candidate `0bd892ef25778650c2808c1608b3095027e780a49c47f16c354320de126fd89f`
  passed all **17** practical gates, **648** normal flows, **227** installed-release
  cases and **96** persistence partitions with both drift states.
  [Qualification receipt](https://github.com/gripsack-dev/gripsack/releases/download/core-v0.50.0/gripsack-0.50.0-local-qualification.json)
  SHA256: `ed0d8c9be9650c2591b3c7423f89ea24fa542de4ba1bd1b2bbd4e5c27a3ce937`.
- All **15** uploaded assets passed byte-for-byte GitHub download readback before
  publication. The downloaded core passed isolated init, approval, preview, apply,
  unchanged-generation reuse, changed-source refusal, rollback, store verification
  and retired IR8 refusal. Published tags and asset bytes were not rewritten.
- Cold public helper provisioning passed without overrides. The frozen Conda
  consumer ran ripgrep 15.2.0 and reused its generation. The actual website
  BuildKit recipe ran natively and as an independently inspected OCI image in
  Docker; retained native execution survived owned worker/cache removal.
- The published [example](https://github.com/gripsack-dev/example-env-typescript)
  installed the GitHub SDK through `npm ci` with a fresh cache, passed TypeScript
  checking and real `grip check`, and left its manifests and locks unchanged.
  [PR6](https://github.com/gripsack-dev/example-env-typescript/pull/6) carries the
  qualified workspace-only sources.
- The product-first [site](https://gripsack.dev/) is live from source
  `4a157e3a36c2f6e891e7beb3f3a8f8240259b853`
  ([PR26](https://github.com/gripsack-dev/gripsack-dev.github.io/pull/26)).
  Static commit `86d3e8021fb735dfdd3ff319ea62984e1497e6b3` deployed through the
  [upload-only Pages run](https://github.com/gripsack-dev/gripsack-dev.github.io/actions/runs/40718329787).
  Live desktop/mobile, dark/light and navigation were exercised: **18** pages,
  **519** internal references, no broken internal anchors, no browser errors and
  no horizontal overflow.

This is the owner's selected Linux/WSL x86_64 practical, unsigned publication
style—not a registry publication or a new formal-proof claim. The original
handover mapping and next product work remain as recorded above.
