//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.54.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// 71f30b08f7ee8dec2b2b4c904309bad1fc194e3df49667814c0d0a903d5fa34e.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "672d652e3227f9efd6e62510e17f461786fac9aeea4a80ce553989d5d570b377",
)];
