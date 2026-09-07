use super::paths;
use crate::{FetchError, FetchLimits};
use std::collections::BTreeSet;
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub(super) fn extract(
    file: &mut std::fs::File,
    dest: &Path,
    limits: FetchLimits,
) -> Result<(), FetchError> {
    if file.metadata()?.len() > limits.expanded_bytes.get() {
        return Err(FetchError::PayloadTooLarge {
            what: "expanded tar stream".into(),
            limit: limits.expanded_bytes.get(),
        });
    }
    // Admit raw GNU/PAX bodies before the normal iterator allocates them.
    file.seek(SeekFrom::Start(0))?;
    for (index, entry) in ::tar::Archive::new(&mut *file)
        .entries_with_seek()?
        .raw(true)
        .enumerate()
    {
        let entry = entry?;
        if index >= limits.archive_entries.get() {
            return Err(FetchError::TooManyEntries {
                limit: limits.archive_entries.get(),
            });
        }
        let kind = entry.header().entry_type().as_byte();
        if matches!(kind, b'g' | b'x' | b'L' | b'K')
            && entry.size() > limits.decoder_bytes.get().min(64 * 1024)
        {
            return Err(FetchError::PayloadTooLarge {
                what: "tar metadata".into(),
                limit: limits.decoder_bytes.get().min(64 * 1024),
            });
        }
        if kind == b'S' {
            return Err(paths::violation(
                &entry.path()?,
                "GNU sparse entries are unsupported",
            ));
        }
    }
    file.seek(SeekFrom::Start(0))?;
    let mut links = BTreeSet::new();
    let mut metadata_bytes = 0u64;
    let metadata_limit = limits.decoder_bytes.get().min(64 * 1024 * 1024);
    for entry in ::tar::Archive::new(&mut *file).entries_with_seek()? {
        let entry = entry?;
        let kind = entry.header().entry_type().as_byte();
        if matches!(kind, b'g' | b'x' | b'L' | b'K') {
            continue;
        }
        if !matches!(kind, 0 | b'0' | b'1' | b'2' | b'5' | b'7') {
            return Err(paths::violation(
                &entry.path()?,
                "special tar entry is unsupported",
            ));
        }
        let path = paths::relative(&entry.path()?)?;
        if path.as_os_str().is_empty() && kind != b'5' {
            return Err(paths::violation(
                &path,
                "entry does not name a payload child",
            ));
        }
        paths::destination(dest, &path)?;
        if entry.size() > limits.expanded_bytes.get() {
            return Err(FetchError::PayloadTooLarge {
                what: "tar entry".into(),
                limit: limits.expanded_bytes.get(),
            });
        }
        // Bound retained path/set overhead, not just each individual name.
        metadata_bytes = metadata_bytes
            .checked_add(path.as_os_str().len() as u64 + 128)
            .filter(|bytes| *bytes <= metadata_limit)
            .ok_or_else(|| FetchError::PayloadTooLarge {
                what: "tar path metadata".into(),
                limit: metadata_limit,
            })?;
        if matches!(kind, b'1' | b'2') {
            let target = entry
                .link_name()?
                .ok_or_else(|| paths::violation(&path, "link has no target"))?;
            paths::link_target(&path, &target, kind == b'1')?;
            if kind == b'2' {
                links.insert(path);
            }
        }
    }
    // Check every logical name against the complete symlink set before writes.
    // Re-reading bounded metadata avoids retaining every entry and ancestor.
    file.seek(SeekFrom::Start(0))?;
    for entry in ::tar::Archive::new(&mut *file).entries_with_seek()? {
        let entry = entry?;
        let kind = entry.header().entry_type().as_byte();
        if matches!(kind, b'g' | b'x' | b'L' | b'K') {
            continue;
        }
        let path = paths::relative(&entry.path()?)?;
        check_ancestors(&path, &links)?;
        if kind == b'1' {
            let target = entry
                .link_name()?
                .ok_or_else(|| paths::violation(&path, "link has no target"))?;
            let target = paths::link_target(&path, &target, true)?;
            check_ancestors(&target, &links)?;
            if links.contains(&target) {
                return Err(paths::violation(&target, "hard link targets a symlink"));
            }
        }
    }
    file.seek(SeekFrom::Start(0))?;
    std::fs::create_dir_all(dest)?;
    let mut archive = ::tar::Archive::new(file);
    archive.set_preserve_permissions(false);
    // Archive::unpack retains directory entries for deferred chmod. Staging
    // never needs those modes: extract one entry, normalize directories now.
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type().as_byte();
        if matches!(kind, b'g' | b'x' | b'L' | b'K') {
            continue;
        }
        let path = paths::relative(&entry.path()?)?;
        paths::destination(dest, &path)?;
        if !entry.unpack_in(dest)? {
            return Err(paths::violation(
                &path,
                "entry was not materialized inside the payload",
            ));
        }
        #[cfg(unix)]
        if kind == b'5' {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dest.join(path), std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

fn check_ancestors(path: &Path, links: &BTreeSet<PathBuf>) -> Result<(), FetchError> {
    if path
        .ancestors()
        .skip(1)
        .any(|parent| links.contains(parent))
    {
        return Err(paths::violation(
            path,
            "archive content is nested beneath a symlink",
        ));
    }
    Ok(())
}
