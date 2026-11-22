//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.59.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// 495573416e9d27de08b198deb739c8384e477dcfabbbd56f86703a01d1426bf6.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "61953c17ea5adf899b47dea97e1ffce9d91914a7f7ca1353efb3d78da3569ef2",
)];
