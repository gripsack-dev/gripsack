//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.44.1";
// Linux-first 0.44.1: both Linux targets are measured. Mac slots remain absent
// and fail closed until native qualification permits publishing their artifacts.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[
    (
        AssetTarget::LinuxX86_64Musl,
        "41227b553bfe0229f5e8f6f4ac784f236c0176d55826952270fa323072af2124",
    ),
    (
        AssetTarget::LinuxAarch64Musl,
        "d5a3a55330aa966d138730f500fa66925a56d99418b9936690da66b1279569ba",
    ),
];
