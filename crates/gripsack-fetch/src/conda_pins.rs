//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.58.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in one pinned native release build; source-input SHA-256
// df23c3a8442be18033f78b1d4c5fb54f77cb6bc2f08699dc5fb96a5c09568174.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "471200b44eeb6d3981cf164cef854cb9aa2ad06b547262454cec4455c55adc55",
)];
