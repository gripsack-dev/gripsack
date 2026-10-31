//! Hash-pinned binary wheel requests and frozen metadata. Wheel installation,
//! Python compatibility and dependency solving belong to the helper, not IR.
use serde::{Deserialize, Serialize};

const MAX_WHEEL_VERSION_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WheelSource {
    pub name: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedWheel {
    pub name: String,
    pub version: String,
    pub filename: String,
    pub url: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_python: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_dist: Vec<String>,
}

fn metadata(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

fn identity(name: &str, version: &str, sha256: &str) -> Result<(), &'static str> {
    if !name.split('-').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    }) {
        return Err("wheel names must use canonical lowercase PEP503 spelling");
    }
    if version.is_empty()
        || version.len() > MAX_WHEEL_VERSION_BYTES
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!+-_".contains(&byte))
    {
        return Err("wheel version must be at most 256 path-safe ASCII version characters");
    }
    if super::identity::ArtifactDigest::parse(sha256).is_err() {
        return Err("wheel sha256 must be exactly 64 lowercase hexadecimal characters");
    }
    Ok(())
}

fn wheel_url(value: &str) -> Result<url::Url, &'static str> {
    if !metadata(value) || !value.starts_with("https://") || value.contains('\\') {
        return Err("wheel URL must be an explicit HTTPS artifact URL without controls");
    }
    let url = url::Url::parse(value).map_err(|_| "invalid wheel URL")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || value[8..]
            .split('/')
            .next()
            .is_some_and(|authority| authority.contains('@'))
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("wheel URL must be HTTPS without credentials, query or fragment");
    }
    let filename = url.path().rsplit('/').next().unwrap_or("");
    if !filename.ends_with(".whl")
        || filename.len() <= 4
        || !filename
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.+-".contains(&byte))
    {
        return Err("wheel URL must end in an unescaped binary .whl filename");
    }
    Ok(url)
}

impl WheelSource {
    pub fn validate(&self) -> Result<(), &'static str> {
        identity(&self.name, &self.version, &self.sha256)?;
        wheel_url(&self.url)?;
        Ok(())
    }
}

pub(crate) fn validate_sources(wheels: &[WheelSource]) -> Result<(), &'static str> {
    let mut names = std::collections::BTreeSet::new();
    let mut urls = std::collections::BTreeSet::new();
    let mut hashes = std::collections::BTreeSet::new();
    for wheel in wheels {
        wheel.validate()?;
        if !names.insert(&wheel.name) || !urls.insert(&wheel.url) || !hashes.insert(&wheel.sha256) {
            return Err("wheel requests cannot duplicate a name, URL or artifact hash");
        }
    }
    Ok(())
}

impl LockedWheel {
    pub fn validate(&self) -> Result<(), &'static str> {
        identity(&self.name, &self.version, &self.sha256)?;
        let url = wheel_url(&self.url)?;
        if url.path().rsplit('/').next() != Some(self.filename.as_str()) {
            return Err("frozen wheel filename does not match its artifact URL");
        }
        if self.size == Some(0)
            || self
                .requires_python
                .as_deref()
                .is_some_and(|value| !metadata(value))
        {
            return Err("invalid frozen wheel size or Requires-Python metadata");
        }
        if self.requires_dist.iter().any(|value| !metadata(value))
            || self.requires_dist.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err("Requires-Dist metadata must be nonempty, sorted and unique");
        }
        Ok(())
    }
}
