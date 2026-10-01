//! Strict stdio client for the `gripsack-conda` helper process.
//!
//! The client owns the helper's stdin/stdout, performs the v1 handshake, and
//! validates every response against the request that provoked it. A success
//! claim from the helper is just data — the core re-validates the returned
//! closure and any materialized tree independently.

use std::collections::BTreeMap;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use gripsack_ir::workspace_v6::{
    identity::conda_closure_digest,
    lock::{LockedCondaEnvironment, LockedVirtualPackage},
};

use crate::protocol::{
    self, ArchiveRef, ErrorResponse, FrameError, Hello, ImportPixiRequest, MaterializeRequest,
    Request, ResolveRequest, Response,
};

/// Everything that can go wrong talking to the helper.
#[derive(Debug, thiserror::Error)]
pub enum CondaError {
    /// The helper process could not be spawned.
    #[error("failed to spawn helper: {0}")]
    Spawn(#[source] io::Error),
    /// The helper did not answer the v1 `hello` handshake identically.
    #[error("helper handshake failed: {0}")]
    Handshake(String),
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
    /// kind. The child is killed before this error is returned.
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
        }
    }
}

/// A spawned `gripsack-conda` helper with a completed v1 handshake.
///
/// Dropping the client kills and reaps the child.
pub struct CondaHelper {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
}

impl CondaHelper {
    /// Spawns the helper at `path` and performs the hello handshake. Any
    /// deviation kills the child and fails.
    pub fn spawn(path: &Path) -> Result<CondaHelper, CondaError> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(CondaError::Spawn)?;
        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout was piped"));
        let mut this = CondaHelper {
            child: Some(child),
            stdin: Some(stdin),
            stdout: Some(stdout),
        };
        if let Err(error) = this.handshake() {
            this.kill_quietly();
            return Err(error);
        }
        Ok(this)
    }

    fn handshake(&mut self) -> Result<(), CondaError> {
        let hello = Hello::new();
        protocol::write_frame(self.stdin_mut(), &hello, protocol::MAX_REQUEST_BYTES)
            .map_err(|error| CondaError::Handshake(error.to_string()))?;
        match protocol::read_frame::<_, Hello>(self.stdout_mut(), protocol::MAX_RESPONSE_BYTES) {
            Ok(Some(reply)) if reply == hello => Ok(()),
            Ok(Some(reply)) => Err(CondaError::Handshake(format!(
                "expected {hello:?}, got {reply:?}"
            ))),
            Ok(None) => Err(CondaError::Handshake(
                "helper closed stdout before the handshake reply".into(),
            )),
            Err(error) => Err(CondaError::Handshake(error.to_string())),
        }
    }

    /// `resolve`: solve a closure for `platform` from `channels`, grounded in
    /// the core-measured `virtual_packages`. The returned environment is
    /// advisory data; the core re-validates it.
    pub fn resolve(
        &mut self,
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
                self.require_attempt(attempt, response.attempt)?;
                Ok(response.environment)
            }
            Response::Error(error) => Err(self.helper_error(attempt, error)?),
            other => Err(self.violation(format!(
                "resolve request answered with unexpected kind: {other:?}"
            ))),
        }
    }

    /// `import_pixi`: import a captured pixi manifest + lock pair. Advisory;
    /// the core re-validates.
    pub fn import_pixi(
        &mut self,
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
                self.require_attempt(attempt, response.attempt)?;
                Ok(response.environment)
            }
            Response::Error(error) => Err(self.helper_error(attempt, error)?),
            other => Err(self.violation(format!(
                "import_pixi request answered with unexpected kind: {other:?}"
            ))),
        }
    }

    /// `materialize`: install the locked closure into `staging_dir`, patched
    /// for `final_prefix`. Advisory; the core re-validates the tree. The
    /// echoes of `lock_digest`, `platform`, and `final_prefix` are validated
    /// against the request. Returns one `(package name, conda-meta relative
    /// receipt path)` per installed package.
    pub fn materialize(
        &mut self,
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
            Response::Materialized(response) => {
                self.require_attempt(attempt, response.attempt)?;
                let Request::Materialize(sent) = &request else {
                    unreachable!("request was constructed above");
                };
                if response.lock_digest != sent.lock_digest
                    || response.platform != sent.closure.platform
                    || response.final_prefix != sent.final_prefix
                {
                    return Err(self.violation(format!(
                        "materialized echo mismatch: sent lock_digest={} platform={} final_prefix={}, got lock_digest={} platform={} final_prefix={}",
                        sent.lock_digest,
                        sent.closure.platform,
                        sent.final_prefix,
                        response.lock_digest,
                        response.platform,
                        response.final_prefix
                    )));
                }
                Ok(response
                    .packages
                    .into_iter()
                    .map(|package| (package.name, package.conda_meta))
                    .collect())
            }
            Response::Error(error) => Err(self.helper_error(attempt, error)?),
            other => Err(self.violation(format!(
                "materialize request answered with unexpected kind: {other:?}"
            ))),
        }
    }

    fn stdin_mut(&mut self) -> &mut ChildStdin {
        self.stdin.as_mut().expect("stdin present until drop")
    }

    fn stdout_mut(&mut self) -> &mut BufReader<ChildStdout> {
        self.stdout.as_mut().expect("stdout present until drop")
    }

    /// Sends one request and reads one response. Any frame-level failure
    /// (malformed, oversize, truncated, EOF mid-session) kills the child.
    fn exchange(&mut self, request: &Request) -> Result<Response, CondaError> {
        if let Err(error) = protocol::write_request_frame(self.stdin_mut(), request) {
            self.kill_quietly();
            return Err(error.into());
        }
        match protocol::read_response_frame(self.stdout_mut()) {
            Ok(Some(response)) => Ok(response),
            Ok(None) => {
                self.kill_quietly();
                Err(CondaError::Truncated)
            }
            Err(error) => {
                self.kill_quietly();
                Err(error.into())
            }
        }
    }

    /// A non-error response must echo the request's attempt exactly.
    fn require_attempt(&mut self, sent: u64, echoed: u64) -> Result<(), CondaError> {
        if echoed != sent {
            return Err(self.violation(format!(
                "stale attempt echo: sent {sent}, got {echoed}"
            )));
        }
        Ok(())
    }

    /// Validates an `error` response's attempt echo and converts it into a
    /// typed [`CondaError::Helper`] with `code`/`message` preserved verbatim.
    /// A mismatched echo is a protocol violation (the child is killed); a
    /// matching or absent echo is a legitimate helper error and the child
    /// stays alive.
    fn helper_error(&mut self, sent: u64, error: ErrorResponse) -> Result<CondaError, CondaError> {
        if let Some(echoed) = error.attempt {
            if echoed != sent {
                return Err(self.violation(format!(
                    "stale attempt echo in error response: sent {sent}, got {echoed}"
                )));
            }
        }
        Ok(CondaError::Helper {
            code: error.code,
            message: error.message,
        })
    }

    /// Records an echo violation: the child is killed before the error is
    /// returned.
    fn violation(&mut self, message: String) -> CondaError {
        self.kill_quietly();
        CondaError::Echo(message)
    }

    fn kill_quietly(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The wire format carries paths as UTF-8 strings; a non-UTF-8 path is
/// rejected client-side rather than lossily corrupted.
fn utf8_path(path: &Path, what: &str) -> Result<String, CondaError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| CondaError::InvalidInput(format!("{what} is not UTF-8: {}", path.display())))
}

impl Drop for CondaHelper {
    fn drop(&mut self) {
        self.kill_quietly();
    }
}
