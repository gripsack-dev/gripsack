# Two-clean-build comparison (plan/0042 F)

Run directory: /tmp/gripsack-review-0450/0.45.0/run-20261007T110152Z-212282
Date: 2026-10-07T11:01:52Z
Target: release (musl static, cargo auditable build --release --locked -p gripsack)

## Inputs held fixed

- Source: git ce1f20754d76c95e545fea5c5ee6025ed4e05a23 (dirty worktree: no)
- Cargo.lock sha256: d2b462442e01fb40c3c625fccd58fa9fb2b4c69dddc5d106b9e7bf7be58dc915
- Builder base pin (parsed from Dockerfile): rust:alpine@sha256:a10e64dd139b7387337c7fbe8aca31b959b57b2fd4c8ae20a02cf1d6ea424dce
- Built image: sha256:e6445e884c3404402973c6267f6a0b286c09f9b4f8e3666cfa61cd8a67ff6dbc (created: 2026-10-07T10:59:20.894334274Z)
  - both runs pinned to exactly this image ID via compose override
    (image=<id>, pull_policy never) — same-image is proven, not assumed
- Container workdir: /app (same path both builds)
- Target dir: fresh per build (fresh container, CARGO_TARGET_DIR=/tmp/repro-target)
- Toolchain: identical between builds (yes) — see toolchain.txt
- Profile: workspace [profile.release], unmodified between builds

## Result

- build a sha256: c33cf2ee114ebc7c584599abc59e08c85a5090f7eb6c035b0f18030091583f31
- build b sha256: c33cf2ee114ebc7c584599abc59e08c85a5090f7eb6c035b0f18030091583f31
- binaries byte-identical: yes

## Not claimed

Pinning a compiler does not imply cross-time reproducibility (a deliberate
toolchain bump changes inputs by design), cross-platform identity (darwin
targets are separate builds), or an external audit.
