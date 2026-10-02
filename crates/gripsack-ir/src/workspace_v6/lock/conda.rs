//! Frozen Conda closure records persisted by the portable lock (A3).
//! These are complete normalized Rattler package records — every
//! install-critical field (depends/constrains/noarch/track_features and
//! the measured artifact identity) survives, so a frozen consumer can
//! validate host capabilities against the FULL constraint set and
//! reconstruct the environment offline from retained original archives.
//! No credentials: authenticated channels are a strict-import error at
//! solve time; the records carry anonymous HTTPS artifact URLs only.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One frozen environment for one conda subdir: the complete transitive
/// closure in canonical order plus the solver policy record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedCondaEnvironment {
    /// Conda subdir, e.g. "linux-64".
    pub platform: String,
    /// Canonical channel URLs in priority order.
    pub channels: Vec<String>,
    /// Solver channel-priority policy record.
    pub channel_priority: ChannelPriority,
    /// Explicit manifest target requirements, distinct from historical solve
    /// facts. Required even when empty so missing policy is not invented.
    pub system_requirements: LockedCondaSystemRequirements,
    /// Virtual packages the solve was grounded in (e.g. the measured
    /// `__glibc` the solver assumed). Frozen admission re-validates the
    /// measured host against every depends/constrains MatchSpec naming a
    /// virtual package — these values are NOT universally floors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub virtual_packages: Vec<LockedVirtualPackage>,
    /// Complete transitive closure, canonical order (name, then the
    /// full record identity); enforced by the lock reader.
    pub packages: Vec<LockedCondaPackage>,
    pub materializer: MaterializerPolicy,
}

/// A complete normalized Rattler package record. `sha256` is mandatory:
/// MD5-only records are a strict-import error at solve/import time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedCondaPackage {
    pub name: String,
    pub version: String,
    pub build: String,
    pub build_number: u64,
    pub subdir: String,
    /// Canonical channel URL.
    pub channel: String,
    /// Immutable artifact URL (.conda/.tar.bz2).
    pub url: String,
    /// 64 lowercase hex; enforced by the lock reader.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexed_timestamp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestations_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_bz2_md5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_bz2_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "LockedNoArch::is_none")]
    pub noarch: LockedNoArch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_family: Option<String>,
    /// Full conda MatchSpec list, preserved verbatim from the record —
    /// host-capability validation evaluates every entry that names a
    /// virtual package against measured host facts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constrains: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra_depends: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python_site_packages_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_exports: Option<LockedRunExports>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purls: Option<BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub track_features: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<String>,
}

/// Frozen build-to-runtime dependency metadata, including constraints and
/// noarch exports. Retaining only ordinary `depends` loses these semantics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedRunExports {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weak: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strong: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub noarch: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weak_constrains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strong_constrains: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockedNoArch {
    #[default]
    None,
    Generic,
    Python,
}

impl LockedNoArch {
    pub fn is_none(&self) -> bool {
        matches!(self, LockedNoArch::None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedVirtualPackage {
    pub name: String,
    pub version: String,
    pub build: String,
}

/// Declared target policy survives even when no selected package needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedCondaSystemRequirements {
    /// Canonical name order; each named capability is required, not optional.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub virtual_packages: Vec<LockedVirtualPackageRequirement>,
    /// Minimum microarchitecture: evaluated by ancestry, not build equality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archspec: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedVirtualPackageRequirement {
    pub name: String,
    pub minimum_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelPriority {
    Strict,
    Flexible,
}

/// How a frozen closure is materialized. v1 admits exactly one policy
/// each; new policies are a lock-version concern, never silent defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializerPolicy {
    pub bytecode: BytecodePolicy,
    pub receipt: ReceiptPolicy,
}

/// v1: no .pyc in the prefix; wrappers export PYTHONDONTWRITEBYTECODE=1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BytecodePolicy {
    Suppress,
}

/// v1: origin URL + sha256 recorded per package; incidental timestamps
/// dropped from the normalized conda-meta receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptPolicy {
    NormalizedCondaMeta,
}
