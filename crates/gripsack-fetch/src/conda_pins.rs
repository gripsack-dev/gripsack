//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.53.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// 8ed30903e667bfce2283ccf776fe01153970a740b787016f85b5c969a4f49ad3.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "12867c5af47543b358e10df6031769b4cb90a75234482e0e6977a6d3ca0612f9",
)];
