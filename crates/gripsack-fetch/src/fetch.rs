//! Built-in fetch transports (0002 §2 rung 1). v0.1: `file` and
//! `tarball` (file:// and https). Everything lands in a staging dir the
//! caller publishes into the store; a pinned `sha256` is verified
//! before anything is staged — hash mismatch is a hard failure, never
//! retried (0007 §retries).

use flate2::read::GzDecoder;
use gripsack_ir::FetchSpec;
use sha2::{Digest, Sha256};
use std::io;
use std::io::Read as _;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("sha256 mismatch for {url}: expected {expected}, got {actual}")]
    HashMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("http error fetching {url}: {reason}")]
    Http { url: String, reason: String },
    #[error("unsupported fetch kind for v0.1: {0}")]
    Unsupported(String),
}

/// The payload's sha256 without staging anything — None for fetch kinds
/// whose resolution lands in 0.2 (git, github_release, plugin).
pub fn payload_hash(spec: &FetchSpec) -> Result<Option<String>, FetchError> {
    match spec {
        FetchSpec::File { path } => {
            let path = Path::new(path);
            if path.is_dir() {
                Ok(Some(gripsack_store::canonical_tree_hash(path)?))
            } else {
                Ok(Some(hex(&Sha256::digest(std::fs::read(path)?))))
            }
        }
        FetchSpec::Tarball { url, .. } => Ok(Some(hex(&Sha256::digest(read_url(url)?)))),
        _ => Ok(None),
    }
}

/// Fetch a spec into `dest` (created by the caller or here). Returns
/// the payload's sha256 — the lockfile pins it (0008 §5).
pub fn fetch(spec: &FetchSpec, dest: &Path) -> Result<String, FetchError> {
    std::fs::create_dir_all(dest)?;
    match spec {
        FetchSpec::File { path } => stage_local(Path::new(path), dest),
        FetchSpec::Tarball { url, sha256 } => {
            let bytes = read_url(url)?;
            let actual = hex(&Sha256::digest(&bytes));
            if let Some(expected) = sha256
                && actual != *expected
            {
                return Err(FetchError::HashMismatch {
                    url: url.clone(),
                    expected: expected.clone(),
                    actual,
                });
            }
            extract_tarball(&bytes, dest)?;
            Ok(actual)
        }
        FetchSpec::Git { .. } => Err(FetchError::Unsupported("git (0.2)".into())),
        FetchSpec::GithubRelease { .. } => {
            Err(FetchError::Unsupported("github_release (0.2)".into()))
        }
        FetchSpec::Plugin { name, .. } => Err(FetchError::Unsupported(format!(
            "plugin gripfetch-{name} (protocol host lands with the scheduler)"
        ))),
    }
}

/// file: a tarball path or a plain directory of payload files.
fn stage_local(path: &Path, dest: &Path) -> Result<String, FetchError> {
    if path.is_dir() {
        copy_tree(path, dest).map_err(FetchError::Io)?;
        Ok(gripsack_store::canonical_tree_hash(path)?)
    } else {
        let bytes = std::fs::read(path)?;
        let hash = hex(&Sha256::digest(&bytes));
        extract_tarball(&bytes, dest)?;
        Ok(hash)
    }
}

fn read_url(url: &str) -> Result<Vec<u8>, FetchError> {
    if let Some(path) = url.strip_prefix("file://") {
        return Ok(std::fs::read(path)?);
    }
    let response = ureq::get(url).call().map_err(|e| FetchError::Http {
        url: url.to_string(),
        reason: e.to_string(),
    })?;
    let reader = response.into_reader();
    let mut reader = io::Read::take(reader, 512 * 1024 * 1024);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::Http {
            url: url.to_string(),
            reason: e.to_string(),
        })?;
    Ok(bytes)
}

fn extract_tarball(bytes: &[u8], dest: &Path) -> Result<(), FetchError> {
    let tar: Box<dyn io::Read> = if bytes.starts_with(&[0x1f, 0x8b]) {
        Box::new(GzDecoder::new(bytes))
    } else {
        Box::new(bytes)
    };
    let mut archive = tar::Archive::new(tar);
    archive.unpack(dest)?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_tarball(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        let content = b"#!/bin/sh\necho hello\n";
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "bin/hello", &content[..])
            .unwrap();
        builder
            .into_inner()
            .unwrap()
            .finish()
            .unwrap()
            .flush()
            .unwrap();
    }

    #[test]
    fn file_fetch_extracts_tarball() {
        let dir = tempfile::tempdir().unwrap();
        let tar = dir.path().join("hello.tar.gz");
        make_tarball(&tar);
        let dest = dir.path().join("out");
        fetch(
            &FetchSpec::File {
                path: tar.to_string_lossy().into_owned(),
            },
            &dest,
        )
        .unwrap();
        assert!(dest.join("bin/hello").exists());
    }

    #[test]
    fn tarball_file_url_verifies_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let tar = dir.path().join("hello.tar.gz");
        make_tarball(&tar);
        let url = format!("file://{}", tar.display());
        let good = hex(&Sha256::digest(std::fs::read(&tar).unwrap()));
        let dest = dir.path().join("out");
        fetch(
            &FetchSpec::Tarball {
                url: url.clone(),
                sha256: Some(good),
            },
            &dest,
        )
        .unwrap();
        assert!(dest.join("bin/hello").exists());

        let err = fetch(
            &FetchSpec::Tarball {
                url,
                sha256: Some("0".repeat(64)),
            },
            &dir.path().join("out2"),
        )
        .unwrap_err();
        assert!(matches!(err, FetchError::HashMismatch { .. }));
    }
}
