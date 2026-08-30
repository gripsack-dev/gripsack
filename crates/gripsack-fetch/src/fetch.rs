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

pub use git::resolve_head as resolve_git_head;
pub(crate) mod pixi;
pub(crate) mod plugin;
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
    /// Error-severity diagnostics from a plugin (0009 §2 rule 1) —
    /// the CLI renders these through the same renderer as its own.
    #[error("plugin reported {} error diagnostic(s)", .0.len())]
    Diagnostics(Vec<gripsack_ir::Diagnostic>),
    #[error("unsupported fetch kind for v0.1: {0}")]
    Unsupported(String),
}

/// The payload's sha256 without staging anything — None for fetch kinds
/// resolved at fetch time (git trees, pixi envs, plugins).
pub fn payload_hash(spec: &FetchSpec) -> Result<Option<String>, FetchError> {
    match spec {
        FetchSpec::File { path } => file::payload_hash(path),
        // a pinned sha256 IS the identity — no download to decide
        // satisfaction (review finding F9: fully-pinned modules paid a
        // full payload download every apply)
        FetchSpec::Tarball {
            sha256: Some(sha), ..
        } => Ok(Some(sha.clone())),
        FetchSpec::Tarball { url, api_url, .. } => {
            tarball::payload_hash(&crate::resolve::expand_platform(url), api_url.as_deref())
        }
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
        FetchSpec::Tarball {
            url,
            sha256,
            api_url,
        } => tarball::fetch(
            &crate::resolve::expand_platform(url),
            sha256.as_deref(),
            api_url.as_deref(),
            dest,
        ),
        FetchSpec::Brew {
            formula, sha256, ..
        } => brew::fetch(formula, sha256.as_deref(), dest),
        FetchSpec::Pixi {
            package, version, ..
        } => pixi::fetch(package, version.as_deref(), dest),
        FetchSpec::Git { url, rev } => match rev {
            Some(rev) => git::fetch(url, rev, dest),
            // resolve fills the rev before dispatch (0016 §D2) — a
            // rev-less spec reaching fetch is a pipeline bug, say so
            None => Err(FetchError::Unsupported(
                "git float needs resolution first (git ls-remote) — this is a core bug".into(),
            )),
        },
        FetchSpec::GithubRelease { .. } => Err(FetchError::Unsupported(
            "github_release resolves to a tarball upstream of fetch (exec::resolve)".into(),
        )),
        FetchSpec::Plugin { name, args } => plugin::fetch(name, args, dest, None).map(|f| f.hash),
    }
}

/// Fetch with the module's lockfile pin, when one exists — plugin
/// fetchers receive it as `locked` in the request (0002 §4).
/// What a fetch yields: the payload identity (core-computed, always)
/// plus the plugin's reported pin for plugin fetches.
pub struct FetchOutcome {
    pub hash: String,
    pub plugin_url: Option<String>,
    pub plugin_version: Option<String>,
}

pub fn fetch_with_locked(
    spec: &FetchSpec,
    dest: &Path,
    locked: Option<&serde_json::Value>,
) -> Result<FetchOutcome, FetchError> {
    if let FetchSpec::Plugin { name, args } = spec {
        std::fs::create_dir_all(dest)?;
        return plugin::fetch(name, args, dest, locked).map(|f| FetchOutcome {
            hash: f.hash,
            plugin_url: f.url,
            plugin_version: f.version,
        });
    }
    fetch(spec, dest).map(|hash| FetchOutcome {
        hash,
        plugin_url: None,
        plugin_version: None,
    })
}
