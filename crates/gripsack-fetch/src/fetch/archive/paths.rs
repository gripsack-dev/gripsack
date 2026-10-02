use crate::FetchError;
use gripsack_fs::cap_std::fs::{Dir, File, OpenOptions};
use std::path::{Component, Path, PathBuf};

pub(super) fn violation(path: &Path, reason: &'static str) -> FetchError {
    FetchError::UnsafeArchive {
        entry: path.to_owned(),
        reason,
    }
}

pub(super) fn relative(path: &Path) -> Result<PathBuf, FetchError> {
    if path.as_os_str().len() > 4096 {
        return Err(violation(path, "name is too long"));
    }
    let mut normalized = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => normalized.push(name),
            Component::CurDir => {}
            Component::ParentDir => return Err(violation(path, "escapes via ..")),
            _ => return Err(violation(path, "is absolute, outside the payload")),
        }
    }
    Ok(normalized)
}

pub(super) fn link_target(path: &Path, target: &Path, hard: bool) -> Result<PathBuf, FetchError> {
    if target.as_os_str().len() > 4096 {
        return Err(violation(path, "link target is too long"));
    }
    let mut resolved = if hard {
        PathBuf::new()
    } else {
        path.parent().unwrap_or(Path::new("")).to_owned()
    };
    for part in target.components() {
        match part {
            Component::Normal(name) => resolved.push(name),
            Component::CurDir => {}
            Component::ParentDir if resolved.pop() => {}
            Component::ParentDir => return Err(violation(path, "link target escapes the payload")),
            _ => return Err(violation(path, "link target is absolute")),
        }
    }
    Ok(resolved)
}

/// No existing symlink may redirect extraction through an ancestor. An archive
/// can contain links, but must store content under the actual directory names.
pub(super) fn destination(root: &Path, relative: &Path) -> Result<PathBuf, FetchError> {
    let mut parent = root.to_owned();
    if let Some(parts) = relative.parent() {
        for part in parts.components() {
            parent.push(part);
            match std::fs::symlink_metadata(&parent) {
                Ok(meta) if meta.is_symlink() || !meta.is_dir() => {
                    return Err(violation(relative, "ancestor is not a real directory"));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    let target = root.join(relative);
    if std::fs::symlink_metadata(&target).is_ok_and(|meta| meta.is_symlink()) {
        return Err(violation(
            relative,
            "would replace a symlink through extraction",
        ));
    }
    Ok(target)
}

/// Pin the payload root to the inode reached through its trusted parent.
/// `O_NOFOLLOW` closes the final-component swap between metadata and open;
/// descendants are always addressed relative to this capability.
pub(super) fn open_root(dest: &Path) -> Result<Dir, FetchError> {
    open_root_impl(dest, true)
}

pub(super) fn open_existing_root(dest: &Path) -> Result<Dir, FetchError> {
    open_root_impl(dest, false)
}

fn open_root_impl(dest: &Path, create: bool) -> Result<Dir, FetchError> {
    let parent = dest
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = dest
        .file_name()
        .ok_or_else(|| violation(dest, "payload destination has no directory name"))?;
    let parent = if create {
        gripsack_fs::open_or_create(parent)?
    } else {
        gripsack_fs::open(parent)?
    };
    match parent.symlink_metadata(name) {
        Ok(meta) if meta.is_dir() && !meta.is_symlink() => {}
        Ok(_) => {
            return Err(violation(
                dest,
                "payload destination is not a real directory",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            parent.create_dir(name)?;
        }
        Err(error) => return Err(error.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use gripsack_fs::cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    }
    let file = parent.open_with(name, &options)?;
    if !file.metadata()?.is_dir() {
        return Err(violation(dest, "payload destination is not a directory"));
    }
    Ok(Dir::from_std_file(file.into_std()))
}

/// Existing parents must be actual directories. Never traverse an archive
/// symlink even if cap-std would keep it inside this pinned root.
pub(super) fn directories(root: &Dir, relative: &Path) -> Result<(), FetchError> {
    let mut cursor = PathBuf::new();
    for part in relative.components() {
        let Component::Normal(name) = part else {
            return Err(violation(
                relative,
                "directory path is not relative and normalized",
            ));
        };
        cursor.push(name);
        match root.symlink_metadata(&cursor) {
            Ok(meta) if meta.is_dir() && !meta.is_symlink() => {}
            Ok(_) => {
                return Err(violation(
                    &cursor,
                    "payload ancestor is not a real directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                root.create_dir(&cursor)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn parents(root: &Dir, relative: &Path) -> Result<(), FetchError> {
    directories(root, relative.parent().unwrap_or(Path::new("")))
}

pub(super) fn file(root: &Dir, relative: &Path) -> Result<File, FetchError> {
    parents(root, relative)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    match root.open_with(relative, &options) {
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(violation(
            relative,
            "payload file would replace an existing name",
        )),
        result => result.map_err(FetchError::from),
    }
}

#[cfg(unix)]
pub(super) fn symlink(root: &Dir, relative: &Path, target: &Path) -> Result<(), FetchError> {
    parents(root, relative)?;
    match root.symlink_contents(target, relative) {
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(violation(
            relative,
            "payload link would replace an existing name",
        )),
        result => result.map_err(FetchError::from),
    }
}

pub(super) fn hard_link(root: &Dir, relative: &Path, target: &Path) -> Result<(), FetchError> {
    parents(root, relative)?;
    match root.hard_link(target, root, relative) {
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(violation(
            relative,
            "payload hard link would replace an existing name",
        )),
        result => result.map_err(FetchError::from),
    }
}
