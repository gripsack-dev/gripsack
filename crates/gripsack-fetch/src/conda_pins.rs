//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.58.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// cd1935a449438f97082232fc53ff12dcf684f9ccd663f160b703ca7aa20f4857.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "7d7494f5cbb8e6ebc343adfc251f5dd83e0724a23dc9187c0e6f6eda032ea2d7",
)];
