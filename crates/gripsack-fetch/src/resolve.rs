//! Release resolution for `github_release` (0002 §8): query the API,
//! pick the asset, return the pin. Runs in the core at lock/update
//! time so API traffic stays inside the throttle domains (0007 §4c).
//!
//! Auth: `GITHUB_TOKEN`/`GH_TOKEN` if set (60/hr anonymous otherwise).
//! `base_url` covers GitHub Enterprise.

use crate::http::RequestKind;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error(transparent)]
    Acquisition(#[from] crate::FetchError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("no release asset matching {asset:?} in {repo} (have: {})", .available.join(", "))]
    NoAsset {
        repo: String,
        asset: String,
        available: Vec<String>,
    },
    #[error(transparent)]
    Placeholder(#[from] crate::PlaceholderError),
}

impl ResolveError {
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Acquisition(error) => error.http_status(),
            _ => None,
        }
    }
}

/// A resolved release: the pin that goes into the lockfile.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRelease {
    pub version: String,
    pub url: String,
    /// The API asset endpoint, when the registry has one (GitHub) —
    /// the authenticated download path for private releases.
    pub api_url: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    /// The API asset endpoint — the only download path that honors a
    /// token; the browser URL needs a browser session on private/GHE
    /// releases (enterprise review finding #1).
    url: String,
}

/// Match an asset list against a pattern; pure — unit-tested offline.
/// `assets` is (name, browser URL, API URL).
pub fn pick_asset(
    repo: &str,
    pattern: &str,
    tag: &str,
    assets: &[(String, String, String)],
) -> Result<ResolvedRelease, ResolveError> {
    for candidate in crate::placeholders::asset_patterns(pattern, tag)? {
        if let Some((_, url, api_url)) = assets.iter().find(|(name, _, _)| *name == candidate) {
            return Ok(ResolvedRelease {
                version: tag.to_string(),
                url: url.clone(),
                api_url: Some(api_url.clone()),
            });
        }
    }
    Err(ResolveError::NoAsset {
        repo: repo.to_string(),
        asset: pattern.to_string(),
        available: assets.iter().map(|(n, _, _)| n.clone()).collect(),
    })
}

/// What a plugin release resolution yields: the tarball download plus
/// its mandatory checksum (from the `<asset>.sha256` sidecar asset).
pub struct PluginRelease {
    pub version: String,
    pub url: String,
    pub api_url: Option<String>,
    pub sha256: String,
}

/// Find `asset_name` and its mandatory `.sha256` sidecar in a release,
/// returning the asset plus the validated pin (missing sidecar =
/// failed install, never a warning — krew rule).
fn asset_with_sidecar<'a>(
    context: &crate::FetchContext,
    release: &'a Release,
    repo: &str,
    asset_name: &str,
) -> Result<(&'a Asset, String), ResolveError> {
    let sidecar_name = format!("{asset_name}.sha256");
    let mut asset = None;
    let mut sidecar = None;
    for a in &release.assets {
        if a.name == asset_name {
            asset = Some(a);
        } else if a.name == sidecar_name {
            sidecar = Some(a);
        }
    }
    let asset = asset.ok_or_else(|| ResolveError::NoAsset {
        repo: repo.to_string(),
        asset: asset_name.to_string(),
        available: release.assets.iter().map(|a| a.name.clone()).collect(),
    })?;
    let sidecar = sidecar.ok_or_else(|| ResolveError::NoAsset {
        repo: repo.to_string(),
        asset: sidecar_name,
        available: release.assets.iter().map(|a| a.name.clone()).collect(),
    })?;
    // the sidecar's first token is the hash
    let sha_text =
        crate::fetch::tarball::text(context, &sidecar.browser_download_url, Some(&sidecar.url))
            .map_err(|e| ResolveError::NoAsset {
                repo: repo.to_string(),
                asset: format!("readable sha256 sidecar ({e})"),
                available: vec![],
            })?;
    let sha256 = sha_text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ResolveError::NoAsset {
            repo: repo.to_string(),
            asset: format!("a 64-hex sha256 in the sidecar (got {sha256:?})"),
            available: vec![],
        });
    }
    Ok((asset, sha256))
}

