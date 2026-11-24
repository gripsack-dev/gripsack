---
name: gripsack-release
description: Qualify exact-source Gripsack artifacts locally and publish an immutable core-vX.Y.Z GitHub release with scripts/release.py
---

# Releasing

A release is the local Buildbot qualification green on the exact source,
then one script that uploads those same bytes ([RFC 0056](../../../docs/rfcs/2026-11-08-verified-foundation.md) §2 rule 6). Core,
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
   Open a release PR. Its head must be the commit you qualify.
2. Qualify a clean checkout of the release commit on a local branch:
   ```sh
   cd /home/tarek/workspace/ci-infra
   python3 -m ci_infra storage-check && python3 -m ci_infra up
   python3 -m ci_infra qualify /home/tarek/workspace/ci-checkouts/gripsack
   python3 -m ci_infra watch <candidate_id>
   ```
   [`.ci/pilot.json`](../../../.ci/pilot.json) defines the required gates and
   their order. `python3 scripts/local_ci.py test` runs both the Rust container
   checks and the Python script tests. `proofs` checks translation drift, Lean,
   axioms and mutants; `smoke` runs `scripts/release.py verify` on the packaged
   artifacts. A failed required gate blocks publication.
3. On the host, post the candidate's status, merge the PR normally (preserving
   the qualified commit on `origin/main`), then publish with your `gh` login:
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
- Existing tags and assets are immutable. `model` (TLC), `tlaps`, manual fuzz
  and the two-build reproducibility check are optional, not required gates.
  Never start fuzz or replay its corpus without an owner request.
- The website lives in `gripsack-dev/gripsack-dev.github.io`; build it locally
  from the release commit and publish with its `website/publish.py`.
