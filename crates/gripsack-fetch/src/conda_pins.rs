//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.59.0";
// PLATFORM-LINUX-X64-2026-10-09: only Linux/WSL x86_64 is supported.
// Measured in two independent pinned native release builds; source-input SHA-256
// b3708a4e565d34f528a37513a71428fc25af8f0e91a1c5b1f65fe50a50c10692.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "bedc978281b61283118fbaf31b8eec931a2f212e08698c60ea1098ce1681e20b",
)];
