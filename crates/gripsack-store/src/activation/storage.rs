//! Capability-rooted private records. A bounded encoder/reader precedes serde
//! or atomic publication. Recovery seals observed records before their contents
//! authorize skipping an effect or discarding pending evidence.
use crate::private_state;
use gripsack_fs::{
    Dir,
    fault::{Boundary, operation},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    io::{self, Read, Write},
    path::Path,
};

pub(super) const POINTER: &str = "activation.json";
pub(super) const ROOT: &str = "activation";
pub(super) const PLAN: &str = "plan.json";
pub(super) const RECEIPT: &str = "receipt.json";
pub(super) const OUTCOMES: &str = "outcomes";
pub(super) const MAX_INTENTS: usize = 4096;

#[derive(Clone, Copy)]
pub(super) enum RecordKind {
    Pointer,
    Plan,
    Outcome,
    Receipt,
}
impl RecordKind {
    fn limit(self) -> usize {
        match self {
            Self::Pointer => 4096,
            Self::Outcome => 64 * 1024,
            Self::Plan | Self::Receipt => 16 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Access {
    Inspect,
    Recover,
}

pub(super) fn read_bytes(
    directory: &Dir,
    name: &Path,
    kind: RecordKind,
    access: Access,
) -> io::Result<Option<Vec<u8>>> {
    let mut file = match gripsack_fs::open_file_nofollow(directory, name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() > kind.limit() as u64 {
        return Err(oversized());
    }
    if access == Access::Recover {
        private_state::restrict_file(&file, name)?;
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(kind.limit() as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > kind.limit() {
        return Err(oversized());
    }
    if access == Access::Recover {
        operation(Boundary::FileSync, name, || file.sync_all())?;
        gripsack_fs::fsync_dir(directory, Path::new("."))?;
    }
    Ok(Some(bytes))
}

pub(super) fn read<T: DeserializeOwned>(
    directory: &Dir,
    name: &Path,
    kind: RecordKind,
    access: Access,
) -> io::Result<Option<T>> {
    read_bytes(directory, name, kind, access)?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        })
        .transpose()
}

pub(super) fn required<T: DeserializeOwned>(
    directory: &Dir,
    name: &Path,
    kind: RecordKind,
    access: Access,
) -> io::Result<T> {
    read(directory, name, kind, access)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "required activation evidence is missing",
        )
    })
}

pub(super) fn encode(value: &impl Serialize, kind: RecordKind) -> io::Result<Vec<u8>> {
    struct Encoder {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Encoder {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit - self.bytes.len() {
                return Err(oversized());
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut encoder = Encoder {
        bytes: Vec::new(),
        limit: kind.limit(),
    };
    serde_json::to_writer(&mut encoder, value).map_err(io::Error::other)?;
    Ok(encoder.bytes)
}

pub(super) fn write(
    directory: &Dir,
    name: &Path,
    value: &impl Serialize,
    kind: RecordKind,
) -> io::Result<()> {
    gripsack_fs::atomic_write_with_mode(directory, name, &encode(value, kind)?, 0o600)
}

pub(super) fn open_directory(parent: &Dir, name: &Path, access: Access) -> io::Result<Dir> {
    let directory = gripsack_fs::open_dir_nofollow(parent, name)?;
    if access == Access::Recover {
        private_state::restrict_directory(&directory, name)?;
    }
    Ok(directory)
}

fn oversized() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "activation metadata exceeds its record budget",
    )
}
