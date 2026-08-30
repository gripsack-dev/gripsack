//! Host-platform asset resolution for bundled tools (pixi, deno).
//!
//! Each bundled tool release is pinned per platform — a version plus
//! one sha256 per supported asset. Verification stays exact: a
//! per-platform pin in the source, never a fetched sidecar checksum
//! (the sidecar travels the same channel as the asset, so it
//! authenticates nothing an attacker couldn't also swap).

use super::FetchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetTarget {
    /// The linux/x86_64 platform slot. Named for the musl-static
    /// flavor pixi ships (a musl binary runs on glibc too); deno
    /// maps it to its glibc asset and refuses actual-musl hosts
    /// upstream in provisioning.
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

    /// The vendor's asset triple — also the tarball's nested dir name.
    pub fn triple(&self) -> &'static str {
        match self {
            Self::LinuxX86_64Musl => "x86_64-unknown-linux-musl",
            Self::LinuxAarch64Musl => "aarch64-unknown-linux-musl",
            Self::MacosX86_64 => "x86_64-apple-darwin",
            Self::MacosAarch64 => "aarch64-apple-darwin",
        }
    }

    /// Fetch-spec placeholders (0016 §D1): `{system}` flake-style,
    /// `{target}` the rust triple, `{arch}`, `{arch.go}` goreleaser,
    /// `{os}`. Naming conventions upstream are a swamp — a small
    /// explicit set, not a pretend-universal one.
    pub fn placeholders(&self) -> [(&'static str, String); 5] {
        let (system, arch, arch_go, os) = match self {
            Self::LinuxX86_64Musl => ("x86_64-linux", "x86_64", "amd64", "linux"),
            Self::LinuxAarch64Musl => ("aarch64-linux", "aarch64", "arm64", "linux"),
            Self::MacosX86_64 => ("x86_64-darwin", "x86_64", "amd64", "darwin"),
            Self::MacosAarch64 => ("aarch64-darwin", "aarch64", "arm64", "darwin"),
        };
        [
            ("{system}", system.into()),
            ("{target}", self.triple().into()),
            ("{arch}", arch.into()),
            ("{arch.go}", arch_go.into()),
            ("{os}", os.into()),
        ]
    }

    /// Deno's asset names: glibc builds for Linux — deno ships no
    /// musl build, so a musl HOST is rejected before the download
    /// (frontend provisioning), never here.
    pub fn deno_name(&self) -> &'static str {
        match self {
            Self::LinuxX86_64Musl => "x86_64-unknown-linux-gnu",
            Self::LinuxAarch64Musl => "aarch64-unknown-linux-gnu",
            Self::MacosX86_64 => "x86_64-apple-darwin",
            Self::MacosAarch64 => "aarch64-apple-darwin",
        }
    }
}

/// One bundled tool release: version + per-platform asset hashes.
pub struct ToolRelease {
    pub version: &'static str,
    /// Download URL with `{version}`, `{triple}`, and `{denotarget}`
    /// placeholders.
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
        .replace("{denotarget}", target.deno_name());
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

/// deno — the TypeScript frontend's sandboxed runtime (plan/0013 D2):
/// the permission model is the reason it exists; nothing else about
/// the runtime is load-bearing here. Hashes from v2.9.6's per-asset
/// `.zip.sha256sum` sidecars (fetched once at pin time, verified
/// against the download — never fetched at runtime).
pub const DENO_RELEASE: ToolRelease = ToolRelease {
    version: "2.9.6",
    url_template: "https://github.com/denoland/deno/releases/download/v{version}/deno-{denotarget}.zip",
    sha256: &[
        (
            AssetTarget::LinuxX86_64Musl,
            "394f07f4da2bebe6ce6f1e7ce0fa16429b29b08c35e3fac3fe25972676dff4b2",
        ),
        (
            AssetTarget::LinuxAarch64Musl,
            "9a46afc6c392c7cd2ff71a31558935545b46408d0e87f7a86908c712721c046e",
        ),
        (
            AssetTarget::MacosX86_64,
            "7d4524b82bcc557fe020a1a5b56956ed42b992ae5b28026e8ad5d17329533f5f",
        ),
        (
            AssetTarget::MacosAarch64,
            "213a2f304f04d3c9cb5220669afad138f60a5aab1fe80962abdeb8f35807a472",
        ),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_table_covers_every_convention() {
        // 0016 §D1's table, pinned per target — the asset-naming swamp
        // is upstream's, so the mapping is exhaustively testable here
        let expect: [(AssetTarget, [&str; 5]); 4] = [
            (
                AssetTarget::LinuxX86_64Musl,
                [
                    "x86_64-linux",
                    "x86_64-unknown-linux-musl",
                    "x86_64",
                    "amd64",
                    "linux",
                ],
            ),
            (
                AssetTarget::LinuxAarch64Musl,
                [
                    "aarch64-linux",
                    "aarch64-unknown-linux-musl",
                    "aarch64",
                    "arm64",
                    "linux",
                ],
            ),
            (
                AssetTarget::MacosX86_64,
                [
                    "x86_64-darwin",
                    "x86_64-apple-darwin",
                    "x86_64",
                    "amd64",
                    "darwin",
                ],
            ),
            (
                AssetTarget::MacosAarch64,
                [
                    "aarch64-darwin",
                    "aarch64-apple-darwin",
                    "aarch64",
                    "arm64",
                    "darwin",
                ],
            ),
        ];
        for (target, values) in expect {
            let placeholders = target.placeholders();
            assert_eq!(placeholders.len(), values.len());
            for ((_, got), want) in placeholders.iter().zip(values) {
                assert_eq!(got, want, "{target:?}");
            }
        }
    }

    #[test]
    fn resolves_the_host_asset() {
        let (url, sha) = resolve(&PIXI_RELEASE).unwrap();
        assert!(url.contains("0.77.1"));
        assert!(url.contains(AssetTarget::current().unwrap().triple()));
        assert_eq!(sha.len(), 64);
    }

    #[test]
    fn bundled_releases_have_full_platform_tables() {
        for release in [&PIXI_RELEASE, &DENO_RELEASE] {
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

    #[test]
    fn deno_urls_use_the_glibc_linux_assets() {
        // deno ships no musl build: the linux slots must resolve to
        // the gnu assets, never a musl-named download
        for (target, asset) in [
            (AssetTarget::LinuxX86_64Musl, "x86_64-unknown-linux-gnu"),
            (AssetTarget::LinuxAarch64Musl, "aarch64-unknown-linux-gnu"),
        ] {
            assert_eq!(target.deno_name(), asset);
        }
        let (url, sha) = resolve(&DENO_RELEASE).unwrap();
        assert!(url.contains("deno-"));
        assert!(url.ends_with(".zip"));
        assert_eq!(sha.len(), 64);
    }
}
