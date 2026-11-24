//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.60.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in two independent pinned native release builds; source-input SHA-256
// 45ea6e18e394e5cd1c843ecaa2c2d6a58bd94fb2a8f3ee4b6a756570a3d2c0ad.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "4878a88510bae8f1a56ce670cfd2871b0bb103935bd0e14078f0443e2875136e",
)];
