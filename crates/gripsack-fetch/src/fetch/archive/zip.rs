use super::{
    links::{Graph, Kind},
    paths,
    tree::Budget,
};
use crate::{FetchError, FetchLimits};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

fn number<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes.try_into().expect("fixed ZIP field")
}

/// Bound central-directory allocations before ZipArchive builds its index.
fn metadata_bounds(file: &mut std::fs::File, limits: FetchLimits) -> Result<(), FetchError> {
    let length = file.metadata()?.len();
    let tail_len = length.min(65_557) as usize;
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0; tail_len];
    file.read_exact(&mut tail)?;
    let offset = tail
        .windows(4)
        .enumerate()
        .rev()
        .find_map(|(at, bytes)| {
            if bytes != b"PK\x05\x06" || at + 22 > tail.len() {
                return None;
            }
            let comment = u16::from_le_bytes(number(&tail[at + 20..at + 22])) as usize;
            (at + 22 + comment == tail.len()).then_some(at)
        })
        .ok_or_else(|| paths::violation(Path::new("zip"), "missing ZIP directory footer"))?;
    let end = &tail[offset..offset + 22];
    if end[4..8] != [0; 4] {
        return Err(paths::violation(
            Path::new("zip"),
            "multi-disk ZIP is unsupported",
        ));
    }
    if end[8..10] != end[10..12] {
        return Err(paths::violation(
            Path::new("zip"),
            "inconsistent ZIP entry counts",
        ));
    }
    let mut entries = u16::from_le_bytes(number(&end[10..12])) as u64;
    let mut directory_bytes = u32::from_le_bytes(number(&end[12..16])) as u64;
    let eocd_position = length - tail_len as u64 + offset as u64;
    let mut directory_end = eocd_position;
    let mut locator = [0; 20];
    if eocd_position >= 20 {
        file.seek(SeekFrom::Start(eocd_position - 20))?;
        file.read_exact(&mut locator)?;
    }
    if &locator[..4] == b"PK\x06\x07" {
        let position = u64::from_le_bytes(number(&locator[8..16]));
        let mut zip64 = [0; 56];
        file.seek(SeekFrom::Start(position))?;
        file.read_exact(&mut zip64)?;
        if &zip64[..4] != b"PK\x06\x06" || zip64[16..24] != [0; 8] {
            return Err(paths::violation(
                Path::new("zip"),
                "invalid ZIP64 directory footer",
            ));
        }
        let record_size = u64::from_le_bytes(number(&zip64[4..12]));
        if record_size < 44
            || record_size > limits.decoder_bytes.get().min(64 * 1024 * 1024)
            || position
                .checked_add(12)
                .and_then(|start| start.checked_add(record_size))
                .is_none_or(|end| end > eocd_position.saturating_sub(20))
            || zip64[24..32] != zip64[32..40]
        {
            return Err(paths::violation(
                Path::new("zip"),
                "invalid ZIP64 metadata extent",
            ));
        }
        directory_end = position;
        entries = u64::from_le_bytes(number(&zip64[32..40]));
        directory_bytes = u64::from_le_bytes(number(&zip64[40..48]));
    }
    if entries > limits.archive_entries.get() as u64 {
        return Err(FetchError::TooManyEntries {
            limit: limits.archive_entries.get(),
        });
    }
    let cap = limits.decoder_bytes.get().min(64 * 1024 * 1024);
    if directory_bytes > cap {
        return Err(FetchError::PayloadTooLarge {
            what: "ZIP directory metadata".into(),
            limit: cap,
        });
    }
    let start = directory_end
        .checked_sub(directory_bytes)
        .ok_or_else(|| paths::violation(Path::new("zip"), "invalid central directory extent"))?;
    file.seek(SeekFrom::Start(start))?;
    let mut consumed = 0u64;
    for _ in 0..entries {
        let mut header = [0; 46];
        file.read_exact(&mut header)?;
        if &header[..4] != b"PK\x01\x02" {
            return Err(paths::violation(
                Path::new("zip"),
                "invalid central directory record",
            ));
        }
        let extra = u16::from_le_bytes(number(&header[28..30])) as u64
            + u16::from_le_bytes(number(&header[30..32])) as u64
            + u16::from_le_bytes(number(&header[32..34])) as u64;
        consumed = consumed
            .checked_add(46 + extra)
            .filter(|bytes| *bytes <= directory_bytes && *bytes <= cap)
            .ok_or_else(|| {
                paths::violation(
                    Path::new("zip"),
                    "central directory exceeds its bounded extent",
                )
            })?;
        file.seek(SeekFrom::Current(extra as i64))?;
    }
    if consumed != directory_bytes {
        return Err(paths::violation(
            Path::new("zip"),
            "unaccounted central directory data",
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

/// Decode only admitted methods, with explicit decoder-memory limits.
/// Raw ZIP readers would silently copy compressed bytes and bypass CRC.
fn decoded<'a, R: Read + ?Sized + 'a>(
    raw: ::zip::read::ZipFile<'a, R>,
    limits: FetchLimits,
) -> Result<Box<dyn Read + 'a>, FetchError> {
    let method = raw.compression();
    match method {
        ::zip::CompressionMethod::Stored => Ok(Box::new(raw)),
        ::zip::CompressionMethod::Deflated => Ok(Box::new(flate2::read::DeflateDecoder::new(raw))),
        ::zip::CompressionMethod::Bzip2 => {
            if limits.decoder_bytes.get() < 8 * 1024 * 1024 {
                return Err(FetchError::PayloadTooLarge {
                    what: "bzip2 decoder memory".into(),
                    limit: limits.decoder_bytes.get(),
                });
            }
            Ok(Box::new(bzip2::read::BzDecoder::new(raw)))
        }
        ::zip::CompressionMethod::Zstd => {
            let mut decoder = zstd::stream::read::Decoder::new(raw)?;
            decoder.window_log_max(limits.decoder_bytes.get().ilog2().min(31))?;
            Ok(Box::new(decoder))
        }
        _ => Err(FetchError::Unsupported(format!(
            "ZIP compression {method:?}"
        ))),
    }
}

pub(super) fn extract(
    file: &mut std::fs::File,
    dest: &Path,
    limits: FetchLimits,
) -> Result<(), FetchError> {
    metadata_bounds(file, limits)?;
    let mut archive = ::zip::ZipArchive::new(file)?;
    if archive.len() > limits.archive_entries.get() {
        return Err(FetchError::TooManyEntries {
            limit: limits.archive_entries.get(),
        });
    }
    let mut graph = Graph::new(limits);
    let mut budget = Budget::new(limits);
    for index in 0..archive.len() {
        budget.entry()?;
        let raw = archive.by_index_raw(index)?;
        if raw.encrypted() {
            return Err(paths::violation(
                Path::new(raw.name()),
                "encrypted ZIP entries are unsupported",
            ));
        }
        if raw.name().contains('\\') {
            return Err(paths::violation(
                Path::new(raw.name()),
                "ambiguous backslash path",
            ));
        }
        let relative = paths::relative(Path::new(raw.name()))?;
        let is_dir = raw.is_dir();
        let is_link = raw.is_symlink();
        let mode = raw
            .unix_mode()
            .unwrap_or(if is_dir { 0o755 } else { 0o644 });
        let kind = mode & 0o170000;
        if !matches!(kind, 0 | 0o100000 | 0o040000 | 0o120000)
            || (is_dir && matches!(kind, 0o100000 | 0o120000))
            || (is_link && matches!(kind, 0o100000 | 0o040000))
        {
            return Err(paths::violation(
                &relative,
                "special or ambiguous ZIP entry is unsupported",
            ));
        }
        if is_dir && raw.size() != 0 {
            return Err(paths::violation(&relative, "directory contains file data"));
        }
        if raw.size() > limits.expanded_bytes.get() {
            return Err(FetchError::PayloadTooLarge {
                what: "ZIP entry".into(),
                limit: limits.expanded_bytes.get(),
            });
        }
        let node = if is_dir {
            Kind::Directory { explicit: true }
        } else if is_link {
            if raw.size() > 4096 {
                return Err(paths::violation(&relative, "ZIP link target is too long"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                let size = raw.size();
                let crc = raw.crc32();
                let mut checksum = crc32fast::Hasher::new();
                let mut bytes = Vec::with_capacity(size as usize);
                let copied = budget.copy(
                    crate::spool::Limited::new(decoded(raw, limits)?, 4096, "ZIP link target"),
                    &mut bytes,
                    |part| checksum.update(part),
                )?;
                if copied != size || checksum.finalize() != crc {
                    return Err(paths::violation(
                        &relative,
                        "ZIP link size or CRC does not match decoded bytes",
                    ));
                }
                Kind::Symlink(std::path::PathBuf::from(std::ffi::OsString::from_vec(
                    bytes,
                )))
            }
            #[cfg(not(unix))]
            return Err(paths::violation(&relative, "symlinks require Unix"));
        } else {
            Kind::Regular
        };
        graph.insert(relative, node)?;
    }
    graph.validate()?;
    let root = paths::open_root(dest)?;
    // Materialize only regular content and real directories first. Every
    // link's CRC/target already passed the complete graph before the root
    // was opened; no later file write may traverse a created symlink.
    for index in 0..archive.len() {
        let raw = archive.by_index_raw(index)?;
        let relative = paths::relative(Path::new(raw.name()))?;
        if raw.is_dir() {
            paths::directories(&root, &relative)?;
            continue;
        }
        if raw.is_symlink() {
            continue;
        }
        let expected_size = raw.size();
        let expected_crc = raw.crc32();
        let mode = raw.unix_mode().unwrap_or(0o644);
        let mut checksum = crc32fast::Hasher::new();
        let mut output = paths::file(&root, &relative)?;
        let copied = budget.copy(decoded(raw, limits)?, &mut output, |part| {
            checksum.update(part)
        })?;
        if copied != expected_size || checksum.finalize() != expected_crc {
            return Err(paths::violation(
                &relative,
                "ZIP size or CRC does not match decoded bytes",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output.set_permissions(gripsack_fs::cap_std::fs::Permissions::from_std(
                std::fs::Permissions::from_mode(mode & 0o777),
            ))?;
        }
    }
    #[cfg(unix)]
    for index in 0..archive.len() {
        let raw = archive.by_index_raw(index)?;
        if raw.is_symlink() {
            let relative = paths::relative(Path::new(raw.name()))?;
            let target = graph
                .symlink_target(&relative)
                .ok_or_else(|| paths::violation(&relative, "validated ZIP link was lost"))?;
            paths::symlink(&root, &relative, target)?;
        }
    }
    Ok(())
}
