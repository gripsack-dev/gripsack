//! Host-platform asset resolution for bundled tools (pixi, uv).
//!
//! Each bundled tool release is pinned per platform — a version plus
//! one sha256 per supported asset. Verification stays exact: a
//! per-platform pin in the source, never a fetched sidecar checksum
//! (the sidecar travels the same channel as the asset, so it
//! authenticates nothing an attacker couldn't also swap).

use super::FetchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetTarget {
    LinuxX86_64Musl,
    LinuxAarch64Musl,
    MacosX86_64,
    MacosAarch64,
}

impl AssetTarget {
    pub fn current() -> Option<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Some(Self::LinuxX86_64Musl),
            ("linux", "aarch64") => Some(Self::LinuxAarch64Musl),
            ("macos", "x86_64") => Some(Self::MacosX86_64),
            ("macos", "aarch64") => Some(Self::MacosAarch64),
            _ => None, // windows: WSL is the story
        }
    }

    /// bun's asset names follow their own convention (linux-x64, …).
    pub fn bun_name(&self) -> &'static str {
        match self {
            Self::LinuxX86_64Musl => "linux-x64",
            Self::LinuxAarch64Musl => "linux-aarch64",
            Self::MacosX86_64 => "darwin-x64",
            Self::MacosAarch64 => "darwin-aarch64",
        }
    }

    /// The vendor's asset triple — also the tarball's nested dir name.
    pub fn triple(&self) -> &'static str {
        match self {
            Self::LinuxX86_64Musl => "x86_64-unknown-linux-musl",
            Self::LinuxAarch64Musl => "aarch64-unknown-linux-musl",
            Self::MacosX86_64 => "x86_64-apple-darwin",
            Self::MacosAarch64 => "aarch64-apple-darwin",
        }
    }
}

/// One bundled tool release: version + per-platform asset hashes.
pub struct ToolRelease {
    pub version: &'static str,
    /// Download URL with `{version}` and `{triple}` placeholders.
    pub url_template: &'static str,
    pub sha256: &'static [(AssetTarget, &'static str)],
}

/// The (url, sha256) for the host we're running on.
pub fn resolve(release: &ToolRelease) -> Result<(String, &'static str), FetchError> {
    let target = AssetTarget::current().ok_or_else(|| FetchError::Http {
        url: release.url_template.to_string(),
        reason: format!(
            "unsupported host platform ({}-{}) — linux and macOS are supported (Windows: use WSL)",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    })?;
    let sha = release
        .sha256
        .iter()
        .find(|(t, _)| *t == target)
        .map(|(_, s)| *s)
        .ok_or_else(|| FetchError::Http {
            url: release.url_template.to_string(),
            reason: format!("no pinned hash for {:?} at {}", target, release.version),
        })?;
    let url = release
        .url_template
        .replace("{version}", release.version)
        .replace("{triple}", target.triple())
        .replace("{buntarget}", target.bun_name());
    Ok((url, sha))
}

pub const PIXI_RELEASE: ToolRelease = ToolRelease {
    version: "0.77.1",
    url_template: "https://github.com/prefix-dev/pixi/releases/download/v{version}/pixi-{triple}.tar.gz",
    sha256: &[
        (
            AssetTarget::LinuxX86_64Musl,
            "74dbe15255c763396ac847eb455573c6400e9caf06614c3fc00cb3de6d1099d5",
        ),
        (
            AssetTarget::LinuxAarch64Musl,
            "f49325536f9d3e68bf60115983f583463bdc68ad1c9e772a072a7f47dbf42d20",
        ),
        (
            AssetTarget::MacosX86_64,
            "75f65e8c34b5435ce00984a671011bbea0b4e9ed18c34ba7227048c5c424392c",
        ),
        (
            AssetTarget::MacosAarch64,
            "2b1280f11ed058477eb2752baac875615eb9ad48002c0536d8063d4249e853c2",
        ),
    ],
};

/// bun — the TypeScript frontend's runtime (single binary, runs TS
/// natively; no node+transpile chain). Hashes from bun-v1.4.0's
/// SHASUMS256.txt.
pub const BUN_RELEASE: ToolRelease = ToolRelease {
    version: "1.4.0",
    url_template: "https://github.com/oven-sh/bun/releases/download/bun-v{version}/bun-{buntarget}.zip",
    sha256: &[
        (
            AssetTarget::LinuxX86_64Musl,
            "2d03fb5fb83ac8b567aca0a281b2ce1a1a19d488f56c2968d88c3f25e92fe452",
        ),
        (
            AssetTarget::LinuxAarch64Musl,
            "4b1a332ee861983eb93bcfe6f770fff94e3e31b2c388bdaea3c8ed35e58eed0e",
        ),
        (
            AssetTarget::MacosAarch64,
            "c669e97f6164e1c96e0701748db98dfa77492908cbd8394c7557134a735de381",
        ),
        (
            AssetTarget::MacosX86_64,
            "1d0211b8f1dc991182344687ad15e72ee86f154845a5f7fa477994cd341dd9b0",
        ),
    ],
};

pub const UV_RELEASE: ToolRelease = ToolRelease {
    version: "0.12.5",
    url_template: "https://github.com/astral-sh/uv/releases/download/{version}/uv-{triple}.tar.gz",
    sha256: &[
        (
            AssetTarget::LinuxX86_64Musl,
            "a4742988791c9aeae68c78150d6cba762062ad2a47e53738c2779d2b596bfcdb",
        ),
        (
            AssetTarget::LinuxAarch64Musl,
            "8767a0e77f2cd45436401b1b42bf7e9ed5a4a91a74a5305d6fe93249d0f6dbc5",
        ),
        (
            AssetTarget::MacosX86_64,
            "b3b2137477cf96c9686ebfb71524614cec780c673fd73e59bce099aef02e70e8",
        ),
        (
            AssetTarget::MacosAarch64,
            "5bb0e5fe008a773c3dbcb97ff79cd89e1241464fe9d2f986d52ad8f1b037bd62",
        ),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_the_host_asset() {
        let (url, sha) = resolve(&PIXI_RELEASE).unwrap();
        assert!(url.contains("0.77.1"));
        assert!(url.contains(AssetTarget::current().unwrap().triple()));
        assert_eq!(sha.len(), 64);
    }

    #[test]
    fn uv_and_pixi_have_full_platform_tables() {
        for release in [&PIXI_RELEASE, &UV_RELEASE] {
            for target in [
                AssetTarget::LinuxX86_64Musl,
                AssetTarget::LinuxAarch64Musl,
                AssetTarget::MacosX86_64,
                AssetTarget::MacosAarch64,
            ] {
                assert!(
                    release.sha256.iter().any(|(t, _)| *t == target),
                    "missing hash for {:?} in {}",
                    target,
                    release.url_template
                );
            }
        }
    }
}
