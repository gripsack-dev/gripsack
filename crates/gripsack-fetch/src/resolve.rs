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
    /// The API asset endpoint, when the registry has one (GitHub) —
    /// the authenticated download path for private releases.
    pub api_url: Option<String>,
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
    /// The API asset endpoint — the only download path that honors a
    /// token; the browser URL needs a browser session on private/GHE
    /// releases (enterprise review finding #1).
    url: String,
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
/// `assets` is (name, browser URL, API URL).
pub fn pick_asset(
    repo: &str,
    pattern: &str,
    tag: &str,
    assets: &[(String, String, String)],
) -> Result<ResolvedRelease, ResolveError> {
    for candidate in expand_asset_pattern(pattern, tag) {
        if let Some((_, url, api_url)) = assets.iter().find(|(name, _, _)| *name == candidate) {
            return Ok(ResolvedRelease {
                version: tag.to_string(),
                url: url.clone(),
                api_url: Some(api_url.clone()),
                sha256: None,
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
        crate::fetch::tarball::read_url_authed(&sidecar.browser_download_url, Some(&sidecar.url))
            .map_err(|e| ResolveError::NoAsset {
            repo: repo.to_string(),
            asset: format!("readable sha256 sidecar ({e})"),
            available: vec![],
        })?;
    let sha256 = String::from_utf8_lossy(&sha_text)
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

/// A grip self-update resolution: the newest `core-v*` release (py/ts
/// tags ship alongside, so /releases/latest is NOT the answer), this
/// platform's `gripsack-<version>-<triple>.tar.gz`, and its sha256.
pub struct SelfRelease {
    pub version: String,
    pub url: String,
    pub api_url: Option<String>,
    pub sha256: String,
}

fn releases_tags(releases: &[Release]) -> Vec<String> {
    releases.iter().map(|r| r.tag_name.clone()).collect()
}

pub fn resolve_self_release() -> Result<SelfRelease, ResolveError> {
    let target = crate::host::AssetTarget::current().ok_or_else(|| ResolveError::NoAsset {
        repo: "gripsack-dev/gripsack".into(),
        asset: "a supported platform".into(),
        available: vec![],
    })?;
    // GRIPSACK_UPDATE_API is the test seam: a verbatim base URL for a
    // loopback fixture server (no /api/v3 normalization).
    let base = match std::env::var("GRIPSACK_UPDATE_API") {
        Ok(b) => b.trim_end_matches('/').to_string(),
        Err(_) => "https://api.github.com".to_string(),
    };
    // three tag types (core/py/ts) per version in creation order —
    // page deep enough that core-v* never falls off
    let url = format!("{base}/repos/gripsack-dev/gripsack/releases?per_page=100");
    let mut request = crate::http::get(&url).set("User-Agent", "gripsack");
    if let Some(header) = crate::http::auth_header(&url) {
        request = request.set("Authorization", &header);
    }
    let releases: Vec<Release> = request.call()?.into_json()?;
    let tags = releases_tags(&releases);
    let release = releases
        .into_iter()
        .find(|r| r.tag_name.starts_with("core-v"))
        .ok_or_else(|| ResolveError::NoAsset {
            repo: "gripsack-dev/gripsack".into(),
            asset: "a core-v* release".into(),
            available: tags,
        })?;
    let version = release.tag_name.trim_start_matches("core-v").to_string();
    let asset_name = format!("gripsack-{version}-{}.tar.gz", target.triple());
    let (asset, sha256) = asset_with_sidecar(&release, "gripsack-dev/gripsack", &asset_name)?;
    Ok(SelfRelease {
        version,
        url: asset.browser_download_url.clone(),
        api_url: Some(asset.url.clone()),
        sha256,
    })
}

/// Resolve a plugin's release: the `<exe>-<tag>-<triple>.tar.gz` asset
/// for this platform plus its sha256 sidecar (missing sidecar = failed
/// install, never a warning — krew rule). `tag` pins; None = latest.
pub fn resolve_plugin_release(
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
    let mut request = crate::http::get(&url).set("User-Agent", "gripsack");
    if let Some(header) = crate::http::auth_header(&url) {
        request = request.set("Authorization", &header);
    }
    // tags may be written v1.0 or 1.0 — try the other form on 404
    // (the same convention pick_asset uses for asset names)
    let release: Release = match request.call() {
        Ok(r) => r.into_json()?,
        Err(e) => {
            let alternate = tag.and_then(|t| {
                if e.to_string().contains("404") {
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
                Some(url) => {
                    let mut request = crate::http::get(&url).set("User-Agent", "gripsack");
                    if let Some(header) = crate::http::auth_header(&url) {
                        request = request.set("Authorization", &header);
                    }
                    request.call()?.into_json()?
                }
                None => return Err(ResolveError::Http(Box::new(e))),
            }
        }
    };
    let tag_name = release.tag_name.clone();
    let bare = tag_name.strip_prefix('v').unwrap_or(&tag_name);
    let asset_name = format!("{exe}-{bare}-{}.tar.gz", target.triple());
    let (asset, sha256) = asset_with_sidecar(&release, repo, &asset_name)?;
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
pub fn resolve_latest(
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
    let mut request = crate::http::get(&url).set("User-Agent", "gripsack");
    if let Some(header) = crate::http::auth_header(&url) {
        request = request.set("Authorization", &header);
    }
    let release: Release = request.call()?.into_json()?;
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
    let f: Formula = crate::http::get(&url)
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
        api_url: None, // brew bottles are CDN downloads, no API path
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
    let t: Token = crate::http::get(&url).call()?.into_json()?;
    Ok(t.token)
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
