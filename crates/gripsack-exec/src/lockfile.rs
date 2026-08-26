//! The lockfile: resolution frozen per host (0001 §4, 0008 §5).
//!
//! `locks/<host>.lock` lives in the env repo. `grip update` is the only
//! mutator; `apply` verifies against it — a hash mismatch is a hard
//! error (tampering signal), never silently re-pinned.
//!
//! Entry shape: `fetch` is the intent, `resolved` is the pin — a URL,
//! a version, and a content hash, uniform across fetcher kinds.

use gripsack_ir::FetchSpec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// The pin: what resolution produced for a fetch spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resolved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The registry's API asset endpoint — the authenticated download
    /// path for private releases (github releases).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
}

/// One module's resolved fetch: the spec plus the pin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockEntry {
    pub fetch: FetchSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Lockfile {
    #[serde(default)]
    pub modules: BTreeMap<String, LockEntry>,
}

pub fn path(repo: &Path, host: &str) -> PathBuf {
    repo.join("locks").join(format!("{host}.lock"))
}

pub fn read(repo: &Path, host: &str) -> Option<Lockfile> {
    let raw = std::fs::read(path(repo, host)).ok()?;
    serde_json::from_slice(&raw).ok()
}

pub fn write(repo: &Path, host: &str, lockfile: &Lockfile) -> io::Result<()> {
    let json = serde_json::to_string_pretty(lockfile)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    gripsack_store::fs::atomic_write(&path(repo, host), json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut lock = Lockfile::default();
        lock.modules.insert(
            "helix".into(),
            LockEntry {
                fetch: FetchSpec::Tarball {
                    url: "https://example.invalid/h.tar.xz".into(),
                    sha256: None,
                    api_url: None,
                },
                resolved: Some(Resolved {
                    url: None,
                    version: None,
                    sha256: Some("ab".repeat(32)),
                    api_url: None,
                }),
            },
        );
        write(dir.path(), "laptop", &lock).unwrap();
        let read_back = read(dir.path(), "laptop").unwrap();
        assert_eq!(lock, read_back);
        assert!(read(dir.path(), "otherhost").is_none());
    }
}
