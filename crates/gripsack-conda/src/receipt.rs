//! Deterministic conda-meta bytes: complete frozen record, installed paths and
//! final destination, reconstructed from lock/archive authority, not helper output.
use gripsack_ir::workspace_model::lock::LockedCondaPackage;
use rattler_conda_types::package::DistArchiveIdentifier;
use serde::Serialize;
use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
};

/// Generated receipt permissions before the shared store publication seals
/// regular files. Independent of the installer's inherited umask.
pub const FILE_MODE: u32 = 0o644;

#[derive(Debug, thiserror::Error)]
pub enum ReceiptError {
    #[error("the frozen artifact URL has no Conda archive identifier")]
    ArchiveIdentifier,
    #[error("encoding or comparing normalized receipt: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("reading normalized receipt: {0}")]
    Read(#[from] io::Error),
    #[error("normalized receipt has trailing bytes")]
    TrailingBytes,
}

#[derive(Serialize)]
struct Receipt<'a, S> {
    #[serde(flatten)]
    record: &'a LockedCondaPackage,
    #[serde(rename = "fn")]
    identifier: DistArchiveIdentifier,
    files: &'a BTreeSet<S>,
    final_prefix: &'a str,
}

pub fn write<S: AsRef<str> + Serialize>(
    writer: impl Write,
    record: &LockedCondaPackage,
    files: &BTreeSet<S>,
    final_prefix: &str,
) -> Result<(), ReceiptError> {
    let url = url::Url::parse(&record.url).map_err(|_| ReceiptError::ArchiveIdentifier)?;
    let identifier =
        DistArchiveIdentifier::try_from_url(&url).ok_or(ReceiptError::ArchiveIdentifier)?;
    serde_json::to_writer(
        writer,
        &Receipt {
            record,
            identifier,
            files,
            final_prefix,
        },
    )?;
    Ok(())
}

/// Compare incrementally; neither an untrusted receipt length nor its JSON shape
/// controls an allocation. Duplicate/extra keys and missing metadata cannot pass.
pub fn verify<S: AsRef<str> + Serialize>(
    reader: impl Read,
    record: &LockedCondaPackage,
    files: &BTreeSet<S>,
    final_prefix: &str,
) -> Result<(), ReceiptError> {
    let mut comparison = Comparison(io::BufReader::new(reader));
    write(&mut comparison, record, files, final_prefix)?;
    let mut extra = [0];
    if comparison.0.read(&mut extra)? != 0 {
        return Err(ReceiptError::TrailingBytes);
    }
    Ok(())
}

struct Comparison<R>(R);
impl<R: Read> Write for Comparison<R> {
    fn write(&mut self, expected: &[u8]) -> io::Result<usize> {
        let mut buffer = [0; 4096];
        for chunk in expected.chunks(buffer.len()) {
            self.0.read_exact(&mut buffer[..chunk.len()])?;
            if buffer[..chunk.len()] != *chunk {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "receipt differs from frozen record or archive-derived paths",
                ));
            }
        }
        Ok(expected.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
