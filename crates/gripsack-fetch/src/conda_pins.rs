//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.46.0";
// REL-X64-FIRST-0460-2026-10-08: only the native x86_64 artifact is qualified.
// Missing architecture slots fail closed; historical release pins are not reused.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[(
    AssetTarget::LinuxX86_64Musl,
    "d88c77dd600b7ef0db4c27d17099e2f7660ea0ddf0b42615211a1a6a2f6bf15a",
)];
