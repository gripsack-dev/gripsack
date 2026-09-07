//! Fixed-buffer streams and private seekable spools, never payload-sized Vecs.

use crate::{DownloadHash, FetchError};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};

#[derive(Debug)]
pub(crate) struct LimitExceeded {
    pub what: &'static str,
    pub limit: u64,
}
impl std::fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} exceeds the {} byte cap", self.what, self.limit)
    }
}
impl std::error::Error for LimitExceeded {}

pub(crate) struct Limited<R> {
    reader: R,
    remaining: u64,
    limit: u64,
    what: &'static str,
}
impl<R> Limited<R> {
    pub(crate) fn new(reader: R, limit: u64, what: &'static str) -> Self {
        Self {
            reader,
            remaining: limit,
            limit,
            what,
        }
    }
}
impl<R: Read> Read for Limited<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut probe = [0];
            return match self.reader.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    LimitExceeded {
                        what: self.what,
                        limit: self.limit,
                    },
                )),
            };
        }
        let max = self.remaining.min(buffer.len() as u64) as usize;
        let read = self.reader.read(&mut buffer[..max])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

pub(crate) struct Download {
    pub file: tempfile::NamedTempFile,
    pub hash: DownloadHash,
}

pub(crate) fn download(reader: impl Read, limit: u64) -> Result<Download, FetchError> {
    let mut file = tempfile::Builder::new()
        .prefix("grip-download-")
        .tempfile()?;
    let hash = copy_hashed(reader, file.as_file_mut(), limit)?;
    Ok(Download { file, hash })
}

pub(crate) fn copy_hashed(
    reader: impl Read,
    mut output: impl Write,
    limit: u64,
) -> Result<DownloadHash, FetchError> {
    let mut reader = Limited::new(reader, limit, "download");
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = match reader.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
    Ok(DownloadHash::finish(digest))
}

pub(crate) fn expanded_spool(
    reader: impl Read,
    limit: u64,
) -> Result<tempfile::NamedTempFile, FetchError> {
    let mut input = Limited::new(reader, limit, "expanded payload");
    let mut file = tempfile::Builder::new()
        .prefix("grip-expanded-")
        .tempfile()?;
    io::copy(&mut input, file.as_file_mut())?;
    Ok(file)
}
