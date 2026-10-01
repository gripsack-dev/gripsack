use gripsack_process::Sha256Digest;
use sha2::{Digest, Sha256};
use std::{io::{self, Read}, time::Instant};

/// EOF is granted only after the copied stream satisfies its compiled-in pin.
/// atomic_copy_with_mode therefore cannot publish a changed local spool.
pub(super) struct CheckedRead<R> {
    source: crate::spool::Limited<R>,
    expected: Sha256Digest,
    digest: Sha256,
    verified: bool,
    deadline: Instant,
}
impl<R: Read> CheckedRead<R> {
    pub fn new(source: R, expected: Sha256Digest, limit: u64, deadline: Instant) -> Self {
        Self { source:crate::spool::Limited::new(source,limit,"bridge publication"),expected,digest:Sha256::new(),verified:false,deadline }
    }
}
impl<R: Read> Read for CheckedRead<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() { return Ok(0); }
        if Instant::now() >= self.deadline { return Err(io::Error::new(io::ErrorKind::TimedOut,"bridge publication deadline expired")); }
        let count = self.source.read(buffer)?;
        if count == 0 && !self.verified {
            let actual = Sha256Digest::from_bytes(std::mem::take(&mut self.digest).finalize().into());
            if actual != self.expected { return Err(io::Error::new(io::ErrorKind::InvalidData,"bridge publication stream differs from its compiled-in SHA-256 pin")); }
            self.verified = true;
        } else if count != 0 {
            self.digest.update(&buffer[..count]);
        }
        Ok(count)
    }
}
