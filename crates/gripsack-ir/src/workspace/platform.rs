//! Per-output target/ABI/minimum-OS requirements (0052 A1-02). These
//! describe declared compatibility, never ambient host selection.

use gripsack_policy::target::{BinaryAbi, OsRelease, TargetArch, TargetOs, TargetRequirement};
use serde::{Deserialize, Serialize};

/// Per-output platform requirements — never inherited from the
/// core-injected `host` facts (0052 §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePlatform {
    pub os: PlatformOs,
    pub arch: PlatformArch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<PlatformAbi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_os: Option<OsVersion>,
}

impl WorkspacePlatform {
    /// Adapt strict v5 declarations to the production policy's typed
    /// requirement. An omitted patch is a zero floor component, not
    /// a wildcard; the conversion never reads the checking host.
    pub(crate) fn policy_requirement(&self) -> TargetRequirement {
        TargetRequirement {
            os: match self.os {
                PlatformOs::Linux => TargetOs::Linux,
                PlatformOs::Macos => TargetOs::Macos,
            },
            arch: match self.arch {
                PlatformArch::X86_64 => TargetArch::X86_64,
                PlatformArch::Aarch64 => TargetArch::Aarch64,
            },
            abi: self.abi.map(|abi| match abi {
                PlatformAbi::Gnu => BinaryAbi::Gnu,
                PlatformAbi::Musl => BinaryAbi::Musl,
                PlatformAbi::Darwin => BinaryAbi::Darwin,
            }),
            minimum_os: self.minimum_os.map(|version| OsRelease {
                major: version.major,
                minor: version.minor,
                patch: version.patch.unwrap_or(0),
            }),
        }
    }

    pub fn valid_abi(&self) -> bool {
        matches!(
            (self.os, self.abi),
            (_, None)
                | (
                    PlatformOs::Linux,
                    Some(PlatformAbi::Gnu | PlatformAbi::Musl)
                )
                | (PlatformOs::Macos, Some(PlatformAbi::Darwin))
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformOs {
    Linux,
    Macos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformArch {
    X86_64,
    Aarch64,
}

/// Binary ABI, not a libc guess inherited from the checking host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformAbi {
    Gnu,
    Musl,
    Darwin,
}

/// A target's minimum OS (Linux kernel or macOS version according to
/// `WorkspacePlatform.os`). An omitted patch is equivalent to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OsVersion {
    pub major: u16,
    pub minor: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_policy::target::supports_target;

    #[test]
    fn target_floor_and_abi_admit_only_compatible_consumers() {
        let provider = WorkspacePlatform {
            os: PlatformOs::Linux,
            arch: PlatformArch::X86_64,
            abi: Some(PlatformAbi::Gnu),
            minimum_os: Some(OsVersion {
                major: 5,
                minor: 15,
                patch: None,
            }),
        };
        let supports = |provider: &WorkspacePlatform, consumer: &WorkspacePlatform| {
            supports_target(
                &provider.policy_requirement(),
                &consumer.policy_requirement(),
            )
        };
        let mut consumer = provider.clone();
        consumer.minimum_os = Some(OsVersion {
            major: 6,
            minor: 1,
            patch: None,
        });
        assert!(supports(&provider, &consumer));
        consumer.minimum_os = Some(OsVersion {
            major: 4,
            minor: 19,
            patch: None,
        });
        assert!(!supports(&provider, &consumer));
        consumer.minimum_os = None;
        assert!(!supports(&provider, &consumer));
        consumer = provider.clone();
        consumer.abi = None;
        assert!(!supports(&provider, &consumer));
        consumer.abi = Some(PlatformAbi::Musl);
        assert!(!supports(&provider, &consumer));
        consumer = provider.clone();
        consumer.minimum_os = Some(OsVersion {
            major: 5,
            minor: 15,
            patch: Some(0),
        });
        assert!(supports(&provider, &consumer));
        assert!(supports(&consumer, &provider));
    }
}
