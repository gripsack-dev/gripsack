//! Independent retained archive admission. Decompress each tar once into an
//! anonymous private spool, hashing every regular member while copying. Random
//! payload/document access never rescans a package or buffers compressed members.
#[path = "archive_headers.rs"]
mod headers;
use gripsack_process::Sha256Digest;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

// Measured large packages have <10k entries and <1 GiB expanded content. These
// hard ceilings also cover tar padding/metadata, not just installed payload.
const MAX_ENTRIES: usize = 131_072;
pub(super) const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_INFO_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TAR_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub(super) const MAX_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EntryKind {
    File {
        size: u64,
        sha256: Sha256Digest,
        offset: u64,
    },
    Symlink {
        target: String,
    },
    Directory,
}

#[derive(Debug, Clone)]
pub(super) struct ArchiveEntry {
    pub kind: EntryKind,
    pub mode: u32,
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("conda archive: {}", what.into()),
    )
}

pub(super) fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH_BYTES
        && !path.contains(['\\', '\0'])
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

pub(super) struct CondaArchive {
    spool: RefCell<File>,
    pub payload: BTreeMap<String, ArchiveEntry>,
    pub info: BTreeMap<String, ArchiveEntry>,
    pub expanded_bytes: u64,
}

impl CondaArchive {
    pub fn open(path: &Path, expected_sha256: &str) -> io::Result<Self> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| invalid("retained archive has no file name"))?;
        let directory = gripsack_fs::open(parent)?;
        let mut file = gripsack_fs::open_file_nofollow(&directory, Path::new(name))?.into_std();
        if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_TAR_BYTES {
            return Err(invalid("retained archive exceeds its byte/type bound"));
        }
        if hash_reader(&mut file, MAX_TAR_BYTES)? != Sha256Digest::parse(expected_sha256)? {
            return Err(invalid("retained archive differs from its frozen SHA-256"));
        }
        file.rewind()?;
        let mut magic = [0; 3];
        file.read_exact(&mut magic)?;
        file.rewind()?;
        let mut result = Self {
            spool: RefCell::new(tempfile::tempfile()?),
            payload: BTreeMap::new(),
            info: BTreeMap::new(),
            expanded_bytes: 0,
        };
        if &magic[..2] == b"PK" {
            let mut zip = zip::ZipArchive::new(file).map_err(invalid_error)?;
            // A .conda container consists of metadata.json and exactly two tars.
            if zip.len() != 3 {
                return Err(invalid(".conda must contain exactly three members"));
            }
            let (mut info, mut payload, mut metadata) = (false, false, false);
            for index in 0..zip.len() {
                let member = zip.by_index(index).map_err(invalid_error)?;
                let name = member.name();
                if name.starts_with("info-") && name.ends_with(".tar.zst") && !info {
                    info = true;
                    result.scan(zstd::stream::read::Decoder::new(member)?, Some(true))?;
                } else if name.starts_with("pkg-") && name.ends_with(".tar.zst") && !payload {
                    payload = true;
                    result.scan(zstd::stream::read::Decoder::new(member)?, Some(false))?;
                } else if name == "metadata.json" && !metadata {
                    metadata = true;
                    let mut bytes = Vec::new();
                    member.take(1025).read_to_end(&mut bytes)?;
                    if bytes.len() > 1024 {
                        return Err(invalid("oversized container metadata"));
                    }
                    let value: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(invalid_error)?;
                    if value
                        .get("conda_pkg_format_version")
                        .and_then(|v| v.as_u64())
                        != Some(2)
                    {
                        return Err(invalid("unsupported .conda format version"));
                    }
                } else {
                    return Err(invalid("duplicate or unsupported .conda member"));
                }
            }
            if !info || !payload || !metadata {
                return Err(invalid("missing .conda member"));
            }
        } else if &magic == b"BZh" {
            result.scan(bzip2::read::BzDecoder::new(file), None)?;
        } else {
            return Err(invalid("neither .conda nor .tar.bz2 magic"));
        }
        Ok(result)
    }

    fn scan(&mut self, reader: impl Read, section: Option<bool>) -> io::Result<()> {
        let remaining = MAX_TAR_BYTES - self.expanded_bytes;
        let mut limited = reader.take(remaining + 1);
        {
            let mut archive = tar::Archive::new(&mut limited);
            let mut extended = headers::ExtendedHeader::default();
            for (count, member) in archive.entries()?.raw(true).enumerate() {
                let mut member = member?;
                if count >= MAX_ENTRIES || self.payload.len() + self.info.len() >= MAX_ENTRIES {
                    return Err(invalid("archive entry count exceeds its bound"));
                }
                if extended.read(&mut member)? {
                    continue;
                }
                let raw = extended.path.as_ref().map_or_else(
                    || member.path_bytes(),
                    |path| std::borrow::Cow::Borrowed(path.as_bytes()),
                );
                let raw = std::str::from_utf8(&raw).map_err(invalid_error)?;
                let raw = raw.strip_prefix("./").unwrap_or(raw);
                let path = raw.trim_end_matches('/').to_owned();
                if !safe_path(&path) {
                    return Err(invalid(format!("unsafe path {path:?}")));
                }
                let is_info = path.starts_with("info/") || path == "info";
                // Modern conda-build puts info/licenses in the pkg tar, but
                // these remain archive metadata rather than installed payload.
                if section == Some(true) && !is_info {
                    return Err(invalid("payload member is in the info .conda tar"));
                }
                let mode = member.header().mode()? & 0o7777;
                if mode & 0o7000 != 0 {
                    return Err(invalid("special permission bits are unsupported"));
                }
                let kind = match member.header().entry_type() {
                    tar::EntryType::Regular => {
                        let size = member.size();
                        let bound = if is_info {
                            MAX_INFO_BYTES
                        } else {
                            MAX_ENTRY_BYTES
                        };
                        if size > bound {
                            return Err(invalid(format!("{path:?} exceeds its byte bound")));
                        }
                        let spool = self.spool.get_mut();
                        let offset = spool.stream_position()?;
                        let mut digest = Sha256::new();
                        let mut buffer = [0; 64 * 1024];
                        let mut copied = 0;
                        loop {
                            let count = member.read(&mut buffer)?;
                            if count == 0 {
                                break;
                            }
                            spool.write_all(&buffer[..count])?;
                            digest.update(&buffer[..count]);
                            copied += count as u64;
                        }
                        if copied != size {
                            return Err(invalid("truncated regular member"));
                        }
                        EntryKind::File {
                            size,
                            sha256: Sha256Digest::from_bytes(digest.finalize().into()),
                            offset,
                        }
                    }
                    tar::EntryType::Symlink => {
                        let target = match extended.link.take() {
                            Some(target) => target,
                            None => member
                                .link_name()?
                                .ok_or_else(|| invalid("missing link target"))?
                                .to_str()
                                .ok_or_else(|| invalid("non-UTF-8 link target"))?
                                .to_owned(),
                        };
                        if target.is_empty()
                            || target.len() > MAX_PATH_BYTES
                            || target.contains(['\\', '\0'])
                        {
                            return Err(invalid("invalid symlink target"));
                        }
                        EntryKind::Symlink { target }
                    }
                    tar::EntryType::Directory => EntryKind::Directory,
                    _ => return Err(invalid(format!("unsupported member kind at {path:?}"))),
                };
                extended = headers::ExtendedHeader::default();
                let listing = if is_info {
                    &mut self.info
                } else {
                    &mut self.payload
                };
                if listing.insert(path, ArchiveEntry { kind, mode }).is_some() {
                    return Err(invalid("duplicate archive path"));
                }
            }
            if extended.pending() {
                return Err(invalid("extended tar header has no member"));
            }
        }
        // Account for trailing padding and reject expansion beyond the limit.
        let mut padding = [0; 64 * 1024];
        loop {
            let count = limited.read(&mut padding)?;
            if count == 0 {
                break;
            }
            if padding[..count].iter().any(|byte| *byte != 0) {
                return Err(invalid("nonzero data follows the tar terminator"));
            }
        }
        let consumed = remaining + 1 - limited.limit();
        if consumed > remaining {
            return Err(invalid("expanded tar exceeds its byte bound"));
        }
        self.expanded_bytes += consumed;
        Ok(())
    }

    fn bytes(&self, entry: &ArchiveEntry) -> io::Result<Vec<u8>> {
        let EntryKind::File { size, offset, .. } = entry.kind else {
            return Err(invalid("requested document/payload is not a regular file"));
        };
        let mut spool = self.spool.borrow_mut();
        spool.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::with_capacity(size as usize);
        Read::by_ref(&mut *spool)
            .take(size)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != size {
            return Err(invalid("truncated private spool"));
        }
        Ok(bytes)
    }

    pub fn info_document(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        self.info
            .get(&format!("info/{name}"))
            .map(|entry| self.bytes(entry))
            .transpose()
    }

    pub fn payload_bytes(&self, path: &str) -> io::Result<Vec<u8>> {
        self.bytes(
            self.payload
                .get(path)
                .ok_or_else(|| invalid("missing original payload"))?,
        )
    }
}

fn invalid_error(error: impl std::fmt::Display) -> io::Error {
    invalid(error.to_string())
}

pub(super) fn hash_reader(reader: impl Read, bound: u64) -> io::Result<Sha256Digest> {
    let mut limited = reader.take(bound + 1);
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut total = 0;
    loop {
        let count = limited.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > bound {
            return Err(invalid("stream exceeds its byte bound"));
        }
        digest.update(&buffer[..count]);
    }
    Ok(Sha256Digest::from_bytes(digest.finalize().into()))
}
