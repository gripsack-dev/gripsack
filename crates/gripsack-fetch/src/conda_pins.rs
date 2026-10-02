//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.44.0";
// Required qualification: LinuxX86_64Musl, LinuxAarch64Musl, MacosAarch64.
// MacosX86_64 is optional and fails closed until measured.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[];
