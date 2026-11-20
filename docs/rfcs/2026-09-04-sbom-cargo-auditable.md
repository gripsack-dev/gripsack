---
rfc: "2026-09-04-sbom-cargo-auditable"
title: "Embed the release binary's software bill of materials"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-04"
updated: "2026-11-20"
issue: "https://github.com/gripsack-dev/gripsack/issues/272"
milestone: "core-v0.21.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0022"
---

# RFC: Embed the release binary's software bill of materials

> **Backfill note (2026-11-20).** The old 'not started' header is obsolete. A detached sidecar as the sole inventory was rejected; an additional CycloneDX file was conditional on demand, not a promised release asset. The dependency inventory is distinct from a publisher signature or builder attestation.
> [Original design 0022](https://github.com/gripsack-dev/gripsack/blob/7924bc2b844d109dbb520040f5954ad237007ff3/plan/0022-sbom-cargo-auditable.md) · [tracking issue #272](https://github.com/gripsack-dev/gripsack/issues/272). The text below is the historical record.

Status: **planned, not started** — handover document. Companion to
the attestations shipped in 0.20.0 (plan/0020 §5.8): attestation
identifies the BUILDER; the SBOM inventories the CONTENTS.

## Decision already made

**cargo-auditable** over sidecar files: it embeds the compressed
dependency list into the binary itself (`--cfg auditable`), so the
SBOM travels with grip after copying, and `cargo audit` can audit a
distributed binary directly. Sidecar SPDX/CycloneDX files get
detached from the binary the moment a user untars elsewhere.

## Implementation

1. **Dockerfile (musl builds) + release workflow (macOS builds)**:
   replace `cargo build` with `cargo auditable build` for release
   targets only — debug/e2e builds stay plain (build-time cost is
   negligible but keep dev builds identical to today).
   - Install: `cargo install cargo-auditable` (or the prebuilt via
     taiki-e/install-action — see ci.yml's audit job for the
     pattern; avoid from-source installs breaking on new toolchains,
     a lesson already paid for).
2. **Verification step in release-core.yml**, after "Verify binary":
   `cargo auditable audit` against the built binary — proves the
   embed landed and the tree is advisory-clean at ship time.
3. **Dockerfile test gate**: unchanged (debug binary, no embed
   needed).
4. **docs**: one line in the website's install/verification page —
   `cargo auditable audit path/to/grip` — plus the changelog entry.
5. **Optional follow-up** (separate PR): a CycloneDX sidecar
   attached to the GitHub release for scanners that want a file.
   `cargo cyclonedx` generates it; attach alongside the tarballs.
   Only if a user asks — the in-binary form is the primary.

Pre-1.0, no users: breaking changes allowed when they serve
readability/simplicity (owner policy); quality bar is high and
modular.

## Acceptance

- `strings target/.../release/grip | grep -c auditable` > 0, or
  better: `cargo auditable audit` succeeds on the release binary in
  CI.
- Release workflow green on a real tag; the verification step fails
  the release if the embed is missing.
- Website mention live.

## Pitfalls

- cargo-auditable must wrap the SAME build invocation that produces
  the shipped binary — wrapping a different profile ships an
  un-auditable binary silently. The in-workflow verification step
  (2) exists to catch exactly that.
- The macOS cross-build (x86_64 on arm) path in release-core.yml
  builds with `cargo build --release --locked --target ...` — the
  wrapper goes on that line, not a native build.
- Workspace: all 9 crates end up in the binary's dependency tree —
  the embed covers the whole graph automatically (it reads
  Cargo.lock); nothing per-crate to do.
