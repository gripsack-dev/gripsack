//! Retained empty node identities survive rollback until receipt-aware GC.
use super::directory_recovery::absent;
use super::{
    JournalRun,
    directory::{self, Direction, DirectoryRecord, DirectoryTransition, invalid},
    directory_nodes as nodes,
};
use gripsack_fs::Dir;
use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) fn committed(records: &[(PathBuf, DirectoryRecord)]) -> io::Result<()> {
    for (_, record) in records {
        let transition = match &record.direction {
            Direction::Reverse => &record.transition,
            Direction::Exchange { original } => original,
            _ => continue,
        };
        nodes::staged(transition)?;
        gripsack_fs::fsync_dir(&transition.parent()?, Path::new("."))?;
    }
    Ok(())
}

/// Clear owned leaves first. The exact empty nodes then retire child-first,
/// each to its own retained name, before the predecessor link is restored.
pub fn reverse(run: &JournalRun<'_>, transition: &DirectoryTransition) -> io::Result<()> {
    let parent = transition.parent()?;
    let name = Path::new(
        transition
            .destination
            .file_name()
            .ok_or_else(|| invalid("missing destination"))?,
    );
    let stage = Path::new(
        transition
            .staging
            .file_name()
            .ok_or_else(|| invalid("missing staging"))?,
    );
    ensure_empty_tree(&parent, name, transition)?;
    if !absent(&parent, stage)? {
        return Err(invalid(
            "directory staging name occupied; preserving foreign object",
        ));
    }
    directory::persist(run, transition, Direction::Reverse)?;
    nodes::retire_all(transition)?;
    if !absent(&parent, name)? {
        return Err(invalid("destination replaced during directory reversal"));
    }
    gripsack_fs::symlink_replace(&parent, name, &transition.predecessor_target)?;
    gripsack_fs::fsync_dir(&parent, Path::new("."))
}

/// The caller has durably pruned candidate generations. Shared root or nested
/// identities survive whenever any retained receipt still references them.
pub fn retire_staging(
    transition: &DirectoryTransition,
    retained: &[&DirectoryTransition],
) -> io::Result<()> {
    for (relative, identity) in &transition.directories {
        if retained
            .iter()
            .any(|receipt| receipt.directories.values().any(|other| other == identity))
        {
            continue;
        }
        nodes::discard(transition, relative)?;
    }
    Ok(())
}

pub(super) fn ensure_empty_tree(
    parent: &Dir,
    name: &Path,
    transition: &DirectoryTransition,
) -> io::Result<()> {
    transition.directories[Path::new("")].verify(parent, name)?;
    let root = gripsack_fs::open_dir_nofollow(parent, name)?;
    for relative in transition.directories.keys() {
        let directory = transition.open_relative(&root, relative)?;
        for entry in directory.read_dir(".")? {
            let child = relative.join(entry?.file_name());
            if !transition.directories.contains_key(&child) {
                return Err(invalid(
                    "foreign or remaining child prevents directory reversal",
                ));
            }
        }
    }
    Ok(())
}
