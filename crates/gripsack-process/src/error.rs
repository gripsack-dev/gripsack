//! Stable bounded native failure metadata. No arbitrary error prose, paths,
//! command arguments or environment values enter a persisted receipt.
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeIoKind {
    NotFound,
    PermissionDenied,
    InvalidInput,
    InvalidData,
    Deadline,
    Unsupported,
    Interrupted,
    WouldBlock,
    ResourceUnavailable,
    Io,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeIoError {
    pub kind: NativeIoKind,
    pub os_code: Option<i32>,
}
impl NativeIoError {
    pub fn capture(error: &io::Error) -> Self {
        let kind = match error.kind() {
            io::ErrorKind::NotFound => NativeIoKind::NotFound,
            io::ErrorKind::PermissionDenied => NativeIoKind::PermissionDenied,
            io::ErrorKind::InvalidInput => NativeIoKind::InvalidInput,
            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => NativeIoKind::InvalidData,
            io::ErrorKind::TimedOut => NativeIoKind::Deadline,
            io::ErrorKind::Unsupported => NativeIoKind::Unsupported,
            io::ErrorKind::Interrupted => NativeIoKind::Interrupted,
            io::ErrorKind::WouldBlock => NativeIoKind::WouldBlock,
            io::ErrorKind::OutOfMemory
            | io::ErrorKind::StorageFull
            | io::ErrorKind::QuotaExceeded
            | io::ErrorKind::ResourceBusy => NativeIoKind::ResourceUnavailable,
            _ => NativeIoKind::Io,
        };
        Self {
            kind,
            os_code: error.raw_os_error(),
        }
    }
}
