//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.45.0";
// Linux-first 0.45.0: both Linux targets are measured. Mac slots remain absent
// and fail closed until native qualification permits publishing their artifacts.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[
    (
        AssetTarget::LinuxX86_64Musl,
        "152de682bbe2e85de3d27163a63f6a7a5583d6f621946173ea32f50924da4516",
    ),
    (
        AssetTarget::LinuxAarch64Musl,
        "57f6db60e08441eaf0ac4813816ff873257ded21770968a64cce7cbc78585bd5",
    ),
];
