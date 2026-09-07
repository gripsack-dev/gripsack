//! Archive materialization from verified seekable spools (0042).
//! Decoder output is streamed to a private spool so metadata can be admitted
//! before tar's allocator or extraction sees it; payload size never becomes RAM.

mod paths;
mod pour;
mod tar;
#[cfg(test)]
mod tests;
mod tree;
mod zip;

use crate::{FetchError, FetchLimits};
pub(crate) use pour::pour;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
pub(crate) use tree::{copy_tree_filtered, validate_tree};

pub(crate) fn extract(
    file: &mut std::fs::File,
    dest: &Path,
    bare_name: &str,
    limits: FetchLimits,
) -> Result<(), FetchError> {
    file.seek(SeekFrom::Start(0))?;
    let mut magic = [0; 6];
    let read = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    if magic[..read].starts_with(b"PK\x03\x04") || magic[..read].starts_with(b"PK\x05\x06") {
        return zip::extract(file, dest, limits);
    }
    if magic[..read].starts_with(b"\xfd7zXZ\x00") {
        let stream = xz2::stream::Stream::new_stream_decoder(
            limits.decoder_bytes.get(),
            xz2::stream::CONCATENATED,
        )
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let decoder = xz2::read::XzDecoder::new_stream(file, stream);
        let mut expanded = crate::spool::expanded_spool(decoder, limits.expanded_bytes.get())?;
        return plain(
            expanded.as_file_mut(),
            dest,
            bare_name.strip_suffix(".xz").unwrap_or(bare_name),
            limits,
        );
    }
    if magic[..read].starts_with(&[0x1f, 0x8b]) {
        if limits.decoder_bytes.get() < 128 * 1024 {
            return Err(FetchError::PayloadTooLarge {
                what: "gzip decoder memory".into(),
                limit: limits.decoder_bytes.get(),
            });
        }
        let decoder = flate2::read::MultiGzDecoder::new(file);
        let mut expanded = crate::spool::expanded_spool(decoder, limits.expanded_bytes.get())?;
        return plain(
            expanded.as_file_mut(),
            dest,
            bare_name.strip_suffix(".gz").unwrap_or(bare_name),
            limits,
        );
    }
    plain(file, dest, bare_name, limits)
}

fn plain(
    file: &mut std::fs::File,
    dest: &Path,
    bare_name: &str,
    limits: FetchLimits,
) -> Result<(), FetchError> {
    file.seek(SeekFrom::Start(0))?;
    let mut buffer = [0; 512];
    let mut length = 0;
    while length < buffer.len() {
        match file.read(&mut buffer[length..]) {
            Ok(0) => break,
            Ok(read) => length += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let prefix = &buffer[..length];
    file.seek(SeekFrom::Start(0))?;
    let tar = prefix.len() > 262 && &prefix[257..262] == b"ustar";
    let empty_tar =
        bare_name.ends_with(".tar") && prefix.len() == 512 && prefix.iter().all(|byte| *byte == 0);
    if tar || empty_tar {
        return tar::extract(file, dest, limits);
    }
    let relative = paths::relative(Path::new(bare_name))?;
    if relative.components().count() != 1 {
        return Err(paths::violation(
            &relative,
            "bare payload needs a single filename",
        ));
    }
    let target = paths::destination(dest, &relative)?;
    std::fs::create_dir_all(dest)?;
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)?;
    tree::Budget::new(limits).copy(file, output, |_| {})?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable(prefix) { 0o755 } else { 0o644 };
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn executable(prefix: &[u8]) -> bool {
    prefix.starts_with(b"\x7fELF")
        || prefix.starts_with(b"#!")
        || [
            b"\xfe\xed\xfa\xce",
            b"\xce\xfa\xed\xfe",
            b"\xfe\xed\xfa\xcf",
            b"\xcf\xfa\xed\xfe",
            b"\xca\xfe\xba\xbe",
            b"\xbe\xba\xfe\xca",
            b"\xca\xfe\xba\xbf",
            b"\xbf\xba\xfe\xca",
        ]
        .iter()
        .any(|magic| prefix.starts_with(magic.as_slice()))
}
