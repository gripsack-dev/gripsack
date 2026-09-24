//! Per-output target/ABI/minimum-OS requirements (0052 A1-02). These
//! describe declared compatibility, never ambient host selection.

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
    /// A provider can satisfy a consumer only on the same OS,
    /// architecture and declared ABI. Its minimum OS floor must not
    /// exceed the consumer's. Missing ABI is not a wildcard.
    pub fn supports(&self, consumer: &Self) -> bool {
        self.os == consumer.os
            && self.arch == consumer.arch
            && self.abi == consumer.abi
            && match (self.minimum_os, consumer.minimum_os) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(producer), Some(requested)) => producer.at_most(requested),
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

impl OsVersion {
    /// Compare minimum requirements, not serialized identity. An
    /// omitted patch and explicit patch zero have the same floor.
    pub fn at_most(self, other: Self) -> bool {
        (self.major, self.minor, self.patch.unwrap_or(0))
            <= (other.major, other.minor, other.patch.unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut consumer = provider.clone();
        consumer.minimum_os = Some(OsVersion {
            major: 6,
            minor: 1,
            patch: None,
        });
        assert!(provider.supports(&consumer));
        consumer.minimum_os = Some(OsVersion {
            major: 4,
            minor: 19,
            patch: None,
        });
        assert!(!provider.supports(&consumer));
        consumer.minimum_os = None;
        assert!(!provider.supports(&consumer));
        consumer = provider.clone();
        consumer.abi = None;
        assert!(!provider.supports(&consumer));
        consumer.abi = Some(PlatformAbi::Musl);
        assert!(!provider.supports(&consumer));
        consumer = provider.clone();
        consumer.minimum_os = Some(OsVersion {
            major: 5,
            minor: 15,
            patch: Some(0),
        });
        assert!(provider.supports(&consumer));
        assert!(consumer.supports(&provider));
    }
}
