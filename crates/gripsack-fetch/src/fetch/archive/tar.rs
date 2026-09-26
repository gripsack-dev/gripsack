use super::{
    links::{Graph, Kind},
    paths,
    tree::Budget,
};
use crate::{FetchError, FetchLimits};
use std::io::{Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Copy)]
enum Pass {
    Files,
    HardLinks,
    Symlinks,
}

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
    let mut graph = Graph::new(limits);
    for entry in ::tar::Archive::new(&mut *file).entries_with_seek()? {
        let entry = entry?;
        let kind = entry.header().entry_type().as_byte();
        if matches!(kind, b'g' | b'x' | b'L' | b'K') {
            continue;
        }
        let path = paths::relative(&entry.path()?)?;
        if path.as_os_str().is_empty() && kind == b'5' {
            continue;
        }
        if entry.size() > limits.expanded_bytes.get() {
            return Err(FetchError::PayloadTooLarge {
                what: "tar entry".into(),
                limit: limits.expanded_bytes.get(),
            });
        }
        let node = match kind {
            0 | b'0' | b'7' => Kind::Regular,
            b'5' => Kind::Directory { explicit: true },
            b'1' | b'2' => {
                let target = entry
                    .link_name()?
                    .ok_or_else(|| paths::violation(&path, "link has no target"))?
                    .into_owned();
                if kind == b'1' {
                    Kind::HardLink(target)
                } else {
                    Kind::Symlink(target)
                }
            }
            _ => return Err(paths::violation(&path, "special tar entry is unsupported")),
        };
        graph.insert(path, node)?;
    }
    graph.validate()?;
    let root = paths::open_root(dest)?;
    let mut budget = Budget::new(limits);
    // Hard links may precede their regular targets. Symlinks are last so no
    // following operation can address an archive-created link as a parent.
    for pass in [Pass::Files, Pass::HardLinks, Pass::Symlinks] {
        file.seek(SeekFrom::Start(0))?;
        for entry in ::tar::Archive::new(&mut *file).entries()? {
            let mut entry = entry?;
            let kind = entry.header().entry_type().as_byte();
            if matches!(kind, b'g' | b'x' | b'L' | b'K') {
                continue;
            }
            let path = paths::relative(&entry.path()?)?;
            if path.as_os_str().is_empty() && kind == b'5' {
                continue;
            }
            match (pass, kind) {
                (Pass::Files, b'5') => {
                    paths::directories(&root, &path)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        root.set_permissions(
                            &path,
                            gripsack_fs::cap_std::fs::Permissions::from_std(
                                std::fs::Permissions::from_mode(0o755),
                            ),
                        )?;
                    }
                }
                (Pass::Files, 0 | b'0' | b'7') => {
                    let mut output = paths::file(&root, &path)?;
                    let copied = budget.copy(&mut entry, &mut output, |_| {})?;
                    if copied != entry.size() {
                        return Err(paths::violation(
                            &path,
                            "tar entry size changed during extraction",
                        ));
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        output.set_permissions(gripsack_fs::cap_std::fs::Permissions::from_std(
                            std::fs::Permissions::from_mode(entry.header().mode()? & 0o777),
                        ))?;
                    }
                }
                (Pass::HardLinks, b'1') => {
                    let target = entry
                        .link_name()?
                        .ok_or_else(|| paths::violation(&path, "hard link has no target"))?;
                    let target = paths::link_target(&path, &target, true)?;
                    paths::hard_link(&root, &path, &target)?;
                }
                (Pass::Symlinks, b'2') => {
                    let target = entry
                        .link_name()?
                        .ok_or_else(|| paths::violation(&path, "symlink has no target"))?;
                    #[cfg(unix)]
                    paths::symlink(&root, &path, &target)?;
                    #[cfg(not(unix))]
                    return Err(paths::violation(&path, "symlinks require Unix"));
                }
                _ => {}
            }
        }
    }
    Ok(())
}
