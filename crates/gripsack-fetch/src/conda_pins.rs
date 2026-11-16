//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.56.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// 97778dbb8eaa9ef3e71840b42e7965c470960e3ea28d4ab327b9ab6989c7ef36.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "8507498f408dc03e3afc738ca9cdb040266cb54fa6238a92d58b027d4a201de2",
)];
