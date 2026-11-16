//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.55.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// 559f6439c7e0e2ea998d9f8bef44b1619e41c5284c43b59bf37bdd726fa39a16.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "444243bcf2055d40ecf2c10d487264294e244d77a6a9962c4e98a5b22ada5bc2",
)];
