//! Local materialization evidence is checked against signed originals, never trust authority.
use super::{
    format::{BlobDescriptor, Component, EntryKind},
    runtime::failure,
};
use crate::ExecError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
};

/// Generated evidence may contain the complete closure's compositions plus
/// independently measured destination/Conda facts. It is not bounded by one
/// 4 MiB manifest. One 32 MiB contract governs encoding and retained reads.
pub(super) const MAX_MATERIALIZATION_RECORD_BYTES: usize =
    2 * super::closure::MAX_CLOSURE_METADATA_BYTES;
pub(super) const RECORD_FILE: &str = "receipt.json";

/// Bound allocation while serializing, before any published receipt can name
/// bytes its reader cannot admit. This also bounds environment.json evidence.
pub(super) fn encode_record(record: &impl Serialize) -> Result<Vec<u8>, ExecError> {
    let mut buffer = gripsack_process::InputBuffer::new(gripsack_process::InputByteLimit::new(
        MAX_MATERIALIZATION_RECORD_BYTES,
    ));
    serde_json::to_writer(&mut buffer, record).map_err(|error| {
        if error.io_error_kind() == Some(std::io::ErrorKind::InvalidInput) {
            failure("generated materialization record exceeds the 32 MiB byte bound")
        } else {
            ExecError::from(error)
        }
    })?;
    Ok(buffer.into_bytes())
}

pub(super) fn write_record(directory: &Path, record: &Receipt) -> Result<(), ExecError> {
    let bytes = encode_record(record)?;
    std::fs::write(directory.join(RECORD_FILE), bytes)?;
    Ok(())
}

pub(super) fn read_record(directory: &gripsack_fs::Dir) -> Result<Receipt, ExecError> {
    Ok(serde_json::from_slice(&read(
        directory,
        Path::new(RECORD_FILE),
        MAX_MATERIALIZATION_RECORD_BYTES as u64,
    )?)?)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    pub version: u32,
    pub manifest: String,
    pub realized_tree: String,
    pub payload: String,
    pub trust_revision: String,
    pub key_id: String,
    pub conda: Option<crate::workspace::conda::CondaRuntimeReceipt>,
    pub external: BTreeMap<String, ExternalReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<super::environment::EnvironmentReceipt>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExternalReceipt {
    pub trust_revision: String,
    pub key_id: String,
}

pub(super) fn read(
    directory: &gripsack_fs::Dir,
    path: &Path,
    limit: u64,
) -> Result<Vec<u8>, ExecError> {
    let file = gripsack_fs::open_file_nofollow(directory, path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(failure("portable record is not a bounded regular file"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(failure("portable record exceeds byte limit"));
    }
    Ok(bytes)
}

pub(super) fn blob(
    directory: &gripsack_fs::Dir,
    descriptor: &BlobDescriptor,
    destination: Option<&Path>,
) -> Result<(), ExecError> {
    let mut source = gripsack_fs::open_file_nofollow(directory, Path::new(&descriptor.sha256))?;
    let metadata = source.metadata()?;
    if !metadata.is_file() || metadata.len() != descriptor.size {
        return Err(failure(
            "portable blob size/type differs from signed descriptor",
        ));
    }
    let mut output = destination.map(std::fs::File::create).transpose()?;
    let mut digest = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| failure("blob size overflow"))?;
        if total > descriptor.size {
            return Err(failure("portable blob grew while reading"));
        }
        digest.update(&buffer[..count]);
        if let Some(output) = &mut output {
            output.write_all(&buffer[..count])?;
        }
    }
    if total != descriptor.size
        || gripsack_process::Sha256Digest::from_bytes(digest.finalize().into())
            != gripsack_process::Sha256Digest::parse(&descriptor.sha256)?
    {
        return Err(failure(
            "portable blob digest differs from signed descriptor",
        ));
    }
    Ok(())
}

pub(super) fn inventory(root: &Path, component: &Component) -> Result<(), ExecError> {
    let directory = gripsack_fs::open(root)?;
    let expected: BTreeSet<_> = component
        .inventory
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    let mut actual = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(parent) = pending.pop() {
        for entry in std::fs::read_dir(parent)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| failure("inventory path escaped root"))?;
            let relative = relative
                .to_str()
                .ok_or_else(|| failure("inventory path is not UTF-8"))?;
            if !expected.contains(relative) {
                return Err(failure("payload contains an undeclared inventory entry"));
            }
            actual.insert(relative.to_owned());
            if entry.file_type()?.is_dir() {
                pending.push(path);
            }
        }
    }
    if actual.len() != expected.len() {
        return Err(failure("payload is missing a signed inventory entry"));
    }
    for entry in &component.inventory {
        let path = root.join(&entry.path);
        let metadata = std::fs::symlink_metadata(&path)?;
        // Publication removes write bits. All other permissions remain signed.
        if metadata.permissions().mode() & 0o555 != entry.mode & 0o555 {
            return Err(failure("payload permissions differ from signed inventory"));
        }
        match &entry.kind {
            EntryKind::Directory if metadata.is_dir() => {}
            EntryKind::Symlink { target } if metadata.file_type().is_symlink() => {
                if std::fs::read_link(path)? != Path::new(target) {
                    return Err(failure("payload symlink differs from signed inventory"));
                }
            }
            EntryKind::File { sha256, size } if metadata.is_file() => {
                let mut file = gripsack_fs::open_file_nofollow(&directory, Path::new(&entry.path))?;
                if file.metadata()?.len() != *size {
                    return Err(failure("inventory file size mismatch"));
                }
                let mut digest = Sha256::new();
                let mut buffer = [0u8; 65536];
                let mut copied = 0u64;
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    copied = copied
                        .checked_add(count as u64)
                        .ok_or_else(|| failure("inventory file size overflow"))?;
                    if copied > *size {
                        return Err(failure("inventory file grew while reading"));
                    }
                    digest.update(&buffer[..count]);
                }
                if copied != *size
                    || gripsack_process::Sha256Digest::from_bytes(digest.finalize().into())
                        != gripsack_process::Sha256Digest::parse(sha256)?
                {
                    return Err(failure("inventory file digest mismatch"));
                }
            }
            _ => return Err(failure("payload entry type differs from signed inventory")),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "receipt_tests.rs"]
mod tests;
