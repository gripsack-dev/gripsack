//! Target compatibility for production workspace admission. Both recipe
//! producers and environment/image package selections call this rule;
//! a missing ABI is not a wildcard and no checking-host fact participates.
//! The IR's typed wire conversion remains a separate admission boundary.

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

/// A producer's minimum OS floor may not exceed its consumer's. This
/// comparison is lexicographic over normalized version components.
pub fn release_at_most(provider: OsRelease, consumer: OsRelease) -> bool {
    (provider.major, provider.minor, provider.patch)
        <= (consumer.major, consumer.minor, consumer.patch)
}

/// Same OS, architecture and exact declared ABI; a provider without
/// a floor satisfies any consumer, while a consumer without a floor
/// cannot accept a provider with one. No host fact or layout rule is
/// inferred from these declarations.
pub fn supports_target(provider: &TargetRequirement, consumer: &TargetRequirement) -> bool {
    if provider.os != consumer.os || provider.arch != consumer.arch || provider.abi != consumer.abi
    {
        return false;
    }
    match (provider.minimum_os, consumer.minimum_os) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(floor), Some(required)) => release_at_most(floor, required),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OS: [TargetOs; 2] = [TargetOs::Linux, TargetOs::Macos];
    const ARCH: [TargetArch; 2] = [TargetArch::X86_64, TargetArch::Aarch64];
    const ABI: [Option<BinaryAbi>; 4] = [
        None,
        Some(BinaryAbi::Gnu),
        Some(BinaryAbi::Musl),
        Some(BinaryAbi::Darwin),
    ];

    fn release(major: u16, minor: u16, patch: u16) -> OsRelease {
        OsRelease {
            major,
            minor,
            patch,
        }
    }

    /// Every pair of components drawn around each other's value: the order is
    /// lexicographic, major first.
    #[test]
    fn release_floors_compare_lexicographically() {
        let values = [0, 1, 2, u16::MAX];
        for a in values.iter().flat_map(|&x| values.map(move |y| (x, y))) {
            for b in values.iter().flat_map(|&x| values.map(move |y| (x, y))) {
                for (pa, pb) in [(0, 0), (0, 1), (1, 0)] {
                    let provider = release(a.0, a.1, pa);
                    let consumer = release(b.0, b.1, pb);
                    let expected =
                        a.0 < b.0 || (a.0 == b.0 && (a.1 < b.1 || (a.1 == b.1 && pa <= pb)));
                    assert_eq!(
                        release_at_most(provider, consumer),
                        expected,
                        "{provider:?} {consumer:?}"
                    );
                }
            }
        }
    }

    /// Every OS × architecture × ABI pair, with every floor shape: the same
    /// platform and exact ABI (no ABI is not a wildcard), and a provider floor
    /// no later than the consumer's; a consumer without a floor accepts only a
    /// provider without one.
    #[test]
    fn every_target_pair_is_exact() {
        let floors = [None, Some(release(10, 0, 0)), Some(release(11, 2, 0))];
        for &(po, co) in &OS
            .iter()
            .flat_map(|&a| OS.map(move |b| (a, b)))
            .collect::<Vec<_>>()
        {
            for &(pa, ca) in &ARCH
                .iter()
                .flat_map(|&a| ARCH.map(move |b| (a, b)))
                .collect::<Vec<_>>()
            {
                for pb in ABI {
                    for cb in ABI {
                        for pf in floors {
                            for cf in floors {
                                let provider = TargetRequirement {
                                    os: po,
                                    arch: pa,
                                    abi: pb,
                                    minimum_os: pf,
                                };
                                let consumer = TargetRequirement {
                                    os: co,
                                    arch: ca,
                                    abi: cb,
                                    minimum_os: cf,
                                };
                                let floor = match (pf, cf) {
                                    (None, _) => true,
                                    (Some(_), None) => false,
                                    (Some(p), Some(c)) => (p.major, p.minor) <= (c.major, c.minor),
                                };
                                let expected = po == co && pa == ca && pb == cb && floor;
                                assert_eq!(
                                    supports_target(&provider, &consumer),
                                    expected,
                                    "{provider:?} {consumer:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
