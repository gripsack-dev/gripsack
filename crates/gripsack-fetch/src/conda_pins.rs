//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.47.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Unpublished candidate measured in two independent native release builds.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "c52dd5389d3cb1a221b14d5f765e5c943fa170d97529c7a0cb986cc4fd7d9f2f",
)];
