//! Fetch dispatcher (0002 §5): one module per fetch kind. `fetch`
//! materializes a payload and returns its sha256; `payload_hash`
//! resolves identity without staging when the kind allows it.

use gripsack_ir::FetchSpec;
use std::io;
use std::path::Path;

pub(crate) mod archive;
pub(crate) mod brew;
pub(crate) mod file;
pub(crate) mod git;
pub(crate) mod pixi;
pub(crate) mod tarball;

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
    #[error("zip extract: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("unsupported fetch kind for v0.1: {0}")]
    Unsupported(String),
}

/// The payload's sha256 without staging anything — None for fetch kinds
/// resolved at fetch time (git trees, pixi envs, plugins).
pub fn payload_hash(spec: &FetchSpec) -> Result<Option<String>, FetchError> {
    match spec {
        FetchSpec::File { path } => file::payload_hash(path),
        FetchSpec::Tarball { url, .. } => tarball::payload_hash(url),
        FetchSpec::Brew { formula, .. } => brew::payload_hash(formula),
        _ => Ok(None),
    }
}

/// Fetch a spec into `dest`, returning the payload hash. Pinned hashes
/// are verified before anything is staged — a mismatch is a hard
/// failure, never retried (0007 §retries).
pub fn fetch(spec: &FetchSpec, dest: &Path) -> Result<String, FetchError> {
    std::fs::create_dir_all(dest)?;
    match spec {
        FetchSpec::File { path } => file::fetch(path, dest),
        FetchSpec::Tarball { url, sha256 } => tarball::fetch(url, sha256.as_deref(), dest),
        FetchSpec::Brew { formula, sha256 } => brew::fetch(formula, sha256.as_deref(), dest),
        FetchSpec::Pixi {
            package, version, ..
        } => pixi::fetch(package, version.as_deref(), dest),
        FetchSpec::Git { url, rev } => git::fetch(url, rev, dest),
        FetchSpec::GithubRelease { .. } => Err(FetchError::Unsupported(
            "github_release resolves to a tarball upstream of fetch (exec::resolve)".into(),
        )),
        FetchSpec::Plugin { name, .. } => Err(FetchError::Unsupported(format!(
            "plugin gripfetch-{name} (protocol host lands with the scheduler)"
        ))),
    }
}
