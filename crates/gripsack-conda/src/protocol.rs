//! Conda helper wire protocol v3: one bounded transaction per process.
//!
//! Length-prefixed JSON frames over stdio: an 8-byte little-endian `u64`
//! length followed by that many bytes of UTF-8 JSON. Every message type uses
//! `#[serde(deny_unknown_fields)]`; unknown fields are a hard error, never a
//! default.
//!
//! Each frame carries `{protocol, payload}`. The helper admits one complete
//! request and stdin EOF before effects, then writes exactly one response.
//! There is no persistent child or duplex handshake outside native supervision.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{self, Read, Write};

use gripsack_ir::workspace_model::lock::{
    LockedCondaEnvironment, LockedCondaSystemRequirements, LockedVirtualPackage,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// The only protocol version this crate speaks.
pub const PROTOCOL_VERSION: u64 = 3;
pub const FRAME_HEADER_BYTES: usize = std::mem::size_of::<u64>();

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
    #[error("unsupported helper protocol version {0}")]
    Version(u64),
    #[error("frame serialization exceeds the {limit}-byte cap")]
    EncodingLimit { limit: u64 },
    #[error("bytes follow the single response frame")]
    TrailingBytes,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Message<T> {
    protocol: u64,
    payload: T,
}

impl<T> Message<T> {
    fn admit(self) -> Result<T, FrameError> {
        if self.protocol != PROTOCOL_VERSION {
            return Err(FrameError::Version(self.protocol));
        }
        Ok(self.payload)
    }
}

/// `resolve` request: solve from channels and baseline-adjusted virtual facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    pub attempt: u64,
    pub channels: Vec<String>,
    pub packages: BTreeMap<String, String>,
    /// Conda subdir, e.g. "linux-64".
    pub platform: String,
    /// Core-measured facts, with explicitly declared floors substituted.
    pub virtual_packages: Vec<LockedVirtualPackage>,
    /// Exact declared floors; required even when empty.
    pub system_requirements: LockedCondaSystemRequirements,
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
    let frame = encode_frame(value, max)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

fn encode_frame(value: &impl Serialize, max: u64) -> Result<Vec<u8>, FrameError> {
    let limit = usize::try_from(max)
        .ok()
        .and_then(|limit| limit.checked_add(FRAME_HEADER_BYTES))
        .ok_or(FrameError::EncodingLimit { limit: max })?;
    let mut frame =
        gripsack_process::InputBuffer::new(gripsack_process::InputByteLimit::new(limit));
    frame.write_all(&[0; FRAME_HEADER_BYTES])?;
    serde_json::to_writer(&mut frame, value).map_err(|error| {
        if error.io_error_kind() == Some(io::ErrorKind::InvalidInput) {
            FrameError::EncodingLimit { limit: max }
        } else {
            FrameError::Malformed(error)
        }
    })?;
    let mut frame = frame.into_bytes();
    let length = (frame.len() - FRAME_HEADER_BYTES) as u64;
    frame[..FRAME_HEADER_BYTES].copy_from_slice(&length.to_le_bytes());
    Ok(frame)
}

/// Encode directly into the supervised input buffer, without a second payload copy.
pub fn encode_request(request: &Request<'_>) -> Result<Vec<u8>, FrameError> {
    encode_frame(
        &Message {
            protocol: PROTOCOL_VERSION,
            payload: request,
        },
        MAX_REQUEST_BYTES,
    )
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
pub fn write_request_frame<W: Write>(
    writer: &mut W,
    request: &Request<'_>,
) -> Result<(), FrameError> {
    write_frame(
        writer,
        &Message {
            protocol: PROTOCOL_VERSION,
            payload: request,
        },
        MAX_REQUEST_BYTES,
    )
}

/// Helper-side read of a [`Request`] under [`MAX_REQUEST_BYTES`].
pub fn read_request_frame<R: Read>(reader: &mut R) -> Result<Option<Request<'static>>, FrameError> {
    read_frame::<_, Message<Request<'static>>>(reader, MAX_REQUEST_BYTES)?
        .map(Message::admit)
        .transpose()
}

/// Helper → client: a [`Response`] under [`MAX_RESPONSE_BYTES`].
pub fn write_response_frame<W: Write>(
    writer: &mut W,
    response: &Response,
) -> Result<(), FrameError> {
    write_frame(
        writer,
        &Message {
            protocol: PROTOCOL_VERSION,
            payload: response,
        },
        MAX_RESPONSE_BYTES,
    )
}

/// Client-side read of a [`Response`] under [`MAX_RESPONSE_BYTES`].
pub fn read_response_frame<R: Read>(reader: &mut R) -> Result<Option<Response>, FrameError> {
    read_frame::<_, Message<Response>>(reader, MAX_RESPONSE_BYTES)?
        .map(Message::admit)
        .transpose()
}

/// Decode one completed native output stream without copying its frame body.
pub fn decode_response_frame(frame: &[u8]) -> Result<Response, FrameError> {
    let header = frame
        .get(..FRAME_HEADER_BYTES)
        .ok_or(FrameError::Truncated)?;
    let length = u64::from_le_bytes(header.try_into().map_err(|_| FrameError::Truncated)?);
    if length > MAX_RESPONSE_BYTES {
        return Err(FrameError::Oversize {
            limit: MAX_RESPONSE_BYTES,
            actual: length,
        });
    }
    let end = FRAME_HEADER_BYTES + length as usize;
    let body = frame
        .get(FRAME_HEADER_BYTES..end)
        .ok_or(FrameError::Truncated)?;
    if frame.len() != end {
        return Err(FrameError::TrailingBytes);
    }
    serde_json::from_slice::<Message<Response>>(body)
        .map_err(FrameError::Malformed)?
        .admit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_protocol_requires_and_preserves_declared_system_policy() {
        let declared: gripsack_ir::workspace_model::CondaSystemRequirements =
            serde_json::from_value(serde_json::json!({
                "libc":{"family":"glibc","version":"2.28"}, "linux":"4.18"
            }))
            .unwrap();
        let request = Request::Resolve(ResolveRequest {
            attempt: 7,
            channels: vec!["conda-forge".into()],
            packages: BTreeMap::from([("python".into(), "*".into())]),
            platform: "linux-64".into(),
            virtual_packages: vec![
                LockedVirtualPackage {
                    name: "__glibc".into(),
                    version: "2.28".into(),
                    build: "0".into(),
                },
                LockedVirtualPackage {
                    name: "__linux".into(),
                    version: "4.18".into(),
                    build: "0".into(),
                },
            ],
            system_requirements: declared.locked().unwrap(),
        });
        let mut frame = Vec::new();
        write_request_frame(&mut frame, &request).unwrap();
        assert_eq!(
            read_request_frame(&mut frame.as_slice()).unwrap().unwrap(),
            request
        );
        let mut wire = serde_json::to_value(Message {
            protocol: PROTOCOL_VERSION,
            payload: request,
        })
        .unwrap();
        wire["payload"]
            .as_object_mut()
            .unwrap()
            .remove("system_requirements");
        let raw = serde_json::to_vec(&wire).unwrap();
        let mut frame = (raw.len() as u64).to_le_bytes().to_vec();
        frame.extend_from_slice(&raw);
        assert!(matches!(
            read_request_frame(&mut frame.as_slice()),
            Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let raw = br#"{"protocol":3,"payload":{"kind":"resolve","attempt":1,"channels":[],"packages":{},"platform":"linux-64","virtual_packages":[],"system_requirements":{},"bogus":1}}"#;
        let mut frame = Vec::new();
        frame.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        frame.extend_from_slice(raw);
        assert!(matches!(
            read_request_frame(&mut &frame[..]),
            Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn unsupported_versions_and_extra_response_frames_are_refused() {
        let request = br#"{"protocol":2,"payload":{"kind":"resolve","attempt":1,"channels":[],"packages":{},"platform":"linux-64","virtual_packages":[],"system_requirements":{}}}"#;
        let mut frame = (request.len() as u64).to_le_bytes().to_vec();
        frame.extend_from_slice(request);
        assert!(matches!(
            read_request_frame(&mut frame.as_slice()),
            Err(FrameError::Version(2))
        ));
        let response = Response::Error(ErrorResponse {
            attempt: Some(1),
            code: "fixture".into(),
            message: "refused".into(),
        });
        let mut frame = Vec::new();
        write_response_frame(&mut frame, &response).unwrap();
        assert_eq!(decode_response_frame(&frame).unwrap(), response);
        frame.push(0);
        assert!(matches!(
            decode_response_frame(&frame),
            Err(FrameError::TrailingBytes)
        ));
    }

    #[test]
    fn serialization_limit_never_publishes_a_partial_frame() {
        let mut wire = Vec::new();
        assert!(write_frame(&mut wire, &"too-large", 4).is_err());
        assert!(wire.is_empty());
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
