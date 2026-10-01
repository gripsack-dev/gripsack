use super::{MAX_FRAME_BYTES, ProtocolError};
use gripsack_process::{InputBuffer, InputByteLimit};
use serde::{Serialize, de::DeserializeOwned};
use std::io::Write;

const HEADER_BYTES: usize = std::mem::size_of::<u64>();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame<T> {
    pub body: T,
}

pub fn encode_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, ProtocolError> {
    let mut request = InputBuffer::new(InputByteLimit::new(MAX_FRAME_BYTES + HEADER_BYTES));
    request.write_all(&[0; HEADER_BYTES])?;
    serde_json::to_writer(&mut request, message)?;
    let mut bytes = request.into_bytes();
    let length = (bytes.len() - HEADER_BYTES) as u64;
    bytes[..HEADER_BYTES].copy_from_slice(&length.to_le_bytes());
    Ok(bytes)
}

pub fn decode_frame<T: DeserializeOwned>(bytes: &[u8]) -> Result<Frame<T>, ProtocolError> {
    let header: &[u8; HEADER_BYTES] = bytes.first_chunk().ok_or(ProtocolError::TruncatedFrame)?;
    let length = admitted_length(header)?;
    let Some(body) = bytes.get(HEADER_BYTES..HEADER_BYTES + length) else {
        return Err(ProtocolError::TruncatedFrame);
    };
    if bytes.len() != HEADER_BYTES + length {
        return Err(ProtocolError::TrailingBytes);
    }
    Ok(Frame {
        body: serde_json::from_slice(body)?,
    })
}

fn admitted_length(header: &[u8; HEADER_BYTES]) -> Result<usize, ProtocolError> {
    let declared = u64::from_le_bytes(*header);
    if declared > MAX_FRAME_BYTES as u64 {
        return Err(ProtocolError::OversizedFrame(declared));
    }
    Ok(declared as usize)
}

/// Incremental bytes from the common process supervisor; the header is admitted
/// before reserving body space. One allocation is reused across bounded frames.
#[derive(Default)]
pub struct FrameDecoder {
    header: [u8; HEADER_BYTES],
    header_used: usize,
    declared: Option<usize>,
    body: Vec<u8>,
}
impl FrameDecoder {
    pub fn push<T: DeserializeOwned>(
        &mut self,
        mut bytes: &[u8],
        mut receive: impl FnMut(T) -> Result<(), ProtocolError>,
    ) -> Result<(), ProtocolError> {
        while !bytes.is_empty() {
            if self.declared.is_none() {
                let count = (HEADER_BYTES - self.header_used).min(bytes.len());
                self.header[self.header_used..self.header_used + count]
                    .copy_from_slice(&bytes[..count]);
                self.header_used += count;
                bytes = &bytes[count..];
                if self.header_used < HEADER_BYTES {
                    continue;
                }
                let length = admitted_length(&self.header)?;
                self.body.clear();
                if self.body.capacity() < length {
                    self.body.reserve_exact(length);
                }
                self.declared = Some(length);
            }
            // A complete zero-byte body is still decoded (and rejected as JSON),
            // rather than being mistaken for a clean EOF after a header.
            let length = self.declared.ok_or(ProtocolError::TruncatedFrame)?;
            let count = (length - self.body.len()).min(bytes.len());
            self.body.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.body.len() == length {
                let message = serde_json::from_slice(&self.body)?;
                receive(message)?;
                self.header_used = 0;
                self.declared = None;
                self.body.clear();
            }
        }
        Ok(())
    }
    pub fn finish(self) -> Result<(), ProtocolError> {
        if self.header_used != 0 || self.declared.is_some() {
            Err(ProtocolError::TruncatedFrame)
        } else {
            Ok(())
        }
    }
}
