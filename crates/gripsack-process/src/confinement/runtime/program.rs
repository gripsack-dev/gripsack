//! One bounded measurement binds every selected runtime program to approval.
use crate::{ResolvedProgram, Sha256Digest, executable::Interpreter};
use sha2::{Digest, Sha256};
use std::{io::{self, Read, Seek}, os::unix::fs::{OpenOptionsExt, PermissionsExt}, path::PathBuf};

#[derive(Debug, serde::Serialize)]
pub(super) struct RuntimeProgram {
    declared: PathBuf,
    canonical: PathBuf,
    sha256: Sha256Digest,
}

impl RuntimeProgram {
    pub(super) fn digest(&self) -> Sha256Digest { self.sha256 }
}

pub(super) fn measure(selected: &ResolvedProgram) -> io::Result<(RuntimeProgram, Option<Interpreter>)> {
    let mut file = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(selected.canonical())?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "selected runtime program is not an executable regular file"));
    }
    if metadata.len() > crate::image::EXECUTABLE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "runtime program exceeds its executable admission budget"));
    }
    let classified = crate::executable::classify(&mut file)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    file.rewind()?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0; 32 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 { break; }
        total = total.checked_add(count as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "runtime byte count overflow"))?;
        if total > crate::image::EXECUTABLE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "runtime program exceeds its executable admission budget"));
        }
        hash.update(&buffer[..count]);
    }
    Ok((RuntimeProgram {
        declared: selected.declared().to_owned(),
        canonical: selected.canonical().to_owned(),
        sha256: Sha256Digest::from_bytes(hash.finalize().into()),
    }, classified.interpreter))
}
