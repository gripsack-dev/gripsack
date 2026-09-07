use crate::FetchError;
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