/// Resolve a plugin's release: the `<exe>-<tag>-<triple>.tar.gz` asset
/// for this platform plus its sha256 sidecar (missing sidecar = failed
/// install, never a warning — krew rule). `tag` pins; None = latest.
pub(crate) fn resolve_plugin_release(
    context: &crate::FetchContext,
    repo: &str,
    exe: &str,
    tag: Option<&str>,
) -> Result<PluginRelease, ResolveError> {
    let target = crate::host::AssetTarget::current().ok_or_else(|| ResolveError::NoAsset {
        repo: repo.to_string(),
        asset: "a supported platform".into(),
        available: vec![],
    })?;
    let base = normalize_base(None);
    let url = match tag {
        Some(t) => format!("{base}/repos/{repo}/releases/tags/{t}"),
        None => format!("{base}/repos/{repo}/releases/latest"),
    };
    // tags may be written v1.0 or 1.0 — try the other form on 404
    // (the same convention pick_asset uses for asset names)
    let release: Release = match context.json(&url, RequestKind::GithubMetadata) {
        Ok(release) => release,
        Err(e) => {
            let alternate = tag.and_then(|t| {
                if e.http_status() == Some(404) {
                    let alt = t
                        .strip_prefix('v')
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| format!("v{t}"));
                    Some(format!("{base}/repos/{repo}/releases/tags/{alt}"))
                } else {
                    None
                }
            });
            match alternate {
                Some(url) => context.json(&url, RequestKind::GithubMetadata)?,
                None => return Err(e.into()),
            }
        }
    };
    let tag_name = release.tag_name.clone();
    let bare = tag_name.strip_prefix('v').unwrap_or(&tag_name);
    let asset_name = format!("{exe}-{bare}-{}.tar.gz", target.triple());
    let (asset, sha256) = asset_with_sidecar(context, &release, repo, &asset_name)?;
    Ok(PluginRelease {
        version: tag_name,
        url: asset.browser_download_url.clone(),
        api_url: Some(asset.url.clone()),
        sha256,
    })
}

/// base_url may be the bare GHE host ("https://ghe.example.com") —
/// the API lives under /api/v3; normalize instead of failing on the
/// HTML index page with a JSON parse error (enterprise finding #4).
fn normalize_base(base_url: Option<&str>) -> String {
    match base_url {
        None => "https://api.github.com".to_string(),
        Some(b) if b.trim_end_matches('/') == "https://api.github.com" => {
            b.trim_end_matches('/').to_string()
        }
        Some(b) => {
            let b = b.trim_end_matches('/');
            if b.ends_with("/api/v3") {
                b.to_string()
            } else {
                format!("{b}/api/v3")
            }
        }
    }
}

/// Resolve a release of `repo` to a concrete asset. `version` pins the
/// tag (fetched by `/releases/tags/<tag>`); None resolves latest. Auth
/// is host-scoped (http::auth_header) — tokens never cross hosts.
pub(crate) fn resolve_latest(
    context: &crate::FetchContext,
    repo: &str,
    asset_pattern: &str,
    base_url: Option<&str>,
    version: Option<&str>,
) -> Result<ResolvedRelease, ResolveError> {
    let base = normalize_base(base_url);
    let url = match version {
        Some(tag) => format!("{base}/repos/{repo}/releases/tags/{tag}"),
        None => format!("{base}/repos/{repo}/releases/latest"),
    };
    let release: Release = context.json(&url, RequestKind::GithubMetadata)?;
    let assets: Vec<(String, String, String)> = release
        .assets
        .into_iter()
        .map(|a| (a.name, a.browser_download_url, a.url))
        .collect();
    pick_asset(repo, asset_pattern, &release.tag_name, &assets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> Vec<(String, String, String)> {
        vec![
            (
                "helix-25.07.1-x86_64-linux.tar.xz".into(),
                "https://x/linux".into(),
                "https://api.x/assets/1".into(),
            ),
            (
                "helix-25.07.1-aarch64-macos.tar.xz".into(),
                "https://x/macos".into(),
                "https://api.x/assets/2".into(),
            ),
        ]
    }

    #[test]
    fn picks_expanded_asset() {
        let r = pick_asset(
            "helix-editor/helix",
            "helix-{version}-x86_64-linux.tar.xz",
            "25.07.1",
            &assets(),
        )
        .unwrap();
        assert_eq!(r.url, "https://x/linux");
        assert_eq!(r.version, "25.07.1");
    }

    #[test]
    fn v_prefix_variants_match() {
        let r = pick_asset(
            "r",
            "tool-{version}.tar.gz",
            "v1.2",
            &[(
                "tool-1.2.tar.gz".into(),
                "https://x".into(),
                "https://api.x/1".into(),
            )],
        )
        .unwrap();
        assert_eq!(r.url, "https://x");
    }

    #[test]
    fn missing_asset_lists_available() {
        let err = pick_asset("r", "nope-{version}", "1.0", &assets()).unwrap_err();
        match err {
            ResolveError::NoAsset { available, .. } => {
                assert!(available.iter().any(|a| a.contains("linux")))
            }
            other => panic!("{other:?}"),
        }
    }
}

#[cfg(test)]
mod base_url_tests {
    use super::normalize_base;

    #[test]
    fn bare_ghe_host_gets_api_v3() {
        assert_eq!(
            normalize_base(Some("https://ghe.example.com")),
            "https://ghe.example.com/api/v3"
        );
        assert_eq!(
            normalize_base(Some("https://ghe.example.com/api/v3")),
            "https://ghe.example.com/api/v3"
        );
        assert_eq!(
            normalize_base(Some("https://ghe.example.com/api/v3/")),
            "https://ghe.example.com/api/v3"
        );
        assert_eq!(normalize_base(None), "https://api.github.com");
    }
}
