---
name: gripsack-release
description: Qualify exact-source Gripsack artifacts locally and publish immutable releases with honest verification scope
---

# Releasing

Current policy is the owner's practical local qualification profile in
`.ci/pilot.json` and `plan/0054`. Core, SDK, Conda helper and bridge assets are
coordinated. A tag or version string is not qualification, and pushing a tag
does not request an Actions product build.

## Core (binary)

1. Freeze the exact source commit, profile, external source pins, tools and
   candidate artifacts. Run the required local gates against those inputs.
   Preserve failed attempts; changed source needs its own evidence, never a
   relabeled receipt.
2. The practical release profile has 17 gates, including all 96 Linux persistence
   partitions. TLC/TLAPS/Verus are separate and must not be claimed qualified.
   No fuzz engine or saved-corpus replay is authorized by this workflow.
3. Prepare a draft release and upload only the admitted candidate's exact bytes.
   Read every uploaded asset back and compare its digest before publication.
   Publish an immutable `core-vX.Y.Z` tag and exact source/qualification scope.
4. Exercise an actual installed archive, its matching SDK and default helper
   provisioning from the public assets. Build/run a frozen consumer without an
   operator helper override. Checksums are transport integrity, not signatures.

0.48 is a public, locally qualified **unsigned** release. The owner explicitly
removed Actions signing and registry publication as prerequisites. Use its
manual installation instructions; do not weaken the signed installer/updater
or synthesize a signed completion record. 0.47 remains an explicitly unverified
prerelease; its exceptional authorization is not a standing waiver for later
releases. Released tags and assets never change during documentation cleanup.

## TypeScript (frontend package)

Ship the SDK archive matching the core version. A repository-installed SDK is a
deliberate frontend pin; an editor-only copy may instead be explicitly excluded
from capture, selecting the embedded frontend. An npm declaration alone neither
installs the SDK nor upgrades the core.

GitHub assets, npm publication and crates.io publication are separate events.
Do not claim either registry was published unless it actually was. Retained
signed-publication tooling and closed historical evidence formats are not the
unsigned release path; the website is independent of those formats.

## Checklist

- [ ] Coordinated versions and actual helper pins; documented IR/lock migration.
- [ ] Required exact-source local gates, including the complete persistence union.
- [ ] Actual installed consumers, failed-path coverage and public asset readback.
- [ ] Honest release notes, source identity, checksums and verification exclusions.
- [ ] No overwritten public tag, asset or historical qualification receipt.
- [ ] Current README, SDK guidance and executable examples.
- [ ] Website rendered locally from the selected core Git commit in the separate
      site repository; `website/publish.py` uploads a static commit and dispatches
      only Pages deployment. Check the live desktop/mobile surface afterward.

Main requires `local-practical-qualification`, not a fabricated hosted `test`
result. Record the actual source and check scope; development status does not
by itself authorize a release. Continue independent work while an external
publication step runs, without mutating the pending candidate.
