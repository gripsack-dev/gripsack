//! Generated release identity: replace this table only with hashes measured
//! from tools/conda-helper/dist.sh output before building the core release.
use crate::host::AssetTarget;
pub const CONDA_VERSION: &str = "0.44.0";
// Required qualification: LinuxX86_64Musl, LinuxAarch64Musl, MacosAarch64.
// MacosX86_64 is optional and fails closed until measured.
pub const CONDA_SHA256: &[(AssetTarget, &str)] = &[
    (
        AssetTarget::LinuxX86_64Musl,
        "dbb6f931709cc208c950e1041db884d6ac5b1246c45fb8775cd5bea9c1cf4a7a",
    ),
    (
        AssetTarget::LinuxAarch64Musl,
        "e49559705e38de0f19f2030f3d88142db6715ff0df71f9a514f27f54459b8b51",
    ),
    (
        AssetTarget::MacosAarch64,
        "87b76df9c477657d18ef83eac8e533603af09ddc1b0ddcb332c373aa7df0eeb9",
    ),
];
