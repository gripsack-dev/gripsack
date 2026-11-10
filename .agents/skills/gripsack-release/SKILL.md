---
name: gripsack-release
description: Qualify exact-source Gripsack artifacts locally and publish an immutable core-vX.Y.Z GitHub release with scripts/release.py
---

# Releasing

A release is the local Buildbot qualification green on the exact source,
then one script that uploads those same bytes (plan/0056 §2 rule 6). Core,
SDK (`@gripsack/core` tarball) and Conda helper share one version. Publication
is manual, unsigned and GitHub-asset-only; no npm/crates.io publication.

## Sequence

1. Bump the coordinated version: `Cargo.toml` workspace, `Cargo.lock`,
   `tools/conda-helper/Cargo.toml` (+ lock), `typescript/package.json` (+ lock),
   the helper pin (`crates/gripsack-fetch/src/conda_pins.rs`, measured with
   `tools/conda-helper/dist.sh`), and the version in `.ci/pilot.json`
   (its `version`, `artifacts` and the `package`/`smoke` file lists —
   `scripts/test_release.py` fails if they drift), and regenerate the embedded
   frontend (`python3 scripts/gen_frontend_embed.py`; it embeds
   `typescript/package.json`, so the `test` gate fails on a stale copy).
   Merge through a PR.
2. Qualify a clean checkout of the release commit on a local branch:
   ```sh
   cd /home/tarek/workspace/ci-infra
   python3 -m ci_infra storage-check && python3 -m ci_infra up
   python3 -m ci_infra qualify /home/tarek/workspace/ci-checkouts/gripsack
   python3 -m ci_infra watch <candidate_id>
   ```
   Gates: test, ts-test, e2e, audit, package (`release.py prepare`), smoke
   (`release.py verify`). A failed gate means fix and re-qualify; never relabel.
3. On the host (uses your `gh` login):
   ```sh
   python3 scripts/release.py status <candidate_id>     # local-practical-qualification=success
   python3 scripts/release.py publish <candidate_id> --notes NOTES.md          # dry run
   python3 scripts/release.py publish <candidate_id> --notes NOTES.md --yes    # publish
   ```
   `publish` refuses an existing tag, a commit not on origin/main or a missing
   status; copies the archived qualified bytes (digest-checked), re-verifies
   them, creates `core-vX.Y.Z` at the qualified commit and reads every asset back.
4. Check a public install by hand: download the tarball + `.sha256`, install
   the SDK from the release URL, run a frozen consumer.

Without Buildbot, `python3 scripts/release.py prepare && python3 scripts/release.py verify`
reproduces the package and smoke gates in any checkout (a dry run; nothing is published).

## Notes

- Release notes lead with product changes. Installer and self-update verify the published SHA-256 sidecar; nothing is signed.
- Existing tags and assets are immutable. Optional campaigns (model, tlaps,
  verify, fuzz, two-build repro) are not part of a release unless run and
  reported; never dispatch fuzz without an owner request.
- The website lives in `gripsack-dev/gripsack-dev.github.io`; build it locally
  from the release commit and publish with its `website/publish.py`.
