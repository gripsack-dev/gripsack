//! Target compatibility for production workspace admission. Both recipe
//! producers and environment/image package selections call this kernel;
//! a missing ABI is not a wildcard and no checking-host fact participates.
//! The IR's typed wire conversion remains a separate admission boundary.

use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    Linux,
    Macos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetArch {
    X86_64,
    Aarch64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryAbi {
    Gnu,
    Musl,
    Darwin,
}

/// Normalized minimum OS requirement. The IR adapter maps an omitted
/// patch to zero before admission; these values are version components,
/// not host-detected versions or timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsRelease {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetRequirement {
    pub os: TargetOs,
    pub arch: TargetArch,
    pub abi: Option<BinaryAbi>,
    pub minimum_os: Option<OsRelease>,
}

pub open spec fn release_floor_le(provider: OsRelease, consumer: OsRelease) -> bool {
    provider.major < consumer.major
        || (provider.major == consumer.major && (
            provider.minor < consumer.minor
                || (provider.minor == consumer.minor && provider.patch <= consumer.patch)
        ))
}

/// A producer's minimum OS floor may not exceed its consumer's. This
/// comparison is lexicographic over normalized version components.
pub fn release_at_most(provider: OsRelease, consumer: OsRelease) -> (result: bool)
    ensures result == release_floor_le(provider, consumer),
{
    if provider.major != consumer.major {
        provider.major < consumer.major
    } else if provider.minor != consumer.minor {
        provider.minor < consumer.minor
    } else {
        provider.patch <= consumer.patch
    }
}

pub open spec fn target_compatible(provider: &TargetRequirement, consumer: &TargetRequirement) -> bool {
    provider.os == consumer.os
        && provider.arch == consumer.arch
        && provider.abi == consumer.abi
        && match (provider.minimum_os, consumer.minimum_os) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(floor), Some(required)) => release_floor_le(floor, required),
        }
}

/// Same OS, architecture and exact declared ABI; a provider without
/// a floor satisfies any consumer, while a consumer without a floor
/// cannot accept a provider with one. No host fact or layout rule is
/// inferred from these declarations.
pub fn supports_target(provider: &TargetRequirement, consumer: &TargetRequirement) -> (result: bool)
    ensures result == target_compatible(provider, consumer),
{
    let os_matches = matches!(
        (provider.os, consumer.os),
        (TargetOs::Linux, TargetOs::Linux) | (TargetOs::Macos, TargetOs::Macos)
    );
    let arch_matches = matches!(
        (provider.arch, consumer.arch),
        (TargetArch::X86_64, TargetArch::X86_64)
            | (TargetArch::Aarch64, TargetArch::Aarch64)
    );
    let abi_matches = matches!(
        (provider.abi, consumer.abi),
        (None, None)
            | (Some(BinaryAbi::Gnu), Some(BinaryAbi::Gnu))
            | (Some(BinaryAbi::Musl), Some(BinaryAbi::Musl))
            | (Some(BinaryAbi::Darwin), Some(BinaryAbi::Darwin))
    );
    proof {
        assert(os_matches == (provider.os == consumer.os));
        assert(arch_matches == (provider.arch == consumer.arch));
        assert(abi_matches == (provider.abi == consumer.abi));
    }
    if !os_matches || !arch_matches || !abi_matches {
        return false;
    }
    match (provider.minimum_os, consumer.minimum_os) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(floor), Some(required)) => release_at_most(floor, required),
    }
}

}
