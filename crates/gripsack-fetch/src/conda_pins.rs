//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.57.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// ff4c3ed99e072b80b7b3357054e9023330da1205c306224c0e741614daf73fa0.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "4ffbc7c54b4ccf685b299765141a1f710ad7f13ee557f6247e3fe2a81eb6bb18",
)];
