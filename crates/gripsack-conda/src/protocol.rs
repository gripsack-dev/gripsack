//! Conda helper wire protocol v1.
//!
//! Length-prefixed JSON frames over stdio: an 8-byte little-endian `u64`
//! length followed by that many bytes of UTF-8 JSON. Every message type uses
//! `#[serde(deny_unknown_fields)]`; unknown fields are a hard error, never a
//! default.
//!
//! Handshake: the client sends `{"kind":"hello","protocol":1}`; the helper
//! replies with the identical frame or the client aborts.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{self, Read, Write};

use gripsack_ir::workspace_v6::lock::{LockedCondaEnvironment, LockedVirtualPackage};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// The only protocol version this crate speaks.
pub const PROTOCOL_VERSION: u64 = 1;

/// Hard cap on one request frame: 32 MiB, i.e. 2× the portable lock cap
/// (`MAX_WORKSPACE_LOCK_BYTES` = 16 MiB in
/// `crates/gripsack-exec/src/workspace/pins.rs`). Every request payload is
/// lock-scale: package maps, archive path lists derived from the lock, and
/// pixi manifest+lock captured-input bytes bounded by the same repository
/// capture budget. An oversize frame is a hard error, never a default.
pub const MAX_REQUEST_BYTES: u64 = 32 * 1024 * 1024;

/// Hard cap on one response frame: 32 MiB, i.e. 2× the portable lock cap
/// (`MAX_WORKSPACE_LOCK_BYTES` = 16 MiB in
/// `crates/gripsack-exec/src/workspace/pins.rs`). A resolve response is
/// exactly one lock-scale closure; imported/materialized responses are no
/// larger. An oversize frame is a hard error, never a default.
pub const MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;

/// A frame could not be read or written. Reading distinguishes a clean EOF at
/// a frame boundary (`Ok(None)` from [`read_frame`]) from a truncated stream,
/// which is always a hard error.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("frame of {actual} bytes exceeds the {limit}-byte cap")]
    Oversize { limit: u64, actual: u64 },
    #[error("truncated frame: stream ended mid-frame")]
    Truncated,
    #[error("malformed frame payload: {0}")]
    Malformed(#[source] serde_json::Error),
}

/// Handshake frame: `{"kind":"hello","protocol":1}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub kind: String,
    pub protocol: u64,
}

impl Hello {
    pub fn new() -> Self {
        Self {
            kind: "hello".to_string(),
            protocol: PROTOCOL_VERSION,
        }
    }

    /// True only for the exact v1 handshake frame.
    pub fn is_v1(&self) -> bool {
        self.kind == "hello" && self.protocol == PROTOCOL_VERSION
    }
}

impl Default for Hello {
    fn default() -> Self {
        Self::new()
    }
}

/// `resolve` request: solve a conda closure from channels and host facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    pub attempt: u64,
    pub channels: Vec<String>,
    pub packages: BTreeMap<String, String>,
    /// Conda subdir, e.g. "linux-64".
    pub platform: String,
    /// Host facts measured by the core.
    pub virtual_packages: Vec<LockedVirtualPackage>,
}

/// `import_pixi` request: import a captured pixi manifest + lock pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPixiRequest {
    pub attempt: u64,
    /// UTF-8 pixi manifest bytes.
    pub manifest_bytes: String,
    /// UTF-8 pixi lock bytes.
    pub lock_bytes: String,
    pub environment: String,
    pub platform: String,
}

/// One retained original archive plus its expected sha256 (64 lowercase hex).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRef {
    pub sha256: String,
    pub path: String,
}

/// `materialize` request: install a locked closure from retained archives.
/// Frozen materialization never solves and does no network I/O.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializeRequest<'a> {
    pub attempt: u64,
    /// 64-hex closure digest of the lock this materialization serves.
    pub lock_digest: String,
    /// Complete frozen records, including repodata patches and install policy.
    pub closure: Cow<'a, LockedCondaEnvironment>,
    /// Absolute path the installed bytes must be patched for.
    pub final_prefix: String,
    /// Absolute path, exists and empty; the tree is written here.
    pub staging_dir: String,
    pub archives: Vec<ArchiveRef>,
}

/// Every request the helper accepts, internally tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request<'a> {
    Resolve(ResolveRequest),
    ImportPixi(ImportPixiRequest),
    Materialize(MaterializeRequest<'a>),
}

impl Request<'_> {
    pub fn attempt(&self) -> u64 {
        match self {
            Request::Resolve(r) => r.attempt,
            Request::ImportPixi(r) => r.attempt,
            Request::Materialize(r) => r.attempt,
        }
    }
}

/// `resolved` response: the solved, normalized closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedResponse {
    pub attempt: u64,
    pub environment: LockedCondaEnvironment,
}

/// `imported` response: the imported, normalized closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedResponse {
    pub attempt: u64,
    pub environment: LockedCondaEnvironment,
}

