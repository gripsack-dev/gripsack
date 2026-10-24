//! Directory shape undo precedes ordinary leaf-prior replay. Changed target
//! branches are cleared only by their journaled identities, never recursively.
use super::{
    Entry, Intended,
    directory::{DirectoryTransition, invalid},
    directory_nodes as nodes, directory_shape_observe as observe,
};
use gripsack_fs::Dir;
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
};

fn changed(before: &DirectoryTransition, after: &DirectoryTransition) -> BTreeSet<PathBuf> {
    before
        .directories
        .iter()
        .filter(|(path, identity)| after.directories.get(*path) != Some(*identity))
        .map(|(path, _)| path.clone())
        .collect()
}

fn affected(path: &Path, changes: &BTreeSet<PathBuf>) -> bool {
    path.ancestors().any(|ancestor| changes.contains(ancestor))
}

pub(super) fn apply(before: &DirectoryTransition, after: &DirectoryTransition) -> io::Result<()> {
    if before.destination != after.destination
        || before.directories[Path::new("")] != after.directories[Path::new("")]
    {
        return Err(invalid(
            "shape evolution cannot replace the root incarnation",
        ));
    }
    let retiring = changed(before, after);
    let installing = changed(after, before);
    for relative in nodes::ordered(before, true) {
        if affected(relative, &retiring) {
            nodes::retire(before, relative)?;
        }
    }
    for relative in nodes::ordered(after, false) {
        if affected(relative, &installing) || affected(relative, &retiring) {
            nodes::install(after, relative)?;
        }
    }
    Ok(())
}

pub(super) fn clear_entry(
    entry: &Entry,
    binding: &DirectoryTransition,
    home: &Dir,
) -> io::Result<()> {
    let (directory, name) = binding.leaf_parent(Path::new(&entry.dest))?;
    clear_in(entry, &directory, &name, home)
}

fn clear_in(entry: &Entry, directory: &Dir, name: &Path, home: &Dir) -> io::Result<()> {
    let Some(live) = super::live_identity(directory, name)? else {
        return gripsack_fs::fsync_dir(directory, Path::new("."));
    };
    let intended = Intended::from_wire(&entry.after);
    let before = entry
        .before
        .as_ref()
        .map(|prior| super::prior_identity(prior, home))
        .transpose()?
        .flatten();
    if matches!(intended, Intended::Removed)
        || (!intended.satisfied_by(&live) && before.as_ref() != Some(&live))
    {
        return Err(invalid("foreign child prevents directory shape recovery"));
    }
    gripsack_fs::remove_file(directory, name)?;
    gripsack_fs::fsync_dir(directory, Path::new("."))
}

fn target_live(
    original: &DirectoryTransition,
    target: &DirectoryTransition,
    relative: &Path,
    entries: &[(PathBuf, Entry)],
    home: &Dir,
    retain_prepared: bool,
) -> io::Result<bool> {
    if observe::staged(target, relative)? {
        return Ok(false);
    }
    let observed = observe::installed(target, relative);
    if matches!(observed, Ok(true)) {
        return Ok(true);
    }
    if observe::identity_restored(original, target.directories[relative])? {
        return Ok(false);
    }
    if !retain_prepared {
        observe::original_namespace(original, target, relative, entries, home)?;
        return Ok(false);
    }
    match observed {
        Err(error) => Err(error),
        _ => Err(invalid("retained target directory identity is unavailable")),
    }
}

fn clear_old_directory_name(
    original: &DirectoryTransition,
    target: &DirectoryTransition,
    relative: &Path,
    entry: &Entry,
    home: &Dir,
) -> io::Result<()> {
    let destination = original.destination.join(relative);
    let capability = match original.leaf_parent(&destination) {
        Ok(capability) => Some(capability),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match target.leaf_parent(&destination) {
                Ok(capability) => Some(capability),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            }
        }
        Err(error) => return Err(error),
    };
    let Some((parent, name)) = capability else {
        return Ok(());
    };
    match parent.symlink_metadata(&name) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            original.directories[relative].verify(&parent, &name)
        }
        Ok(_) => clear_in(entry, &parent, &name, home),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(super) fn recover(
    original: &DirectoryTransition,
    target: &DirectoryTransition,
    entries: &[(PathBuf, Entry)],
    home: &Dir,
    retain_prepared: bool,
) -> io::Result<()> {
    let target_changes = changed(target, original);
    let original_changes = changed(original, target);
    let roots: Vec<_> = target_changes
        .iter()
        .filter(|path| {
            !path
                .ancestors()
                .skip(1)
                .any(|ancestor| target_changes.contains(ancestor))
        })
        .collect();
    for relative in roots {
        if !target_live(original, target, relative, entries, home, retain_prepared)? {
            continue;
        }
        let scope = target.destination.join(relative);
        for (_, entry) in entries {
            let path = Path::new(&entry.dest);
            if path != scope
                && path.starts_with(&scope)
                && !target.directories.contains_key(
                    path.strip_prefix(&target.destination)
                        .map_err(|_| invalid("target leaf escaped scope"))?,
                )
            {
                clear_entry(entry, target, home)?;
            }
        }
    }
    for relative in nodes::ordered(target, true) {
        if affected(relative, &target_changes)
            && target_live(original, target, relative, entries, home, retain_prepared)?
        {
            nodes::retire(target, relative)?;
        }
    }
    // New file writes at old directory names normalize before the original
    // directory returns. Repeated recovery recognizes an already-restored ID.
    for relative in &original_changes {
        let destination = original.destination.join(relative);
        if let Some((_, entry)) = entries
            .iter()
            .find(|(_, entry)| Path::new(&entry.dest) == destination)
        {
            clear_old_directory_name(original, target, relative, entry, home)?;
        }
    }
    for relative in nodes::ordered(original, false) {
        if affected(relative, &original_changes) || affected(relative, &target_changes) {
            nodes::restore(original, relative)?;
        }
    }
    if !retain_prepared {
        for relative in &target_changes {
            if !original
                .directories
                .values()
                .any(|identity| target.directories[relative] == *identity)
            {
                nodes::discard(target, relative)?;
            }
        }
    }
    Ok(())
}
