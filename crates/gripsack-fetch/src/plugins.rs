//! Plugin provisioning (plan/0012 §move-2): `package = "owner/repo@tag"`
//! on a `[fetchers.x]` / `[linters.x]` entry downloads a release binary
//! into the store — declarative, pinned by the tag, sha256-verified,
//! receipted. grip manages the lifecycle; the plugin is a fetch.
//!
//! Layout (the krew/rootle model — versioned dirs, receipt last):
//!
//! ```text
//! $GRIPSACK_HOME/plugins/<exe>/<tag>/<exe>       versioned binaries
//! $GRIPSACK_HOME/plugins/<exe>/current -> <tag>/ pointer
//! $GRIPSACK_HOME/plugins/receipts/<exe>.toml     provenance
//! ```
//!
//! Release contract: the repo ships `<exe>-<tag>-<triple>.tar.gz` plus
//! a `<same>.sha256` sidecar asset (the 4-target matrix naming). A
//! missing checksum is a failed install, not a warning.

use super::FetchError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Parse `owner/repo[@tag]`. Returns (repo, tag).
pub fn parse_ref(package: &str) -> Option<(String, Option<String>)> {
    if !package.contains('/') {
        return None; // a wheel name — the caller's other path
    }
    let (repo, tag) = match package.split_once('@') {
        Some((r, t)) => (r.to_string(), Some(t.to_string())),
        None => (package.to_string(), None),
    };
    if repo.matches('/').count() != 1 {
        return None;
    }
    Some((repo, tag))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub source: String,
    pub tag: String,
    pub sha256: String,
}

pub struct PluginStore {
    home: PathBuf,
}

impl PluginStore {
    pub fn new(home: &Path) -> Self {
        PluginStore {
            home: home.to_path_buf(),
        }
    }

    fn exe_dir(&self, exe: &str) -> PathBuf {
        self.home.join("plugins").join(exe)
    }

    fn receipt_path(&self, exe: &str) -> PathBuf {
        self.home
            .join("plugins")
            .join("receipts")
            .join(format!("{exe}.toml"))
    }

    /// The binary the `current` pointer resolves to, if installed.
    pub fn current_binary(&self, exe: &str) -> Option<PathBuf> {
        let link = self.exe_dir(exe).join("current");
        let target = std::fs::read_link(&link).ok()?;
        let bin = link.parent()?.join(target).join(exe);
        bin.is_file().then_some(bin)
    }

    pub fn receipt(&self, exe: &str) -> Option<Receipt> {
        let text = std::fs::read_to_string(self.receipt_path(exe)).ok()?;
        toml::from_str(&text).ok()
    }

