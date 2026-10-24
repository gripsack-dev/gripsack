//! Recovery observations distinguish absence, a recorded flat intermediate,
//! and an exact original inode. No unknown IO/type failure becomes absence.
use super::{
    Entry, Intended,
    directory::{DirectoryIdentity, DirectoryTransition, invalid},
    directory_nodes as nodes,
};
use gripsack_fs::Dir;
use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) fn staged(binding: &DirectoryTransition, relative: &Path) -> io::Result<bool> {
    let parent = binding.parent()?;
    let name = nodes::stage(binding, relative)?
        .file_name()
        .ok_or_else(|| invalid("missing staging name"))?;
    match parent.symlink_metadata(name) {
        Ok(_) => {
            nodes::empty(&parent, Path::new(name), binding.directories[relative])?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn installed(binding: &DirectoryTransition, relative: &Path) -> io::Result<bool> {
    let (parent, name) = match binding.leaf_parent(&binding.destination.join(relative)) {
        Ok(capability) => capability,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    match parent.symlink_metadata(&name) {
        Ok(_) => {
            binding.directories[relative].verify(&parent, &name)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn identity_restored(
    original: &DirectoryTransition,
    identity: DirectoryIdentity,
) -> io::Result<bool> {
    let Some((relative, _)) = original
        .directories
        .iter()
        .find(|(_, candidate)| **candidate == identity)
    else {
        return Ok(false);
    };
    installed(original, relative)
}

fn known_flat(
    parent: &Dir,
    name: &Path,
    absolute: &Path,
    entries: &[(PathBuf, Entry)],
    home: &Dir,
) -> io::Result<bool> {
    let Some((_, entry)) = entries
        .iter()
        .find(|(_, entry)| Path::new(&entry.dest) == absolute)
    else {
        return Ok(false);
    };
    let live = super::live_identity(parent, name)?;
    let intended = Intended::from_wire(&entry.after);
    if live
        .as_ref()
        .is_some_and(|live| intended.satisfied_by(live))
    {
        return Ok(true);
    }
    if live == super::prior_identity(&entry.prior, home)? {
        return Ok(true);
    }
    Ok(entry
        .before
        .as_ref()
        .map(|prior| super::prior_identity(prior, home))
        .transpose()?
        .is_some_and(|before| before == live))
}

/// Walk only admitted original nodes and supplemental borrowed witnesses. A
/// missing component or a recorded file/link intermediate is an explicit
/// namespace barrier; recovery must never walk through it into old payloads.
pub(super) fn original_namespace(
    original: &DirectoryTransition,
    target: &DirectoryTransition,
    relative: &Path,
    entries: &[(PathBuf, Entry)],
    home: &Dir,
) -> io::Result<bool> {
    let outer = original.parent()?;
    let root_name = Path::new(
        original
            .destination
            .file_name()
            .ok_or_else(|| invalid("missing root name"))?,
    );
    original.directories[Path::new("")].verify(&outer, root_name)?;
    let mut parent = gripsack_fs::open_dir_nofollow(&outer, root_name)?;
    let mut walked = PathBuf::new();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        walked.push(component.as_os_str());
        let name = Path::new(component.as_os_str());
        let metadata = match parent.symlink_metadata(name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            if known_flat(
                &parent,
                name,
                &original.destination.join(&walked),
                entries,
                home,
            )? {
                return Ok(true);
            }
            return Err(invalid(
                "foreign non-directory prevents original hierarchy recovery",
            ));
        }
        let identity = original
            .directories
            .get(&walked)
            .or_else(|| original.borrowed_directories.get(&walked))
            .or_else(|| target.borrowed_directories.get(&walked))
            .ok_or_else(|| invalid("foreign directory replaces an unowned original path"))?;
        identity.verify(&parent, name)?;
        if components.peek().is_none() {
            return Ok(true);
        }
        parent = gripsack_fs::open_dir_nofollow(&parent, name)?;
    }
    Ok(true)
}
