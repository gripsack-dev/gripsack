//! Bounded reader for retained Conda package archives (.conda zip +
//! zstd tars, legacy .tar.bz2). The core reads package inventories and
//! original payload bytes itself: helper-produced trees are validated
//! against THESE bytes, never against helper claims. Every count and
//! byte total is capped; caps are justified against the portable lock
//! bound (the closure is lock-scale) and measured closure contents.
//! Listings are one pass; original payload bytes are re-read on demand
//! so whole packages never become RAM.

use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};

/// A package archive carries at most this many entries. Measured
/// conda-forge closures stay below ~10k entries per package (large
/// packages like numpy ~4k); 16× headroom stays far under memory bounds.
const MAX_ENTRIES: usize = 131_072;
/// One info/* document. info/paths.json is proportional to the entry
/// count; 32 MiB covers ~500k path records, far past the entry cap.
const MAX_INFO_BYTES: u64 = 32 * 1024 * 1024;
/// Original bytes are re-read only for prefix-patched files
/// (executables, shared libraries, scripts). libpython in the measured
/// closure is ~30 MiB; 16x headroom.
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// Decompressed tars are bounded by the compressed archive's expansion;
/// conda packages stay far below this (largest measured ~1 GiB raw).
const MAX_TAR_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EntryKind {
    File { size: u64 },
    Symlink { target: String },
    Directory,
    Other,
}

#[derive(Debug, Clone)]
pub(super) struct ArchiveEntry {
    pub path: String,
    pub kind: EntryKind,
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("conda archive: {}", what.into()),
    )
}

