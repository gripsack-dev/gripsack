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
    /// Present when the registry gives us the hash upfront (brew bottles).
    pub sha256: Option<String>,
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
                sha256: None,
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

// ---------------------------------------------------------------- brew

#[derive(Deserialize)]
pub struct Formula {
    pub versions: Versions,
    pub bottle: Bottles,
}

#[derive(Deserialize)]
pub struct Versions {
    pub stable: String,
}

#[derive(Deserialize)]
pub struct Bottles {
    pub stable: BottleStable,
}

#[derive(Deserialize)]
pub struct BottleStable {
    pub files: std::collections::BTreeMap<String, BottleFile>,
}

#[derive(Deserialize)]
pub struct BottleFile {
    pub url: String,
    pub sha256: String,
}

/// The bottle file key for this platform: linux is `x86_64_linux`;
/// macOS keys are arm64_* / plain names (sonoma…) — take the newest.
pub fn bottle_key(files: &std::collections::BTreeMap<String, BottleFile>) -> Option<&str> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    if os == "linux" {
        return if arch == "x86_64" {
            Some("x86_64_linux")
        } else {
            Some("arm64_linux")
        }
        .filter(|k| files.contains_key(*k));
    }
    if os == "macos" {
        if arch == "aarch64" {
            return files
                .keys()
                .rfind(|k| k.starts_with("arm64_"))
                .map(String::as_str);
        }
        return files
            .keys()
            .rfind(|k| !k.starts_with("arm64") && *k != "all")
            .map(String::as_str);
    }
    None
}

/// Resolve a brew formula to a bottle URL — the sha256 comes from the
/// formula JSON, so pinning needs no download.
pub fn resolve_brew(formula: &str) -> Result<ResolvedRelease, ResolveError> {
    let url = format!("https://formulae.brew.sh/api/formula/{formula}.json");
    let f: Formula = ureq::get(&url)
        .set("User-Agent", "gripsack")
        .call()?
        .into_json()?;
    let key = bottle_key(&f.bottle.stable.files).ok_or_else(|| ResolveError::NoAsset {
        repo: formula.to_string(),
        asset: "bottle for this platform".into(),
        available: f.bottle.stable.files.keys().cloned().collect(),
    })?;
    let file = &f.bottle.stable.files[key];
    Ok(ResolvedRelease {
        version: f.versions.stable,
        url: file.url.clone(),
        sha256: Some(file.sha256.clone()),
    })
}

/// ghcr.io blobs need an anonymous bearer token.
pub fn ghcr_token(scope_repo: &str) -> Result<String, ResolveError> {
    #[derive(Deserialize)]
    struct Token {
        token: String,
    }
    let url = format!("https://ghcr.io/token?scope=repository:{scope_repo}:pull");
    let t: Token = ureq::get(&url).call()?.into_json()?;
    Ok(t.token)
}
