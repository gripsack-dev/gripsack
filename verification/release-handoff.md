# Practical qualification and manual release handoff

Standing owner policy: `REL-PRACTICAL-051-2026-10-28`, qualification authority
`LOCAL-PRACTICAL-QUALIFICATION-2026-10-28`. This supersedes the historical
all-gates handoff and the 0.50 requirement to run 96 persistence partitions.
See [plan 0048](../plan/0048-review-response-0.42.0.md) for safety invariants and
owner amendments and [plan 0055](../plan/0055-workspace-only-product.md) for the
current product scope. Historical reports remain historical, not evidence for a
new candidate. This document does not claim any new check has passed.

## Required routine qualification

The committed `.ci/pilot.json` is the authoritative selected gate inventory.
Its twelve gates are:

1. `inventory`: delivery/schema/architecture checks and release/admission unit tests.
2. `test`: Rust fmt, clippy, unit tests and generated-source freshness.
3. `ts-test`: TypeScript tests and checked examples.
4. `runtime-images`: source-bound runtime/tool images for the following gates.
5. `dependency-audits`: both Rust dependency graphs against one recorded advisory revision.
6. `helper-pins`: one Conda helper build, exact compiled pin, static/protocol and license admission.
7. `core-package-static-sbom`: exact musl release tarball, checksum, ELF/static, version and binary audit.
8. `candidate-sdk`: exact TypeScript package build/pack.
9. `publication-packages`: source/package inventory and installer freshness; preparing registry-format bytes is not registry publication.
10. `external-source-pins`: exact external example/site revisions.
11. `installed-consumers-docs`: exact core/Conda/SDK bytes, real CLI consumers and external examples.
12. `flows`: normal real-CLI e2e, excluding exhaustive persistence and golden regeneration.

Changed-feature real CLI scenarios are also required before claiming that feature
qualified. In particular, services require an isolated Docker/PostgreSQL smoke
with unique container/volume names, a digest-pinned image and explicit readiness.
Never run that smoke against developer production state; tasks and normal flow
qualification must not implicitly start services or delete data volumes.

Scheduler qualification still requires actual isolated systemd registration,
calendar invocation, update/disable, rollback/GC interaction and rejection of
unsupported timing policies. The owner separately makes disruptive sleep/wake,
logout/login, reboot and host-clock/DST campaigns optional and explicitly
requested; do not disrupt the developer session or change its clock as a routine
gate. Ordinary scheduler observations do not claim those campaigns passed.

