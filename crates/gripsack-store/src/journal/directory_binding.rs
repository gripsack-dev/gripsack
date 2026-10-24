//! Mutation and recovery pin the persisted logical hierarchy, not ambient links.
use super::directory_wire::{Direction, DirectoryRecord, persist};
use super::{
    JournalRun,
    directory::{DirectoryTransition, invalid},
};
use gripsack_fs::Dir;
use std::{
    io,
    path::{Path, PathBuf},
};

/// Bind existing generation ownership before any descendant mutation. This
/// record grants no permission to replace or remove the directory itself.
pub fn bind_existing(run: &JournalRun<'_>, transition: &DirectoryTransition) -> io::Result<()> {
    let parent = transition.parent()?;
    let name = Path::new(
        transition
            .destination
            .file_name()
            .ok_or_else(|| invalid("missing destination"))?,
    );
    transition.directories[Path::new("")].verify(&parent, name)?;
    let root = gripsack_fs::open_dir_nofollow(&parent, name)?;
    for relative in transition.directories.keys() {
        transition.open_relative(&root, relative)?;
    }
    {
        let records = run
            .directories
            .read()
            .map_err(|_| invalid("directory authority lock poisoned"))?;
        if let Some(record) = records
            .iter()
            .find(|record| record.transition.destination == transition.destination)
        {
            if record.transition == *transition {
                return Ok(());
            }
            return Err(invalid("directory authority changed during transaction"));
        }
    }
    persist(run, transition, Direction::Bound)
}

pub fn destination_capability(
    run: &JournalRun<'_>,
    destination: &Path,
) -> io::Result<Option<(Dir, PathBuf)>> {
    bound_parent(run, destination)
}

/// Child capture uses the same persisted hierarchy as crash recovery.
pub(super) fn bound_parent(
    run: &JournalRun<'_>,
    destination: &Path,
) -> io::Result<Option<(Dir, PathBuf)>> {
    let records = run
        .directories
        .read()
        .map_err(|_| invalid("directory authority lock poisoned"))?;
    for record in records.iter() {
        if destination == record.transition.destination {
            let parent = record.transition.parent()?;
            return Ok(Some((
                parent,
                PathBuf::from(
                    destination
                        .file_name()
                        .ok_or_else(|| invalid("missing directory destination"))?,
                ),
            )));
        }
        if destination != record.transition.destination
            && destination.starts_with(&record.transition.destination)
        {
            return record.transition.leaf_parent(destination).map(Some);
        }
    }
    Ok(None)
}

pub(super) fn recovery_parent(
    records: &[(PathBuf, DirectoryRecord)],
    destination: &Path,
) -> io::Result<Option<(Dir, PathBuf)>> {
    for (_, record) in records {
        let transition = match &record.direction {
            Direction::Exchange { original } | Direction::Evolve { original } => original,
            _ => &record.transition,
        };
        if destination == transition.destination {
            return Ok(Some((
                transition.parent()?,
                PathBuf::from(
                    destination
                        .file_name()
                        .ok_or_else(|| invalid("missing directory destination"))?,
                ),
            )));
        }
        if destination.starts_with(&transition.destination) {
            return transition
                .leaf_parent_with(destination, &record.transition.borrowed_directories)
                .map(Some);
        }
    }
    Ok(None)
}
