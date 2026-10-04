//! GENERATED — measured per-platform pins for the BuildKit bridge helper.
//!
//! `tools/buildkit-bridge/dist.sh --update-pins` rewrites this file from
//! a deterministic build inside the pinned golang image: every hash is
//! the measured sha256 of the exact artifact that ships on the
//! `core-v<BRIDGE_VERSION>` GitHub release — never a fetched sidecar
//! checksum. The release gate rebuilds and compares every platform.

use crate::host::AssetTarget;

/// The core release whose `core-v<version>` tag carries the matching
/// helper artifacts (parent decision: helper rides the core tag, no
/// separate namespace). Flipping this without re-measuring the hashes
/// below fails the release workflow's `--check`.
pub(crate) const BRIDGE_VERSION: &str = "0.44.1";

/// (platform, sha256 of `grip-buildkit-bridge-<version>-<triple>`).
/// Generated only from the measured, gated helper source.
pub(crate) const BRIDGE_SHA256: &[(AssetTarget, &str)] = &[
    (
        AssetTarget::LinuxX86_64Musl,
        "cda198af66f16c889ab69d54563c3ae0bc251126d066aeb239bb131349722e54",
    ),
    (
        AssetTarget::LinuxAarch64Musl,
        "9635d8869b1223beacead2477e6d0674bb06692ed9cc812e21d220f4eb9d7ff7",
    ),
    (
        AssetTarget::MacosX86_64,
        "ce196138c3ffcfa70e280d78a543762c8a8754d5d6612ef35a4b62d8d6ec3310",
    ),
    (
        AssetTarget::MacosAarch64,
        "c6d08934df6da46f9681796a8487f78f074440c35db442a3e9ac49af7dff41bb",
    ),
];
