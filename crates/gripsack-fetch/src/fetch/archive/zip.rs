use super::{paths, tree::Budget};
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
    std::fs::create_dir_all(dest)?;
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
        let target = paths::destination(dest, &relative)?;
        let is_dir = raw.is_dir();
        let is_link = raw.is_symlink();
        let mode = raw
            .unix_mode()
            .unwrap_or(if is_dir { 0o755 } else { 0o644 });
        let kind = mode & 0o170000;
        if !matches!(kind, 0 | 0o100000 | 0o040000 | 0o120000) {
            return Err(paths::violation(
                &relative,
                "special ZIP entry is unsupported",
            ));
        }
        if is_dir {
            if raw.size() != 0 {
                return Err(paths::violation(&relative, "directory contains file data"));
            }
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if relative.as_os_str().is_empty() {
            return Err(paths::violation(
                &relative,
                "entry does not name a payload child",
            ));
        }
        if raw.size() > limits.expanded_bytes.get() {
            return Err(FetchError::PayloadTooLarge {
                what: "ZIP entry".into(),
                limit: limits.expanded_bytes.get(),
            });
        }
        let expected_size = raw.size();
        let expected_crc = raw.crc32();
        let method = raw.compression();
        let reader: Box<dyn Read + '_> = match method {
            ::zip::CompressionMethod::Stored => Box::new(raw),
            ::zip::CompressionMethod::Deflated => Box::new(flate2::read::DeflateDecoder::new(raw)),
            ::zip::CompressionMethod::Bzip2 => {
                if limits.decoder_bytes.get() < 8 * 1024 * 1024 {
                    return Err(FetchError::PayloadTooLarge {
                        what: "bzip2 decoder memory".into(),
                        limit: limits.decoder_bytes.get(),
                    });
                }
                Box::new(bzip2::read::BzDecoder::new(raw))
            }
            ::zip::CompressionMethod::Zstd => {
                let mut decoder = zstd::stream::read::Decoder::new(raw)?;
                decoder.window_log_max(limits.decoder_bytes.get().ilog2().min(31))?;
                Box::new(decoder)
            }
            _ => {
                return Err(FetchError::Unsupported(format!(
                    "ZIP compression {method:?}"
                )));
            }
        };
        let mut crc = crc32fast::Hasher::new();
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let size = if is_link {
            let mut bytes = Vec::new();
            let size = budget.copy(
                crate::spool::Limited::new(reader, 4096, "ZIP link target"),
                &mut bytes,
                |bytes| crc.update(bytes),
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                let link = std::path::PathBuf::from(std::ffi::OsString::from_vec(bytes));
                let resolved = paths::link_target(&relative, &link, false)?;
                paths::destination(dest, &resolved)?;
                std::os::unix::fs::symlink(link, &target)?;
            }
            #[cfg(not(unix))]
            return Err(paths::violation(&relative, "symlinks require Unix"));
            size
        } else {
            let output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            let size = budget.copy(reader, output, |bytes| crc.update(bytes))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode & 0o777))?;
            }
            size
        };
        if size != expected_size || crc.finalize() != expected_crc {
            return Err(paths::violation(
                &relative,
                "ZIP size or CRC does not match decoded bytes",
            ));
        }
    }
    Ok(())
}
