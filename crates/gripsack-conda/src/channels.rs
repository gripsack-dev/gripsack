//! Channel canonicalization for resolve requests.
//!
//! Contract C1 declares channels as names or URLs. A bare channel name (one
//! path segment, no scheme, no slashes) canonicalizes to
//! `https://conda.anaconda.org/<name>`; absolute `https://` URLs pass through
//! unchanged. Userinfo credentials are a strict error, and everything else —
//! other schemes, relative paths, stray whitespace — is an `invalid_channel`
//! error, never a default.

use std::fmt;

/// A channel string could not be canonicalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidChannel {
    pub channel: String,
    pub reason: &'static str,
}

impl fmt::Display for InvalidChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "channel '{}': {}", self.channel, self.reason)
    }
}

impl std::error::Error for InvalidChannel {}

/// The canonical host bare channel names resolve against.
pub const DEFAULT_CHANNEL_BASE: &str = "https://conda.anaconda.org";

/// Canonicalizes one channel string to its canonical URL.
///
/// * `"conda-forge"` → `"https://conda.anaconda.org/conda-forge"`
/// * `"https://custom.example/chan"` → unchanged
/// * credentials, non-https schemes, relative paths, whitespace → `Err`
pub fn canonicalize_channel(raw: &str) -> Result<String, InvalidChannel> {
    let fail = |reason: &'static str| InvalidChannel {
        channel: raw.to_string(),
        reason,
    };
    if raw.is_empty() {
        return Err(fail("empty channel"));
    }
    if !raw.bytes().all(|b| !b.is_ascii_whitespace()) {
        return Err(fail("channel must not contain whitespace"));
    }
    if raw.contains("://") {
        // Absolute URL: anonymous HTTPS only.
        let Some(rest) = raw.strip_prefix("https://") else {
            return Err(fail("anonymous HTTPS only; scheme must be https"));
        };
        let authority = rest.split('/').next().unwrap_or("");
        if authority.is_empty() {
            return Err(fail("missing host"));
        }
        if authority.contains('@') {
            return Err(fail("userinfo credentials are a strict error"));
        }
        return Ok(raw.to_string());
    }
    if raw.contains('/') {
        return Err(fail(
            "neither a bare channel name nor an absolute https:// URL",
        ));
    }
    // Bare channel name: one path segment of conda's name alphabet.
    if raw.starts_with('.')
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(fail("not a valid bare channel name"));
    }
    Ok(format!("{DEFAULT_CHANNEL_BASE}/{raw}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_name_canonicalizes() {
        assert_eq!(
            canonicalize_channel("conda-forge").unwrap(),
            "https://conda.anaconda.org/conda-forge"
        );
        assert_eq!(
            canonicalize_channel("bioconda").unwrap(),
            "https://conda.anaconda.org/bioconda"
        );
    }

    #[test]
    fn absolute_https_passes_through() {
        assert_eq!(
            canonicalize_channel("https://custom.example/chan").unwrap(),
            "https://custom.example/chan"
        );
    }

    #[test]
    fn credentials_are_refused() {
        assert!(canonicalize_channel("https://user:pw@host/chan").is_err());
        assert!(canonicalize_channel("https://user@host/chan").is_err());
    }

    #[test]
    fn garbage_is_refused() {
        for raw in [
            "./local",
            "../up",
            "http://host/chan",
            "file:///chan",
            "conda-forge ",
            " conda-forge",
            "",
            "https:///no-host",
            ".hidden",
            "..",
        ] {
            assert!(
                canonicalize_channel(raw).is_err(),
                "channel {raw:?} must refuse"
            );
        }
    }
}