The combined D/E journey also requires the scheduled fixture's
`--package-bundle` and `--package-trust` variant on a separate recipient with no
producer paths mounted. It must execute an imported tool and its postconditions,
retain the admitted in-flight run across GC/update, and refuse new consumption
after local trust revocation. See the
[disposable manager fixture](../examples/workspaces/04-scheduled-task/README.md#disposable-real-manager-smoke).


External Docker/registry/Conda/native-wheel scenarios use `native_integration`. Ordinary
pytest/flow/installed-consumer selections exclude that marker; those exclusions
do **not** qualify the changed feature. Prepare its isolated runtime, actual
helper and network fixtures, then run the marked cases explicitly:

```sh
# GRIPSACK_BIN and GRIPSACK_CONDA_HELPER_MIRROR must name the exact candidate
# and measured helper. Prepare the compiled extension against the actual runtime:
python3 e2e/test_package_native_wheel.py prepare \
  --directory /tmp/native-wheel-inputs \
  --runtime-bundle /tmp/conda-runtime-inputs/bundle
export GRIPSACK_TEST_NATIVE_WHEEL_BUNDLE=/tmp/native-wheel-inputs/bundle
uv run --project e2e --locked pytest -o addopts= -m native_integration \
  e2e/test_package_sharing.py e2e/test_package_conda_offline.py \
  e2e/test_package_native_wheel.py e2e/test_package_external_closure.py \
  e2e/test_package_environment.py
```

Set `GRIPSACK_BIN` to the exact candidate binary and supply the fixture-specific
runtime/helper environment, including `GRIPSACK_TEST_PACKAGE_REGISTRY` for the
isolated registry and `GRIPSACK_TEST_CONDA_RUNTIME_BUNDLE` for the prepared real
runtime. The external-closure lane includes real-registry graph consumption;
the environment lane includes mixed local/external Conda use from project,
profile and task contexts with typed artifact environments. Their unmarked
offline admission/authority cases remain in ordinary flows.
`e2e/test_package_environment_projection.py` is an unmarked ordinary-flow lane
for actual shared-DAG, contextual authority, cyclic runtime and non-root export
consumption. It is not replaced by a catalog-only or pointer-identity test.
Keep real OCI existing-image consumption and service readiness in changed-feature
evidence; retired BuildKit production is unrelated.
Record the explicit result separately from the ordinary flows receipt.
The native-wheel lane must exercise the real compiled extension and Python,
ABI, architecture and libc mismatch refusals. Merely preparing its fixture or
running the unmarked offline cases does not qualify that lane.

### Private TLS registry/native handover

`scripts/smoke_package_registry.py` is a standalone stdlib Python journey, not
a passing receipt or a registry provisioner. Before invoking it, provision a
real isolated Distribution 3.1.2 registry with basic authentication and a private
CA, and install that CA in the disposable client OS trust store. Supply the
candidate core, a signed relocatable native bundle exposing `grip` (local native
components), its independently obtained publisher
trust policy, the CA certificate, and a JSON credentials file containing only
`username` and `password`. Do not use production/public registry resources.
The core also receives `SSL_CERT_FILE` pointing to the supplied CA; the independent
Python reader explicitly uses that CA with ordinary hostname/certificate checks.

```sh
python3 scripts/smoke_package_registry.py \
  --grip /private/candidate/grip \
  --registry https://registry.fixture.example:5443 \
  --bundle /private/inputs/native-grip-bundle \
  --trust /private/inputs/publisher-trust.json \
  --credentials /private/inputs/registry-credentials.json \
  --ca /private/inputs/registry-ca.pem \
  --workdir /private/evidence/registry-journey-fresh
```

The work directory must not exist. The script creates separate private sender
and recipient homes, copies the exact supplied policy into each, and uses
mode-0600 scoped credential copies. It imports/inspects/pushes using the real
CLI into a unique repository, independently reads the authenticated OCI manifest
and every referenced blob with `urllib`/`ssl`, and checks digest, size, descriptor
media type, and exact portable-manifest/DSSE bytes against the supplied bundle.
This independent check verifies transport bytes and DSSE payload binding;
signature authority is admitted by ordinary core import/inspect using the
independent policy, not by a second cryptographic implementation.

Only the script-owned sender home/cache and copied bundle are removed before
recipient pulls. Missing/wrong credentials must produce HTTP 401 independently
and authentication refusals through the CLI, followed by authenticated digest-only
pull, inspect, and actual immutable imported `grip --version` execution. No profile
or automation is activated. Caller-owned inputs are never removed, and no
container is started/stopped. Captured output is checked for credential values
before logging; unsafe output is withheld rather than echoed.

Success emits bounded JSON and saves `evidence.json` plus mode-0600
`commands.jsonl` under the private work directory. Failures exit nonzero and
preserve available private evidence; use a new directory for another run.
Retain these receipts with the exact candidate digest. Merely adding this
script or preparing its prerequisites does not qualify the handover.

For focused iteration:

```sh
docker compose run --build --rm test
docker compose run --build --rm ts-test
docker compose run --build --rm e2e
```

For complete qualification, use the existing selected controller installation
and a clean committed checkout (no replacement runner or ambient old receipts):

```sh
export PYTHONPATH=/home/tarek/workspace/ci-infra
export CI_STATE="$HOME/.local/share/buildbot-ci"
python3 -m ci_infra --state "$CI_STATE" qualify "$PWD"
# Use the candidate_id returned above:
python3 -m ci_infra --state "$CI_STATE" watch "$CANDIDATE_ID"
python3 -m ci_infra --state "$CI_STATE" status "$CANDIDATE_ID"
```

The controller checks every gate declared by that frozen profile, its exit code,
source/image/adapter/argv binding, output inventory and hashes. Admission does
not require an obsolete hard-coded 17-gate list. It also does **not** accept a
missing, failed, cancelled or skipped selected gate. The bundle validator
requires exact equality between frozen profile, qualification and gate receipts;
private re-admission checks the original controller archive. Profile edits change
the candidate identity. Source-qualified admission additionally compares the
profile with the committed source. A standalone internally consistent synthetic
bundle is not trusted private admission.

The protected status context is `local-practical-qualification`.
`scripts/release_admission.py` produces that exact context only after selected
private re-admission; it validates the newest matching source/repository/creator
status, never an older convenient success. It does not post GitHub statuses.
Do not substitute the old `local-qualification` context or mark unrun gates green.

## Optional checks: only on explicit request

The following are **not run by default**, not waived failures and not passing
claims. They retain their real entrypoints and original acceptance criteria:

```sh
# Separate GNU/full build (not a second routine prerequisite):
docker compose --profile optional run --build --rm gnu-test
# Model/proof and attributed mutation campaigns:
docker compose --profile optional run --build --rm model
docker compose --profile optional run --build --rm tlaps
docker compose --profile optional run --build --rm verify
docker compose --profile optional run --build --rm mutants
python3 scripts/delivery_checker_calibration.py
# Independent two-build core comparison:
sh scripts/check_reproducible.sh /absolute/path/to/fresh-repro-output
# Independent two-build helper comparison:
python3 tools/conda-helper/package.py --target x86_64-unknown-linux-musl \
  --dist /absolute/path/to/fresh-helper-output --check --reproducible
```

For the complete persistence campaign, first prepare source-bound images in a
clean checkout, set a distinct candidate/checkpoint identity, then invoke the
retained runner:

```sh
python3 scripts/local_ci.py images
export CI_CANDIDATE_ID=explicit-persistence-candidate
export CI_CHECKPOINT_ROOT=/absolute/path/to/private-checkpoints
python3 scripts/local_ci_persistence.py
```

That runner still requires all 96 partitions, both drift states, original
cut inventories and matching source/image/dispatcher receipts. An incomplete
campaign is not success. Its checkpoints do not count as controller gate-success
receipts. An explicitly selected `persistence-96` release gate must supply the
complete partition evidence; an unselected campaign must not invent partitions.
The ordinary profile does not select it.

CI's manual-dispatch optional booleans select deep lanes explicitly. The
aggregate requires selected jobs to succeed and unselected jobs to be skipped;
it never treats a failed selected lane as optional. Fuzz/replay remains separately
owner-authorized (`REL-FUZZ-2026-09-27`); do not run it as part of routine release
qualification. Existing fuzz entrypoints remain available for that assignment.

## Storage hygiene and archive relocation

Standing owner authorization `STORAGE-HYGIENE-2026-10-30` permits bounded
cleanup and verified cold relocation without another capacity-approval round.
It does not authorize discarding unique evidence, reducing admission floors,
rewriting receipts, deleting data volumes or interrupting active work.

1. Measure both filesystems before acting: `df`/`du` for Ubuntu and
   `Get-Volume` for Windows. The current Ubuntu VHDX lives on C:, Docker-managed
   storage on B:, and Seagate's archive on D:. Inspect the WSL registration,
   configured storage identities and `/mnt/d/CI-Archive/.ci-storage-identity.json`;
   do not infer identity from a drive letter or an existing empty directory.
2. Classify the exact paths. Cargo `target/` intermediates are rebuildable;
   executables used for a reported observation, diagnostics, source, `dist/`,
   receipts, checkpoints and backups are not disposable merely because their
   parent looks temporary. Check active compiler processes, open/mapped files
   and live container mounts. Retain active files and their parent directories.
3. Prefer clearing inactive compiler output first. Checkpoint retained
   executables, generated packages and diagnostics to a fresh Seagate directory;
   preserve modes and links in an archive, compare its contents against the
   source, then hash the completed archive. Never use broad workspace globs as
   deletion authority. Root-owned container output can be handled through a
   container bound only to the explicitly selected cache directories.
4. For duplicate CI attempt evidence or predecessor inputs, hold the existing
   controller request lock and confirm no live request/build owns the candidate.
   Validate the candidate/gate/attempt receipts, complete inventories, every
   file's size/hash and stable identities against the retained archive.
   A directory name or a successful old log is not backup evidence. Keep
   unmatched, interrupted and unbacked attempts.
5. If more room is needed, copy inactive cold artifacts to a fresh directory
   under the verified Seagate archive. Record source-to-archive paths and a
   complete inventory, preserve filesystem metadata in an archive where needed,
   read back and independently verify every retained byte before removing the
   original. Keep published release/controller admission records available at
   their required paths; do not replace live state with NTFS symlinks or assume
   an archive copy is immediately usable as restored state. Same-host external
   storage is not offline/disaster-recovery proof.
6. Save a cleanup receipt outside the deleted tree: selected paths, protected
   paths, source/retained hashes, verification results, before/after available
   bytes and the exact removals. Run the real `ci_infra storage-check` afterward.
   Never substitute lowering the 64-GiB state floor or waiving a gate.

For development compilation, use a toolchain/profile-specific Docker named
volume, not a host checkout's `target/` bind:

```sh
docker compose run --build --rm \
  -v gripsack-dev-rust198-musl:/app/target \
  -e CARGO_PROFILE_DEV_OPT_LEVEL=1 \
  -e CARGO_PROFILE_DEV_DEBUG_ASSERTIONS=true \
  -e CARGO_PROFILE_DEV_OVERFLOW_CHECKS=true \
  build cargo build --locked -p gripsack
```

Named volumes remain caches, not release evidence, and consume B: capacity.
The ordinary e2e image already uses an optimized development build with debug
assertions and overflow checks; do not mistake opt-level-zero hashing cost for
a reason to weaken runtime admission or enlarge command deadlines.

Deleting Ubuntu files does not necessarily shrink its Windows VHDX. Online
`fstrim` and an eventual offline
[DiskPart compaction](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/compact-vdisk)
are separate maintenance steps. Inspect the actual Ubuntu VHDX path first.
Compaction requires the disk detached or attached read-only; quit Docker/WSL
clients and stop the distro only in an explicit downtime window. Do not terminate
the running assistant, editor or qualification jobs to shrink the file. Report
guest free space and Windows free space separately; an unrun compaction is not
a claimed recovery.

### Docker Desktop privilege boundary

Start and stop Docker Desktop as its normal owning user. Run only the necessary
disk-maintenance command in Administrator PowerShell, detach the disk, then
leave that elevated shell before starting Docker. Do not bundle an elevated
`docker desktop start` into the compaction instructions.

If WSL reports `MountDisk/HCS/E_ACCESSDENIED`, inspect the exact VHDX named in
the error, its attributes, attachment state, owner and effective permissions.
Docker's small `main/ext4.vhdx` bootstrap and large `disk/docker_data.vhdx`
are different files. The 2026-10-30 recovery found an Administrator-owned
bootstrap whose normal account could read/write bytes but could not manage
its ACL; the data disk did not have that restriction. Saved ACLs and a narrowly
scoped, owner-approved permission repair restored normal-user startup without
resetting Docker or changing the bootstrap bytes.

Back up security descriptors before repair. Grant only the intended owning
account the required access within the actual Docker WSL directory; retain
SYSTEM and existing VM/capability entries. Protected Administrator-owned child
ACLs can require elevation even when the parent is user-owned. Never grant
Everyone full control, recursively reset the whole drive, unregister a distro,
or replace the data disk as a first response. Verify the original daemon ID,
retained volume bytes and an actual container execution afterward.

Ubuntu and Docker also have separate guest filesystems. Trimming Ubuntu's `/`
does not trim Docker's data disk. Identify the actual Docker data mount before
any discard operation; a reported guest discard does not establish Windows
space recovery. Re-measure host capacity and retain the configured admission
floor.

### Planned hardware, unchanged storage roles

The owner-selected next fast tier is a Lexar NM790 4-TB NVMe in an OWC Express
1M2 enclosure on the workstation's USB4 40-Gb/s port. It replaces the existing
fast build tier; Seagate remains the cold archive. Until it is installed and
verified, the existing drive and storage identity remain authoritative.

Use the supported Docker storage-location workflow in a quiescent window.
Retain and verify the old copy before retirement, check normal-user ACLs and
the engine/image/volume inventories, and deliberately update the recorded
physical/volume identities for the replacement device. Keeping drive letter B:
does not make a new disk match the old storage binding. Keep bounded cache
retention even with the extra capacity.

The HP mini is a future independent verification/signing boundary, not a
provisioned signer today. It should receive immutable candidates and verify
their bytes and approval evidence independently; signing keys must not enter
the shared build cache or untrusted build execution. The current unsigned
GitHub-only release authority remains unchanged until the real host and its
explicit policy are provisioned and exercised.



## Conda measurements and unchanged-source reuse

Conda remains supported. Core/SDK/helper versions are coordinated at 0.51.0 for
this release; the integration owner owns version/pin changes. One build is the
default; `reproducible_rounds: 1` is not a reproducibility claim. A requested
second independent build records `2` only after byte equality.

A new measurement records `source_sha256` over the helper manifests, lock,
packager, Rust sources, vendored inputs and transitive local dependency sources,
manifests and build scripts. Used inherited workspace package/dependency/lint
settings are bound; unrelated core/SDK code is not. The Conda pin table is
checked independently, not hashed into its own measurement. Unchanged
measurements can be imported without rebuilding:

```sh
python3 scripts/local_ci.py helper --reuse-helper-dist /absolute/path/to/measured-helper
# Or directly with the pinned toolchain installed:
python3 tools/conda-helper/package.py --target x86_64-unknown-linux-musl \
  --dist /absolute/path/to/fresh-helper-output --reuse-dist /absolute/path/to/measured-helper --check
```

Reuse requires exact compiled pin/version, source/lock/compiler/image metadata,
checksum sidecar, original license bytes, static binary admission and a fresh
protocol smoke. Changed/missing source metadata fails; it never silently falls
back to a rebuild or claims a new measurement. For controller qualification,
select the explicit reuse argv/path in the reviewed profile before freezing it;
its resulting bytes still pass the normal installed consumer gate. Historical
manifests without the source binding require one new measurement.

The 0.51 local profile explicitly re-admits the existing helper measurement at
`/home/tarek/.local/share/buildbot-ci/worker/retained-inputs/conda-0.51.0-60d66f94`.
Its six files were copied and hash-verified from the passed `helper-pins` attempt
of candidate `1a2df07b09fa6b9d441e88d9dc52fc3b96d048dea2950cf3c7dd035e8bef6b10`;
that candidate's later failure does not become a passing overall qualification.
The helper source digest remains
`971f8f393ca26612059d4fd41454687e6bb5ba5107369ff51ec6d97dc43d6d76`.
This path is a worker-visible input of the selected local installation, not a
new measurement or a portable host default. Another installation must stage
and verify that exact tuple or explicitly select an ordinary helper build
before freezing its source/profile. No fallback or gate-success reuse is implied.


## Manual unsigned GitHub-only publication

No Homebrew channel, BuildKit bridge/B0 release asset, hosted duplicate build,
registry migration or new signing infrastructure is a prerequisite. Generic OCI
transport and Docker service runtime are separate from the retired BuildKit
backend. Retained generations/receipts/journals/rollback/GC safety is unchanged.

After actual complete selected qualification, assemble and independently admit
the exact candidate bundle using the existing selected CI runtime:

```sh
python3 scripts/release_prepare.py assemble --source "$PWD" \
  --candidate "$CI_STATE/inbox/$CANDIDATE_ID/candidate.json" \
  --evidence "$CI_STATE/archive/$CANDIDATE_ID/evidence" \
  --destination /absolute/path/to/fresh-release-nomination
export NOMINATION=/absolute/path/to/fresh-release-nomination
python3 scripts/release_manifest.py validate --source "$PWD" \
  --root "$NOMINATION/bundle" \
  --manifest "$NOMINATION/bundle/gripsack-release-0.51.0.json"
```

Review the nomination, exact asset hashes and source tag before the irreversible
manual publication. With `NOMINATION` pointing at that admitted directory and
`RELEASE_NOTES` at the reviewed product notes:

```sh
export VERSION=0.51.0
export TAG="core-v$VERSION"
gh release create "$TAG" --repo gripsack-dev/gripsack --verify-tag \
  --title "Gripsack $VERSION" --notes-file "$RELEASE_NOTES" --draft \
  "$NOMINATION"/bundle/artifacts/* \
  "$NOMINATION"/bundle/gripsack-release-"$VERSION".json
# Inspect/download/check the actual draft assets before publishing:
gh release view "$TAG" --repo gripsack-dev/gripsack
gh release edit "$TAG" --repo gripsack-dev/gripsack --draft=false --latest
```

The tag must already point at the qualified source; `--verify-tag` refuses a
missing remote tag. Do not overwrite public assets or retag an existing release.
Publish the SDK tarball as a GitHub asset, not an unrequested npm/crates.io upload.
Record the qualification receipt and explicit unrun optional checks in the
release evidence. A draft or successful upload alone is not consumer verification.

`release-core.yml` is dispatch-only guidance, not a publisher. The retained
signed installer/update admission path remains fail-closed and requires its
actual signed evidence; this unsigned manual release must not be advertised as
satisfying it. Manual tarball/checksum installation is the selected distribution
path. No synthetic signature or passing hosted/proof status is manufactured.

## Integration checks to exercise after edits

```sh
python3 scripts/check_ci_gate.py --self-check
PYTHONPATH=/home/tarek/workspace/ci-infra python3 -m unittest discover -s scripts -p 'test_release_*.py'
python3 scripts/release_installer.py --check
docker compose config
```

Then run the complete selected profile and the changed-feature real CLI/service
smokes. These commands are recommendations, not a report that they ran. Preserve
raw failures and fix their actual causes; do not suppress selected checks to
produce a successful status.