/// One written receipt: `conda_meta` is the receipt path relative to the
/// staging directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedPackage {
    pub name: String,
    pub conda_meta: String,
}

/// `materialized` response. Advisory only: the core re-validates the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedResponse {
    pub attempt: u64,
    pub lock_digest: String,
    pub platform: String,
    pub final_prefix: String,
    pub packages: Vec<MaterializedPackage>,
}

/// `error` response for a failed request (`attempt` echoes the request when
/// the failure is attributable to one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub attempt: Option<u64>,
    pub code: String,
    pub message: String,
}

/// Every response the helper can send, internally tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Resolved(ResolvedResponse),
    Imported(ImportedResponse),
    Materialized(MaterializedResponse),
    Error(ErrorResponse),
}

/// Reads exactly `buf.len()` bytes. Returns `Ok(false)` only when the stream
/// ends cleanly *before the first byte*; EOF anywhere later is a hard
/// truncation error.
fn read_header(reader: &mut impl Read, buf: &mut [u8]) -> Result<bool, FrameError> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FrameError::Io(e)),
        }
    }
    Ok(true)
}

/// Writes `value` as one length-prefixed JSON frame, enforcing `max` as a hard
/// cap (oversize payloads are rejected, never silently split or truncated).
pub fn write_frame<W: Write, T: Serialize>(
    writer: &mut W,
    value: &T,
    max: u64,
) -> Result<(), FrameError> {
    let body = serde_json::to_vec(value).map_err(FrameError::Malformed)?;
    if body.len() as u64 > max {
        return Err(FrameError::Oversize {
            limit: max,
            actual: body.len() as u64,
        });
    }
    writer.write_all(&(body.len() as u64).to_le_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

/// Reads one length-prefixed JSON frame, enforcing `max` as a hard cap.
/// Returns `Ok(None)` on a clean EOF at a frame boundary; oversize, truncated,
/// and garbage frames are hard errors.
pub fn read_frame<R: Read, T: DeserializeOwned>(
    reader: &mut R,
    max: u64,
) -> Result<Option<T>, FrameError> {
    let mut len = [0u8; 8];
    if !read_header(reader, &mut len)? {
        return Ok(None);
    }
    let n = u64::from_le_bytes(len);
    if n > max {
        return Err(FrameError::Oversize {
            limit: max,
            actual: n,
        });
    }
    let mut body = vec![0u8; n as usize];
    reader.read_exact(&mut body).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            FrameError::Truncated
        } else {
            FrameError::Io(e)
        }
    })?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(FrameError::Malformed)
}

/// Client → helper: a [`Request`] under [`MAX_REQUEST_BYTES`].
pub fn write_request_frame<W: Write>(writer: &mut W, request: &Request<'_>) -> Result<(), FrameError> {
    write_frame(writer, request, MAX_REQUEST_BYTES)
}

/// Helper-side read of a [`Request`] under [`MAX_REQUEST_BYTES`].
pub fn read_request_frame<R: Read>(reader: &mut R) -> Result<Option<Request<'static>>, FrameError> {
    read_frame(reader, MAX_REQUEST_BYTES)
}

/// Helper → client: a [`Response`] under [`MAX_RESPONSE_BYTES`].
pub fn write_response_frame<W: Write>(
    writer: &mut W,
    response: &Response,
) -> Result<(), FrameError> {
    write_frame(writer, response, MAX_RESPONSE_BYTES)
}

/// Client-side read of a [`Response`] under [`MAX_RESPONSE_BYTES`].
pub fn read_response_frame<R: Read>(reader: &mut R) -> Result<Option<Response>, FrameError> {
    read_frame(reader, MAX_RESPONSE_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn unknown_fields_are_rejected() {
        let raw = br#"{"kind":"resolve","attempt":1,"channels":[],"packages":{},"platform":"linux-64","virtual_packages":[],"bogus":1}"#;
        let mut frame = Vec::new();
        frame.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        frame.extend_from_slice(raw);
        assert!(matches!(
            read_request_frame(&mut &frame[..]),
            Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn oversize_frame_is_rejected() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&(MAX_REQUEST_BYTES + 1).to_le_bytes());
        assert!(matches!(
            read_request_frame(&mut &frame[..]),
            Err(FrameError::Oversize { .. })
        ));
    }

    #[test]
    fn truncated_frame_is_rejected() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&100u64.to_le_bytes());
        frame.extend_from_slice(b"{}");
        assert!(matches!(
            read_request_frame(&mut &frame[..]),
            Err(FrameError::Truncated)
        ));
    }

    #[test]
    fn clean_eof_is_none() {
        let empty: &[u8] = &[];
        assert!(read_request_frame(&mut &empty[..]).unwrap().is_none());
    }

    #[test]
    fn partial_header_is_truncated() {
        let partial: &[u8] = &[1, 2, 3];
        assert!(matches!(
            read_request_frame(&mut &partial[..]),
            Err(FrameError::Truncated)
        ));
    }
}
