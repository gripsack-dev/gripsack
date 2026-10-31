//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.51.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// 971f8f393ca26612059d4fd41454687e6bb5ba5107369ff51ec6d97dc43d6d76.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "60d66f94ef93fa36d3de6de7bf9b264b3e4d0ef16fc547791218f8849330ea38",
)];
