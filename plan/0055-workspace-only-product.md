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

| Slice | Owner / boundary | Qualified behavior |
|---|---|---|
| Bash/QoL | SDK command normalization and source mapping | Actual tagged-template task/check/hook execution and invalid interpolation |
| Service runtime | Engine API, resource ownership and native task readiness | PostgreSQL readiness, task use, stop/restart data retention, foreign-resource refusal |
| Backend cutover | IR/schema/SDK, realization and retained-data readers | Native consumers and old-generation rollback/integrity without backend provisioning |
| Scheduled jobs | Profile transaction, captured task revisions, systemd adapter | Install/update/remove/rollback, stale callback and retained invocation roots |
| Semantic review | Admitted snapshots, impact explanations and saved execution binding | Offline comparison and actual unchanged/stale-plan executions |
| Portable sharing | Signed manifests, acquisition/materialization and current trust | Independent recipient, registry/offline transport, tamper/revocation and runtime closure |
| Gate policy | Existing profile/runner and release admission | Essential-only default and truthful optional-gate selection |
| Release/site | Integration owner | Exact artifacts, download/install readback and rendered website |

Implementation merged in [PR200](https://github.com/gripsack-dev/gripsack/pull/200);
the older checkout's uncommitted plan edits remain untouched.

### Published 0.51.0 — 2026-10-31

[0.51.0](https://github.com/gripsack-dev/gripsack/releases/tag/core-v0.51.0) is
public/latest, manually distributed as unsigned GitHub assets. Its immutable
tag names qualified source `3332429b2b92392e267c1df28f673fa798d43a02`, not the
later integration merge `375fcddf19ae21e3f7ba8cf42cd81f922d5fb868`.

- Candidate `8c0bc53c23bfc815c727b334d1279c2250e558e9de51fa68fe64015b45c26aa4`
  passed all twelve selected gates: 706 ordinary CLI flows, 227 installed-release
  cases, exact packages, dependency audits, documentation and frozen consumers.
  Original private controller receipts were re-admitted after downloading the
  actual content-addressed bundle from GitHub.
- Packaged core SHA-256
  `8ddc3ba79dba5ef20004190fca46b5eb0470f74e8c2f0d4844dbb0dadd7b0fd0`
  passed all twelve marked native cases and the Bash, PostgreSQL, semantic,
  independent-filesystem, authenticated TLS registry and real systemd journeys.
  [Runtime evidence](https://github.com/gripsack-dev/gripsack/releases/download/core-v0.51.0/gripsack-0.51.0-runtime-evidence.json)
  records exact hashes and the smoke's corrected terminal-receipt wait.
- All thirteen uploaded assets passed byte-for-byte download readback before
  publication. Subsequent anonymous public downloads and a fresh 17-command
  consumer verified SDK installation/typechecking, cold default-helper
  provisioning, frozen Conda execution/profile use, and helper-unavailable
  retained reuse without changing manifests or locks.
- [Example PR7](https://github.com/gripsack-dev/example-env-typescript/pull/7)
  merged at `7c1e2278f33935049f343b9b3494000baca585a6`; its public SDK
  typecheck also passed in run `41184250933`.
- [Website PR27](https://github.com/gripsack-dev/gripsack-dev.github.io/pull/27)
  merged at `b2bc6183980b68336a77d4aa8f9663c141ad9b11`. Static commit
  `0d035eb4b98f9ea6b4b52d5d94df5b11808ef918` deployed in upload-only
  [Pages run 41184337645](https://github.com/gripsack-dev/gripsack-dev.github.io/actions/runs/41184337645).
  Live desktop/mobile navigation and light/dark themes passed: 22 pages,
  706 internal references, no broken anchors, browser errors or horizontal
  overflow. Code links name the exact released source.
- Owner decision `REL-051-QUALIFIED-ARTIFACT-2026-10-31` separately permits the
  verified smoke-only fix and evidence/docs admin merge. No product code changed
  after qualification; no passing status is attributed to that later head.
  Optional deep/disruptive campaigns, registry publication and signed-installer
  admission are not claimed.
- Completed registry volumes and the safe scheduler filesystem were archived
  and read-back verified on Seagate before their three owned containers were
  removed. WSL's interop registration stayed unchanged. Receipts are under
  `D:\CI-Archive\gripsack-release-0.51.0\20261031T082527Z`.

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
- Frozen source `75269f4afb277fe9c5bfd2ce87ed6f3c6f853ead` entered candidate
  `bda22db687347c7dc6aa62d5bb66eefecefa0478450e7cedf8719596822529ea`.
  Inventory, Rust, TypeScript and runtime-image gates passed; dependency audit
  refused ZIP 6 under RUSTSEC-2026-0367/0368/0369. Later gates were not admitted.
  Both direct ZIP consumers are migrated to ZIP 9, without UV vendoring or
  audit-policy changes. Paired remediation audits passed at advisory revision
  `ee1a49a2769a24a596afc1c20cb33748d2769586`; the compatible helper Tokio update
  removes its separately reported I/O-safety warning. New source qualification
  and helper measurement are required, not a reused successful candidate.
- The 26-node shared-DAG timeout was repaired after profiling a preserved cold
  update: 4,103 CPU samples identified repeated signature/payload verification.
  Operation-local reuse binds the home, policy revision, receipt, target,
  budgets and retained input snapshots; mutations poison the scope and a new
  operation admits afresh. Shared snapshot roots avoid exhausting descriptors
  on a 256-member runtime cycle. The preserved cold update completed in 23.871
  seconds under its unchanged 120-second deadline, and all 13 projection
  journeys passed, including independent graph orderings and non-root exports.
- Python fixture admission correctly refused missing X11 runtime and an
  omitted GNU ABI. Its actual package request now owns X11 and declares glibc
  2.28/Linux 4.18; the signed runtime and enclosing environment retain that
  requirement. Those errors are not waived or converted to ambient host lookup.
- Patched helper measurement is 25,189,704 bytes, SHA-256
  `60d66f94ef93fa36d3de6de7bf9b264b3e4d0ef16fc547791218f8849330ea38`,
  with source inputs
  `971f8f393ca26612059d4fd41454687e6bb5ba5107369ff51ec6d97dc43d6d76`
  and lock SHA-256
  `b5f7422cf43a5b574f14e44b54ff3df3c0495bade18be489e3fff52dc4b07003`.
  The core pin matches this single measured build. All eight real offline
  Conda/native-wheel cases passed with those bytes; this is not a two-build
  reproducibility claim.
- The full container Rust/helper gate passed after scoped-readmission changes.
  The mixed local environment journey passed both independent recipients,
  executable Python/native/wheel use, profiles, tasks and GC. An unoptimized
  manual development binary exceeded the external variant's 120-second limit:
  4,074 CPU samples and a cold file-read trace attributed the cost to SHA-256
  and distinct archive/tree validation phases. The repository's ordinary e2e
  profile already retains debug assertions/overflow checks with optimization.
  The exact optimized candidate binary, SHA-256
  `6ce06d5b13443a859c3a96a4f890674de7ba921e41a5a1ca5f583827da950c2e`,
  passed the complete external journey with all original assertions and limits.
  Its two recipients' first Python commands took 25.020 and 14.032 seconds;
  the full journey took 276.218 seconds. No additional cache, weaker admission
  or raised timeout was introduced. Evidence:
  `release-evidence/gripsack/0.51.0/dag-profile-yr75cf5u/external-release-result.json`.
- Source `553298f3a6cf2b1c6ec9b07050b807a739a12b30` entered candidate
  `1a2df07b09fa6b9d441e88d9dc52fc3b96d048dea2950cf3c7dd035e8bef6b10`.
  Its first ten gates passed; installed-consumer qualification found stale
  documentation example classifications, so flows remained unrun. The corrected
  checker removes the retired builder example, admits complete Bash/service
  examples under IR10, and explicitly classifies partial schedule/provider
  snippets. The obsolete IR999 mock-emitter echo calibration is removed rather
  than repinned; real packaged-SDK evaluation, core admission and missing-return
  refusals remain. All ten complete examples passed those real checks; four
  fragments are classified, not claimed executed. New complete qualification
  remains required. Core PR200 and website PR27 remain drafts; no public tag,
  asset or deployed-site claim follows.
- The optimized candidate passed literal-Bash execution (nine commands),
  PostgreSQL ownership/readiness/data retention (27 commands), semantic/saved
  plans (26 commands), and the real systemd255 imported-tool lifecycle with
  three current-trust refusals. Its signed static-tool manifest is
  `1982abd94e9077bb4df310c0676aeca4de417a71d8f546a83516d27ce8dadb5e`;
  independent OpenSSL DSSE verification and authenticated private-TLS registry
  transfer/independent blob readback/actual execution also passed.
- The complete marked native lane passed 11 cases but exposed a fixture
  assumption: under a shorter pytest base directory, its supposedly overlong
  prefix was 253 bytes and correctly imported. The refusal case now adds a
  separate bounded path component, independent of the sandbox's root length;
  that exact case passes under the same short root. No product rejection was
  weakened or added to make the original valid path fail.
- Owner-authorized storage cleanup preserved verified Seagate checkpoints,
  source/uncommitted changes, release artifacts and failure evidence. The
  current task's compiler cache released 24,341,135,360 allocated bytes; three
  older compiler caches released 201,628,319,744 bytes, retaining mapped editor
  libraries and Cargo locks. The Seagate checkpoints total 521,376,262 bytes
  and passed content comparison before deletion. Ubuntu reached 243 GiB free;
  online trim reported 266.4 GiB discarded, but its nonsparse Windows VHDX did
  not shrink. Receipt:
  `release-evidence/gripsack/0.51.0/old-compiler-cache-reclamation-g93hq574/receipt.json`.
- The owner grants ongoing bounded cache cleanup and verified archive
  relocation under `STORAGE-HYGIENE-2026-10-30`, now recorded in `AGENTS.md`
  and the release handoff. Docker-managed caches belong on B:, but a host
  `target/` bind writes into Ubuntu on C:. No blanket Docker/volume prune or
  storage-floor reduction is authorized.
- B:'s nonsparse Docker data VHDX subsequently crossed the unchanged 128-GiB
  host floor while a separate STROP build was active. Exact-ID removal of 22
  inactive, private, immutable Gripsack compiler/checker cache records reclaimed
  50.66 GB inside Docker but zero Windows bytes. Actual storage admission still
  refuses; no other build, container, volume or daemon was interrupted. Safe
  offline VHDX compaction remains separate from guest-cache deletion, and no
  successful new candidate or release is claimed.
- The full ordinary development flow run completed with 698 passes and nine
  failures. It found one real cold-store regression: apply tried to reuse an
  absent previous semantic artifact before publishing its fresh capture.
  Only the optional reuse path now treats an explicitly absent store/root as a
  miss; history/audit readers and existing malformed, incomplete, nonprivate,
  symlinked or incorrectly bound artifacts remain strict. The remaining failures
  were stale lock-format/example/platform/capability assumptions and a fixture
  which corrupted the first store object instead of the deployed payload.
- Integrated recovery core SHA-256
  `fa7cb1d1a81f6380194bce0bcf9ab6be069eafdae2985ab1e8802483afe1250b`
  passed formatting, strict workspace clippy and all workspace Rust tests in
  the pinned container. Its six cold-store/journey/repair cases and a separate
  26-command semantic/saved-plan smoke passed; 21 migrated diagnostic/example
  cases also passed. The complete ordinary-flow rerun then passed all 706 cases
  with 12 explicitly marked native cases deselected. Compilation
  used the Docker-managed `gripsack-dev-rust198-musl` volume, not a host target
  bind. The measured helper's source-input digest is unchanged.
- The completed profiler fixture was archived to Seagate before removal.
  An initial Drvfs readback failed with ENOMEM; no source was removed then.
  Native Windows file I/O subsequently verified all 80,109 archived entries
  against a fresh paused-container export, including hashes, modes, links and
  modification times. Archive SHA-256:
  `e0eeb4729a61587e13fb4d1a680f58f8b8fc4e6d28540577e06b3c870282c771`.
  Verification inventories and the deliberate teardown receipt accompany it;
  bind-mounted evidence remains separately retained.
- The disposable systemd fixture and both registry data volumes were also
  checkpointed and read back before their owned containers were removed.
  Scheduler teardown left the host WSLInterop bytes unchanged. The unrelated
  Docker build and controller containers were not stopped.
- After the owner's manual Docker-data compaction, Windows B: free space rose
  from 124,474,019,840 to 175,293,255,680 bytes. The VHDX shrank by the same
  50,819,235,840 bytes; actual `storage-check` now passes without reducing any
  floor. Normal-user Docker startup retains engine identity
  `9be6eb8e-4477-4485-9cf1-4fa32d04a4fe`, and an existing container executed the
  preserved `grip 0.51.0` build. The selected helper gate explicitly re-admits
  the unchanged measured tuple rather than compiling identical helper inputs;
  fresh static/protocol/license and installed-consumer admission still apply.
- Candidate `9f0d7978eb2f772f2607ee171a3dc123e7cd722044388525b4ef1c1e6fa8bbc9`
  passed all twelve selected gates for source
  `33cd2c1716f54d0e8b611582af8b61c490fc387f`. Final assembly then found an
  operator check tied to the parser's former `version != ...` spelling rather
  than its current equivalent `match`. Only that incidental syntax predicate
  is removed; public version declarations and the actual IR/consumer gates
  remain required. The corrected operator assembled and independently admitted
  the real frozen candidate, and all 141 release-tooling tests passed.
  The corrected source must be qualified afresh before publication.
- Requalification reached the physical B: reserve again after the first full
  run. With 118.2 GiB physically free and 152.5 GiB available inside Docker,
  the owner explicitly chose `STORAGE-FAST-96-2026-10-31`: a temporary 96-GiB
  physical B: floor until the 4-TB replacement. The system 128-GiB and state
  64-GiB floors and all twelve gates remain unchanged. Actual storage admission
  passed after that single-field policy change; the old policy and exact delta
  are retained in the recovery evidence. Restore B: to 128 GiB on replacement.
- The packaged core from candidate `9f0d7978...`, SHA-256
  `f69ff3e8dd3c442d14413b79e0c3ba06767adc813710533535a3b52a4166e628`,
  passed all twelve marked native-integration cases together, including the
  corrected short-base overlong-prefix refusal. Standalone literal-Bash and
  semantic/saved-plan journeys also passed with those exact bytes. Later source
  qualification may reuse these runtime observations only if its final
  executable is byte-identical; this is not a passing new-source claim.
- Candidate `29232f4640b810555f2eeedc4fe1f3f51bf7cce657c2bd1a78932907d168f965`
  passed its first eight gates, then correctly refused stale generated installer
  payloads in `publication-packages`. The compatibility-check repair also changes
  the embedded verifier: both installers are regenerated from their reviewed
  modules, and the actual entrypoint's help command and freshness check pass.
  This changes compiled core bytes; final-byte runtime scenarios must run again
  after the new source is qualified. No failed gate is waived or receipt reused.
  The prior `f69ff3e8...` core also passed PostgreSQL persistence, imported-tool
  systemd scheduling, authenticated TLS registry consumption and independent
  executable-filesystem relocation after removal of the producer and first recipient.
- Candidate `8c0bc53c23bfc815c727b334d1279c2250e558e9de51fa68fe64015b45c26aa4`
  passed all twelve selected gates for source
  `3332429b2b92392e267c1df28f673fa798d43a02`: 706 ordinary flows and 227
  installed-release cases. Its packaged core, SHA-256
  `8ddc3ba79dba5ef20004190fca46b5eb0470f74e8c2f0d4844dbb0dadd7b0fd0`,
  also passed all twelve marked native cases, Bash, semantic/saved-plan,
  PostgreSQL persistence, private TLS registry and independent-filesystem smokes.
  Assembly, independent validation and private controller re-admission passed.
  Private admission initially refused the installation root's 0755 mode; only
  that operator-owned directory was restricted to 0700. Its already-private
  configuration and all admission checks remained unchanged.
- The real scheduler reached success, but its smoke observed a postcondition
  file before the terminal invocation receipt was committed. The retained
  status and manager both report success. The fixture now waits for both
  observations within its original deadline, rather than taking a racy receipt
  snapshot. A fresh real systemd255 recipient passed the complete imported-tool
  lifecycle, admitted-old completion and all three revocation refusals with the
  same `8ddc3ba7...` core. The fixture repair still requires new exact-source
  qualification; runtime evidence is reusable only for byte-identical artifacts.
- The owner selected `REL-051-QUALIFIED-ARTIFACT-2026-10-31`: publish the fully
  qualified `3332429...` artifact and merge only the verified smoke fix and
  evidence/docs under a scoped admin override. The follow-up submission stopped
  before creating a candidate when B: had 75.9 GiB free against its 96-GiB floor.
  No product source changed after qualification. The release tag, manifest and
  successful qualification status remain bound to `3332429...`; the follow-up
  head is not reported as fully qualified, and no storage floor is lowered.

These development observations span preserved intermediate binaries and source
checkpoints. Exact-source selected-gate, final-artifact, independent-consumer
and deployed-website evidence must be recorded separately before publication.



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
