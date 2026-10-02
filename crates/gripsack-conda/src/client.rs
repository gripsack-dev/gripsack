//! Typed client for one-shot, natively supervised `gripsack-conda` transactions.
//! Selected executable bytes, operator environment, deadlines and output bounds
//! belong to the common process boundary. Helper success remains advisory.

mod exchange;
#[cfg(test)]
mod tests;

use gripsack_process::{OperatorEnvironment, ProcessReceipt, SelectedProgram, Sha256Digest};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use gripsack_ir::workspace_v6::{
    identity::conda_closure_digest,
    lock::{LockedCondaEnvironment, LockedVirtualPackage},
};

use crate::protocol::{
    ArchiveRef, ErrorResponse, FrameError, ImportPixiRequest, MaterializeRequest, Request,
    ResolveRequest, Response,
};

/// Everything that can go wrong talking to the helper.
#[derive(Debug, thiserror::Error)]
pub enum CondaError {
    #[error("helper process admission or launch failed: {0}")]
    Native(#[source] io::Error),
    #[error("helper process did not complete successfully: {receipt:?}; {stderr}")]
    Process {
        receipt: Box<ProcessReceipt>,
        stderr: String,
    },
    /// A request argument could not be encoded for the wire (e.g. a non-UTF-8
    /// path). Distinct from [`CondaError::Echo`]: this is invalid local
    /// input, never a helper response that failed request binding.
    #[error("invalid request input: {0}")]
    InvalidInput(String),
    /// A frame carried malformed JSON, or frame-level I/O failed.
    #[error("malformed helper frame: {0}")]
    Frame(String),
    /// The response did not echo the request: stale `attempt`, mismatched
    /// `lock_digest`/`platform`/`final_prefix`, or an unexpected message
    /// kind. Native supervision has already completed before response admission.
    #[error("helper echo violation: {0}")]
    Echo(String),
    /// The helper reported a well-formed `error` response; `code` and
    /// `message` are preserved verbatim.
    #[error("helper error {code}: {message}")]
    Helper { code: String, message: String },
    /// A frame exceeded the protocol hard cap.
    #[error("helper frame of {actual} bytes exceeds the {limit}-byte cap")]
    Oversize { limit: u64, actual: u64 },
    /// The helper's stream ended mid-frame or mid-session.
    #[error("truncated helper stream")]
    Truncated,
}

impl From<FrameError> for CondaError {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Oversize { limit, actual } => CondaError::Oversize { limit, actual },
            FrameError::Truncated => CondaError::Truncated,
            FrameError::Malformed(source) => CondaError::Frame(source.to_string()),
            FrameError::Io(source) => CondaError::Frame(source.to_string()),
            FrameError::Version(version) => {
                CondaError::Frame(format!("unsupported helper protocol {version}"))
            }
            FrameError::EncodingLimit { limit } => {
                CondaError::InvalidInput(format!("frame exceeds {limit} bytes"))
            }
            FrameError::TrailingBytes => {
                CondaError::Frame("bytes follow the single response frame".into())
            }
        }
    }
}

/// Immutable helper selection reused across separately supervised transactions.
pub struct CondaHelper {
    program: SelectedProgram,
    environment: OperatorEnvironment,
    directory: PathBuf,
    deadline: Instant,
}

impl CondaHelper {
    /// Select exact helper bytes without starting a child. Every operation gets
    /// a fresh process, one versioned frame, closed stdin and mandatory cleanup.
    pub fn select(
        path: &Path,
        expected_sha256: Option<Sha256Digest>,
        directory: &Path,
        deadline: Instant,
    ) -> Result<Self, CondaError> {
        if !directory.is_absolute() {
            return Err(CondaError::InvalidInput(
                "helper working directory must be absolute".into(),
            ));
        }
        let environment = OperatorEnvironment::capture().map_err(CondaError::Native)?;
        let program = SelectedProgram::select(&environment, path, expected_sha256, deadline)
            .map_err(CondaError::Native)?;
        Ok(Self {
            program,
            environment,
            directory: directory.to_path_buf(),
            deadline,
        })
    }

