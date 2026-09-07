//! Built-in transport modules. FetchContext owns dispatch, policy and budgets.

use std::io;

pub(crate) mod archive;
pub(crate) mod brew;
pub(crate) mod file;
pub(crate) mod git;
pub(crate) mod pixi;
pub(crate) mod plugin;
pub(crate) mod tarball;

pub use git::resolve_head as resolve_git_head;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("io: {0}")]
    Io(io::Error),
    #[error("sha256 mismatch for {url}: expected {expected}, got {actual}")]
    HashMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("http error fetching {url}: {reason}")]
    Http { url: String, reason: String },
    #[error("{what} exceeds the {limit} byte cap — refusing to truncate silently")]
    PayloadTooLarge { what: String, limit: u64 },
    #[error("payload exceeds the {limit} entry cap")]
    TooManyEntries { limit: usize },
    #[error("refusing archive entry {entry:?}: {reason}")]
    UnsafeArchive {
        entry: std::path::PathBuf,
        reason: &'static str,
    },
    #[error("zip extract: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("plugin reported {} error diagnostic(s)", .0.len())]
    Diagnostics(Vec<gripsack_ir::Diagnostic>),
    #[error("unsupported fetch: {0}")]
    Unsupported(String),
}

impl From<io::Error> for FetchError {
    fn from(error: io::Error) -> Self {
        if let Some(limit) = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<crate::spool::LimitExceeded>())
        {
            Self::PayloadTooLarge {
                what: limit.what.into(),
                limit: limit.limit,
            }
        } else {
            Self::Io(error)
        }
    }
}

/// Core-produced content identity plus transport-provided resolution metadata.
/// Metadata informs future acquisition; it never substitutes for the digest.
#[derive(Debug)]
pub struct FetchOutcome {
    pub identity: crate::FetchIdentity,
    pub url: Option<String>,
    pub version: Option<String>,
}
