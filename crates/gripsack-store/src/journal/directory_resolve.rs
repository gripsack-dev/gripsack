//! Once a root is inode-owned, descendants remain logical children of that
//! root. Ambient symlinks below it can never redirect writes into old payloads.
use super::{
    JournalRun,
    directory::{DirectoryTransition, invalid},
};
use gripsack_fs::Dir;
use std::{
    io,
    path::{Component, Path, PathBuf},
};

fn project<'a>(
    receipts: impl Iterator<Item = &'a DirectoryTransition>,
    destination: &str,
) -> io::Result<PathBuf> {
    let mut receipts = receipts.peekable();
    if receipts.peek().is_none() {
        return crate::canonical_dest(destination);
    }
    let expanded = crate::expand_home(destination);
    if expanded.to_str().is_none()
        || expanded
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(invalid(
            "directory destination must be an absolute normalized UTF-8 path",
        ));
    }
    let expanded = if expanded.is_absolute() {
        expanded
    } else {
        std::path::absolute(expanded)?
    };
    let expanded: PathBuf = expanded.components().collect();
    for receipt in receipts {
        if let Ok(relative) = expanded.strip_prefix(&receipt.destination) {
            return Ok(receipt.destination.join(relative));
        }
        for ancestor in expanded.ancestors() {
            let key = crate::canonical_dest(
                ancestor
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF-8 ancestor"))?,
            )?;
            if key == receipt.destination {
                return Ok(key.join(
                    expanded
                        .strip_prefix(ancestor)
                        .map_err(|_| invalid("ancestor escaped destination"))?,
                ));
            }
        }
    }
    crate::canonical_dest(destination)
}

pub fn canonical_destination(run: &JournalRun<'_>, destination: &str) -> io::Result<PathBuf> {
    let records = run
        .directories
        .read()
        .map_err(|_| invalid("directory authority lock poisoned"))?;
    project(records.iter().map(|record| &record.transition), destination)
}

pub fn canonical_for_receipts(
    receipts: &[DirectoryTransition],
    destination: &str,
) -> io::Result<PathBuf> {
    project(receipts.iter(), destination)
}

pub enum ParentInspection {
    Existing(Dir, PathBuf),
    Missing,
    Blocked(PathBuf),
}

/// Read-only prospective observation. Unknown real directories may be borrowed
/// by a later journaled plan; symlinks/files are reported, never followed.
pub fn inspect_parent(
    receipt: &DirectoryTransition,
    destination: &Path,
) -> io::Result<ParentInspection> {
    let relative = destination
        .strip_prefix(&receipt.destination)
        .map_err(|_| invalid("destination outside directory scope"))?;
    let outer = receipt.parent()?;
    let root_name = Path::new(
        receipt
            .destination
            .file_name()
            .ok_or_else(|| invalid("missing root name"))?,
    );
    let mut parent = receipt.directories[Path::new("")].open_verified(&outer, root_name)?;
    let mut walked = PathBuf::new();
    for component in relative.parent().unwrap_or(Path::new("")).components() {
        walked.push(component.as_os_str());
        let name = Path::new(component.as_os_str());
        if let Some(identity) = receipt
            .directories
            .get(&walked)
            .or_else(|| receipt.borrowed_directories.get(&walked))
        {
            parent = identity.open_verified(&parent, name)?;
        } else {
            match parent.symlink_metadata(name) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => return Ok(ParentInspection::Blocked(receipt.destination.join(&walked))),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(ParentInspection::Missing);
                }
                Err(error) => return Err(error),
            }
            parent = gripsack_fs::open_dir_nofollow(&parent, name)?;
        }
    }
    Ok(ParentInspection::Existing(
        parent,
        PathBuf::from(
            relative
                .file_name()
                .ok_or_else(|| invalid("missing leaf name"))?,
        ),
    ))
}