    /// `resolve`: solve a closure for `platform` from `channels`, grounded in
    /// the core-measured `virtual_packages`. The returned environment is
    /// advisory data; the core re-validates it.
    pub fn resolve(
        &self,
        attempt: u64,
        channels: &[String],
        packages: &BTreeMap<String, String>,
        platform: &str,
        virtual_packages: &[LockedVirtualPackage],
    ) -> Result<LockedCondaEnvironment, CondaError> {
        let request = Request::Resolve(ResolveRequest {
            attempt,
            channels: channels.to_vec(),
            packages: packages.clone(),
            platform: platform.to_string(),
            virtual_packages: virtual_packages.to_vec(),
        });
        match self.exchange(&request)? {
            Response::Resolved(response) => {
                // The exchange admits operation, live attempt and platform.
                Ok(response.environment)
            }
            Response::Error(error) => Err(self.helper_error(attempt, error)?),
            _ => Err(CondaError::Echo("unexpected response to resolve".into())),
        }
    }

    /// `import_pixi`: import a captured pixi manifest + lock pair. Advisory;
    /// the core re-validates.
    pub fn import_pixi(
        &self,
        attempt: u64,
        manifest: &str,
        lock: &str,
        environment: &str,
        platform: &str,
    ) -> Result<LockedCondaEnvironment, CondaError> {
        let request = Request::ImportPixi(ImportPixiRequest {
            attempt,
            manifest_bytes: manifest.to_string(),
            lock_bytes: lock.to_string(),
            environment: environment.to_string(),
            platform: platform.to_string(),
        });
        match self.exchange(&request)? {
            Response::Imported(response) => {
                // The exchange admits operation, live attempt and platform.
                Ok(response.environment)
            }
            Response::Error(error) => Err(self.helper_error(attempt, error)?),
            _ => Err(CondaError::Echo(
                "unexpected response to import_pixi".into(),
            )),
        }
    }

    /// `materialize`: install the locked closure into `staging_dir`, patched
    /// for `final_prefix`. Advisory; the core re-validates the tree. The
    /// echoes of `lock_digest`, `platform`, and `final_prefix` are validated
    /// against the request. Returns one `(package name, conda-meta relative
    /// receipt path)` per installed package.
    pub fn materialize(
        &self,
        attempt: u64,
        closure: &LockedCondaEnvironment,
        final_prefix: &Path,
        staging_dir: &Path,
        archives: &[(String, PathBuf)],
    ) -> Result<Vec<(String, String)>, CondaError> {
        let request = Request::Materialize(MaterializeRequest {
            attempt,
            lock_digest: conda_closure_digest(closure).to_string(),
            closure: std::borrow::Cow::Borrowed(closure),
            final_prefix: utf8_path(final_prefix, "final_prefix")?,
            staging_dir: utf8_path(staging_dir, "staging_dir")?,
            archives: archives
                .iter()
                .map(|(sha256, path)| {
                    Ok(ArchiveRef {
                        sha256: sha256.clone(),
                        path: utf8_path(path, "archive path")?,
                    })
                })
                .collect::<Result<Vec<_>, CondaError>>()?,
        });
        match self.exchange(&request)? {
            Response::Materialized(response) => Ok(response
                .packages
                .into_iter()
                .map(|package| (package.name, package.conda_meta))
                .collect()),
            Response::Error(error) => Err(self.helper_error(attempt, error)?),
            _ => Err(CondaError::Echo(
                "unexpected response to materialize".into(),
            )),
        }
    }

    /// Validates an `error` response's attempt echo and converts it into a
    /// typed [`CondaError::Helper`] with `code`/`message` preserved verbatim.
    fn helper_error(&self, sent: u64, error: ErrorResponse) -> Result<CondaError, CondaError> {
        if let Some(echoed) = error.attempt {
            if echoed != sent {
                return Err(CondaError::Echo(format!(
                    "stale attempt echo in error response: sent {sent}, got {echoed}"
                )));
            }
        }
        Ok(CondaError::Helper {
            code: error.code,
            message: error.message,
        })
    }
}

/// The wire format carries paths as UTF-8 strings; a non-UTF-8 path is
/// rejected client-side rather than lossily corrupted.
fn utf8_path(path: &Path, what: &str) -> Result<String, CondaError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| CondaError::InvalidInput(format!("{what} is not UTF-8: {}", path.display())))
}
