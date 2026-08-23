//! Release resolution for `github_release` (0002 §8): query the API,
//! pick the asset, return the pin. Runs in the core at lock/update
//! time so API traffic stays inside the throttle domains (0007 §4c).
//!
//! Auth: `GITHUB_TOKEN`/`GH_TOKEN` if set (60/hr anonymous otherwise).
//! `base_url` covers GitHub Enterprise.

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("http: {0}")]
    Http(Box<ureq::Error>),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("no release asset matching {asset:?} in {repo} (have: {})", .available.join(", "))]
    NoAsset {
        repo: String,
        asset: String,
        available: Vec<String>,
    },
    #[error("no releases found for {0}")]
    NoReleases(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<ureq::Error> for ResolveError {
    fn from(e: ureq::Error) -> Self {
        ResolveError::Http(Box::new(e))
    }
}

/// A resolved release: the pin that goes into the lockfile.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRelease {
    pub version: String,
    pub url: String,
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
}

/// The asset pattern's `{version}` placeholder, expanded against a tag.
/// `v25.07` and `25.07` both match.
pub fn expand_asset_pattern(pattern: &str, tag: &str) -> Vec<String> {
    let bare = tag.strip_prefix('v').unwrap_or(tag);
    vec![
        pattern.replace("{version}", tag),
        pattern.replace("{version}", bare),
    ]
}

/// Match an asset list against a pattern; pure — unit-tested offline.
pub fn pick_asset(
    repo: &str,
    pattern: &str,
    tag: &str,
    assets: &[(String, String)],
) -> Result<ResolvedRelease, ResolveError> {
    for candidate in expand_asset_pattern(pattern, tag) {
        if let Some((_, url)) = assets.iter().find(|(name, _)| *name == candidate) {
            return Ok(ResolvedRelease {
                version: tag.to_string(),
                url: url.clone(),
            });
        }
    }
    Err(ResolveError::NoAsset {
        repo: repo.to_string(),
        asset: pattern.to_string(),
        available: assets.iter().map(|(n, _)| n.clone()).collect(),
    })
}

/// Resolve the latest release of `repo` to a concrete asset URL.
pub fn resolve_latest(
    repo: &str,
    asset_pattern: &str,
    base_url: Option<&str>,
) -> Result<ResolvedRelease, ResolveError> {
    let base = base_url.unwrap_or("https://api.github.com");
    let url = format!("{base}/repos/{repo}/releases/latest");
    let mut request = ureq::get(&url).set("User-Agent", "gripsack");
    if let Ok(token) = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")) {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let release: Release = request.call()?.into_json()?;
    let assets: Vec<(String, String)> = release
        .assets
        .into_iter()
        .map(|a| (a.name, a.browser_download_url))
        .collect();
    pick_asset(repo, asset_pattern, &release.tag_name, &assets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> Vec<(String, String)> {
        vec![
            (
                "helix-25.07.1-x86_64-linux.tar.xz".into(),
                "https://x/linux".into(),
            ),
            (
                "helix-25.07.1-aarch64-macos.tar.xz".into(),
                "https://x/macos".into(),
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
            &[("tool-1.2.tar.gz".into(), "https://x".into())],
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
