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
    /// Canonical tree hash of the published store tree (0014) — the
    /// content-addressed path key. Distinct from `sha256`, which is the
    /// transport hash of the raw download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree256: Option<String>,
    /// Canonical overlay hash of the repo-sourced `from` files: a
    /// config tree that gains a file moves this WITHOUT moving the
    /// transport pin — presence checks and `grip update` compare it so
    /// a stale tree256 is never trusted. None when no entry's `from`
    /// exists in the repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo256: Option<String>,
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

/// Why a lockfile could not be read: absent (first run — pinning is
/// TOFU by design) vs present-but-unusable. A corrupt lock must never
/// read as "no lock": update would rewrite the whole file from nothing
/// and silently erase every other module's pin.
#[derive(Debug)]
pub enum LockRead {
    Missing,
    Corrupt(String),
    Parsed(Lockfile),
}

/// Hash-shaped pin fields: 64 lowercase hex chars. Anything else means
/// a hand-edited or truncated lock — refuse it before the values flow
/// into store paths (a short `tree256` used to panic the slice).
fn pin_is_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_pins(lock: &Lockfile) -> Result<(), String> {
    for (name, entry) in &lock.modules {
        let Some(resolved) = &entry.resolved else {
            continue;
        };
        for (field, value) in [
            ("sha256", &resolved.sha256),
            ("tree256", &resolved.tree256),
            ("repo256", &resolved.repo256),
        ] {
            if let Some(v) = value
                && !pin_is_hex(v)
            {
                return Err(format!(
                    "module {name}: resolved.{field} is not a sha256 hex string"
                ));
            }
        }
    }
    Ok(())
}

pub fn read(repo: &Path, host: &str) -> LockRead {
    let raw = match std::fs::read(path(repo, host)) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return LockRead::Missing,
        Err(e) => return LockRead::Corrupt(format!("io: {e}")),
    };
    match serde_json::from_slice::<Lockfile>(&raw) {
        Ok(lock) => match validate_pins(&lock) {
            Ok(()) => LockRead::Parsed(lock),
            Err(why) => LockRead::Corrupt(why),
        },
        Err(e) => LockRead::Corrupt(format!("invalid JSON: {e}")),
    }
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
                    tree256: None,
                    repo256: Some("cd".repeat(32)),
                    api_url: None,
                }),
            },
        );
        write(dir.path(), "laptop", &lock).unwrap();
        let LockRead::Parsed(read_back) = read(dir.path(), "laptop") else {
            panic!("expected a parsed lockfile");
        };
        assert_eq!(lock, read_back);
        assert!(matches!(read(dir.path(), "otherhost"), LockRead::Missing));
        // a corrupt lock is not a missing one — update would erase pins
        std::fs::write(dir.path().join("locks/laptop.lock"), b"{ truncated").unwrap();
        assert!(matches!(read(dir.path(), "laptop"), LockRead::Corrupt(_)));
        // non-hex pins are corrupt too, not silent wrong paths
        let mut bad = lock.clone();
        bad.modules
            .get_mut("helix")
            .unwrap()
            .resolved
            .as_mut()
            .unwrap()
            .tree256 = Some("ab".into());
        write(dir.path(), "badhost", &bad).unwrap();
        assert!(matches!(read(dir.path(), "badhost"), LockRead::Corrupt(_)));
    }
}