    /// Provision `<kind>-<name>` from `owner/repo[@tag]`: satisfied when
    /// the receipt already records this tag. Returns the binary path.
    pub fn ensure(&self, name: &str, package: &str, kind: &str) -> Result<PathBuf, FetchError> {
        let exe = format!("{kind}-{name}");
        let (repo, tag) = parse_ref(package).ok_or_else(|| FetchError::Http {
            url: package.to_string(),
            reason: format!(
                "package must be owner/repo[@tag] for plugin provisioning (got {package:?})"
            ),
        })?;
        if let Some(receipt) = self.receipt(&exe)
            && tag.as_deref() == Some(receipt.tag.as_str())
            && let Some(bin) = self.current_binary(&exe)
        {
            return Ok(bin); // satisfied — the receipt is the record
        }

        let release =
            crate::resolve::resolve_plugin_release(&repo, &exe, tag.as_deref()).map_err(|e| {
                FetchError::Http {
                    url: repo.clone(),
                    reason: e.to_string(),
                }
            })?;
        // the tag names the on-disk version dir and the `current`
        // symlink target — it arrives from a network response, so a
        // `a/../../evil` tag must not walk out of the plugin store
        if !safe_segment(&release.version) {
            return Err(FetchError::Http {
                url: repo.clone(),
                reason: format!(
                    "release tag {:?} is not a safe version directory name",
                    release.version
                ),
            });
        }

        let staging = self.home.join("plugins").join(format!(".staging-{exe}"));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;

        // download + verify BEFORE anything lands (checksum mandatory)
        let bytes =
            crate::fetch::tarball::read_url_authed(&release.url, release.api_url.as_deref())?;
        let actual = crate::fetch::archive::sha256(&bytes);
        if actual != release.sha256 {
            return Err(FetchError::HashMismatch {
                url: release.url.clone(),
                expected: release.sha256.clone(),
                actual,
            });
        }
        crate::fetch::archive::extract(&bytes, &staging, &exe)?;

        // the binary must be at the bundle root or under bin/
        let staged = if staging.join(&exe).is_file() {
            staging.join(&exe)
        } else if staging.join("bin").join(&exe).is_file() {
            staging.join("bin").join(&exe)
        } else {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(FetchError::Http {
                url: release.url.clone(),
                reason: format!("the bundle has no {exe} at its root or under bin/"),
            });
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }

        let version_dir = self.exe_dir(&exe).join(&release.version);
        let _ = std::fs::remove_dir_all(&version_dir);
        std::fs::create_dir_all(&version_dir)?;
        std::fs::rename(&staged, version_dir.join(&exe))?;
        let _ = std::fs::remove_dir_all(&staging);

        // receipt LAST — a failed step leaves no phantom install
        let receipt = Receipt {
            source: repo.clone(),
            tag: release.version.clone(),
            sha256: release.sha256.clone(),
        };
        let receipt_dir = self.receipt_path(&exe);
        std::fs::create_dir_all(receipt_dir.parent().expect("receipts dir"))?;
        gripsack_fs::atomic_write_at(
            &receipt_dir,
            toml::to_string(&receipt)
                .map_err(|e| FetchError::Http {
                    url: repo.clone(),
                    reason: e.to_string(),
                })?
                .as_bytes(),
        )?;
        // re-point current atomically (the store's symlink swap —
        // same primitive as the generation flip)
        let current = self.exe_dir(&exe).join("current");
        gripsack_fs::symlink_replace_at(&current, &self.exe_dir(&exe).join(&release.version))?;

        tracing::info!(
            plugin = exe,
            source = repo,
            tag = release.version,
            "provisioned plugin"
        );
        Ok(version_dir.join(&exe))
    }
}
/// A tag/version that names exactly one directory level: nonempty,
/// no separators, no `.`/`..`, no control characters. Network-derived
/// names must never widen into a path.
fn safe_segment(tag: &str) -> bool {
    !tag.is_empty()
        && !tag.starts_with('.')
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !tag.contains("..")
}

#[cfg(test)]
mod segment_tests {
    #[test]
    fn traversal_tags_are_refused() {
        assert!(super::safe_segment("v1.2.3"));
        assert!(super::safe_segment("release-2026-09-01"));
        for bad in [
            "a/../../evil",
            "..",
            ".",
            "v1/x",
            "",
            "a\\b",
            "ta\u{1b}g",
            ".hidden",
            "v..2",
        ] {
            assert!(!super::safe_segment(bad), "{bad:?} must be refused");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ref_shapes() {
        assert_eq!(
            parse_ref("acme/gripfetch-x@1.2.0"),
            Some(("acme/gripfetch-x".into(), Some("1.2.0".into())))
        );
        assert_eq!(
            parse_ref("acme/gripfetch-x"),
            Some(("acme/gripfetch-x".into(), None))
        );
        assert_eq!(parse_ref("griplint-yazi==1.2.0"), None); // a wheel name
        assert_eq!(parse_ref("too/many/slashes"), None);
    }

    #[test]
    fn receipt_roundtrips_and_current_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::new(dir.path());
        let exe = "gripfetch-demo";
        let bin = store.exe_dir(exe).join("v1").join(exe);
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();

        std::os::unix::fs::symlink("v1/", store.exe_dir(exe).join("current")).unwrap();
        assert_eq!(store.current_binary(exe), Some(bin));

        std::fs::create_dir_all(dir.path().join("plugins/receipts")).unwrap();
        std::fs::write(
            store.receipt_path(exe),
            "source = \"acme/gripfetch-demo\"\ntag = \"v1\"\nsha256 = \"ab\"\n",
        )
        .unwrap();
        let receipt = store.receipt(exe).unwrap();
        assert_eq!(receipt.tag, "v1");
    }
}