/// A bounded decompressed-tar cursor: listing with optional single-entry
/// byte capture. Expansion beyond MAX_TAR_BYTES is a hard error.
fn scan_tar(
    read: impl Read,
    what: &str,
    wanted: Option<&str>,
) -> io::Result<(Vec<ArchiveEntry>, Option<Vec<u8>>)> {
    let mut limited = read.take(MAX_TAR_BYTES);
    let mut archive = tar::Archive::new(&mut limited);
    let mut entries = Vec::new();
    let mut captured = None;
    for entry in archive.entries().map_err(|e| invalid(format!("{what}: {e}")))? {
        let mut entry = entry.map_err(|e| invalid(format!("{what}: {e}")))?;
        if entries.len() >= MAX_ENTRIES {
            return Err(invalid(format!("more than {MAX_ENTRIES} entries")));
        }
        let path = entry
            .path()
            .map_err(|e| invalid(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches("./")
            .to_owned();
        let header = entry.header();
        let kind = match header.entry_type() {
            tar::EntryType::Regular => EntryKind::File {
                size: header.size().map_err(|e| invalid(e.to_string()))?,
            },
            tar::EntryType::Symlink => EntryKind::Symlink {
                target: header
                    .link_name()
                    .map_err(|e| invalid(e.to_string()))?
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            },
            tar::EntryType::Directory => EntryKind::Directory,
            _ => EntryKind::Other,
        };
        if wanted == Some(path.as_str()) {
            let mut bytes = Vec::new();
            let read = entry
                .take(MAX_ENTRY_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| invalid(e.to_string()))?;
            if read as u64 > MAX_ENTRY_BYTES {
                return Err(invalid(format!("entry {path:?} exceeds its byte bound")));
            }
            captured = Some(bytes);
        }
        entries.push(ArchiveEntry { path, kind });
    }
    Ok((entries, captured))
}

fn zstd_tar(bytes: &[u8]) -> io::Result<impl Read + '_> {
    zstd::stream::read::Decoder::new(bytes).map_err(|e| invalid(e.to_string()))
}

/// One opened package archive: full entry listings plus every info/*
/// document (each bounded); payload bytes are re-read on demand.
pub(super) struct CondaArchive {
    path: PathBuf,
    format: Format,
    payload: Vec<ArchiveEntry>,
    info: Vec<ArchiveEntry>,
    documents: Vec<(String, Vec<u8>)>,
    /// .conda inner tar member names for on-demand payload reads.
    pkg_member: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Conda,
    TarBz2,
}

impl CondaArchive {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let mut magic = [0u8; 3];
        let read = file.read(&mut magic)?;
        file.rewind()?;
        if read >= 2 && &magic[..2] == b"PK" {
            return Self::open_conda(path, file);
        }
        if read >= 3 && &magic[..3] == b"BZh" {
            return Self::open_tarbz2(path, file);
        }
        Err(invalid("neither .conda zip nor .tar.bz2 bzip2 magic"))
    }

    fn open_conda(path: &Path, file: std::fs::File) -> io::Result<Self> {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| invalid(e.to_string()))?;
        if zip.len() > MAX_ENTRIES {
            return Err(invalid(format!("more than {MAX_ENTRIES} zip members")));
        }
        let mut payload = Vec::new();
        let mut info = Vec::new();
        let mut documents = Vec::new();
        let mut pkg_member = None;
        for index in 0..zip.len() {
            let mut member = zip.by_index(index).map_err(|e| invalid(e.to_string()))?;
            let name = member.name().to_owned();
            let is_info = name.starts_with("info-") && name.ends_with(".tar.zst");
            let is_pkg = name.starts_with("pkg-") && name.ends_with(".tar.zst");
            if !is_info && !is_pkg {
                continue;
            }
            let mut compressed = Vec::new();
            let read = member
                .by_ref()
                .take(MAX_ENTRY_BYTES + 1)
                .read_to_end(&mut compressed)
                .map_err(|e| invalid(e.to_string()))?;
            if read as u64 > MAX_ENTRY_BYTES {
                return Err(invalid(format!("zip member {name:?} exceeds its byte bound")));
            }
            if is_info {
                let (entries, _) = scan_tar(zstd_tar(&compressed)?, "info tar", None)?;
                for entry in &entries {
                    if let EntryKind::File { size } = entry.kind
                        && size <= MAX_INFO_BYTES
                        && entry.path.starts_with("info/")
                    {
                        let (_, bytes) =
                            scan_tar(zstd_tar(&compressed)?, "info tar", Some(&entry.path))?;
                            if let Some(bytes) = bytes {
                                documents.push((entry.path.clone(), bytes));
                            }
                    }
                }
                info = entries;
            } else {
                let (entries, _) = scan_tar(zstd_tar(&compressed)?, "pkg tar", None)?;
                payload = entries;
                pkg_member = Some(name);
            }
        }
        if pkg_member.is_none() || info.is_empty() {
            return Err(invalid(".conda is missing its info/pkg tars"));
        }
        Ok(Self {
            path: path.to_owned(),
            format: Format::Conda,
            payload,
            info,
            documents,
            pkg_member,
        })
    }

    fn open_tarbz2(path: &Path, file: std::fs::File) -> io::Result<Self> {
        let decoder = bzip2::read::BzDecoder::new(file);
        let (entries, _) = scan_tar(decoder, ".tar.bz2", None)?;
        let mut payload = Vec::new();
        let mut info = Vec::new();
        for entry in entries {
            if entry.path.starts_with("info/") {
                info.push(entry);
            } else {
                payload.push(entry);
            }
        }
        Ok(Self {
            path: path.to_owned(),
            format: Format::TarBz2,
            payload,
            info,
            documents: Vec::new(),
            pkg_member: None,
        })
    }

    pub fn payload_entries(&self) -> &[ArchiveEntry] {
        &self.payload
    }

    pub fn info_entries(&self) -> &[ArchiveEntry] {
        &self.info
    }

    /// Bytes of one info/* document (paths.json etc.), each bounded.
    /// .tar.bz2 documents are re-streamed from the retained archive.
    pub fn info_document(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let wanted = format!("info/{name}");
        if let Some((_, bytes)) = self.documents.iter().find(|(path, _)| *path == wanted) {
            return Ok(Some(bytes.clone()));
        }
        match self.format {
            Format::Conda => Ok(None),
            Format::TarBz2 => {
                if !self.info.iter().any(|entry| entry.path == wanted) {
                    return Ok(None);
                }
                let decoder = bzip2::read::BzDecoder::new(std::fs::File::open(&self.path)?);
                let (_, bytes) = scan_tar(decoder, ".tar.bz2", Some(&wanted))?;
                Ok(bytes)
            }
        }
    }

    /// Original payload bytes for prefix-patch verification, re-read on
    /// demand. A missing entry is None; oversized entries are errors at
    /// scan time, never silent truncation.
    pub fn payload_bytes(&self, path: &str) -> io::Result<Option<Vec<u8>>> {
        if !self.payload.iter().any(|entry| entry.path == path) {
            return Ok(None);
        }
        match self.format {
            Format::Conda => {
                let member = self
                    .pkg_member
                    .as_ref()
                    .expect("pkg member recorded")
                    .clone();
                let mut zip =
                    zip::ZipArchive::new(std::fs::File::open(&self.path)?)
                        .map_err(|e| invalid(e.to_string()))?;
                let mut member = zip.by_name(&member).map_err(|e| invalid(e.to_string()))?;
                let mut compressed = Vec::new();
                let read = member
                    .by_ref()
                    .take(MAX_ENTRY_BYTES + 1)
                    .read_to_end(&mut compressed)
                    .map_err(|e| invalid(e.to_string()))?;
                if read as u64 > MAX_ENTRY_BYTES {
                    return Err(invalid("pkg tar exceeds its byte bound"));
                }
                let (_, bytes) = scan_tar(zstd_tar(&compressed)?, "pkg tar", Some(path))?;
                Ok(bytes)
            }
            Format::TarBz2 => {
                let decoder = bzip2::read::BzDecoder::new(std::fs::File::open(&self.path)?);
                let (_, bytes) = scan_tar(decoder, ".tar.bz2", Some(path))?;
                Ok(bytes)
            }
        }
    }
}
