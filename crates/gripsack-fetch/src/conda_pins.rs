//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.56.1";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// eb5d2896daef25911dd3a905283ab89e16792c5046c68e526d390cb2a9f294ac.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "d2436b94ea0612d304d48c90f92ea29b0355a18dab35cdd3d40d29c0f558ad98",
)];
