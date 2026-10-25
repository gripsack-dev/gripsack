//! Exchange undo returns every target node to its own retained stage before
//! restoring the original root and its hierarchy. Leaf priors replay afterward.
//!
//! Exchange is emitted only by retained-incarnation reactivation after Reverse,
//! not by new preparation (which emits Forward). Rollback supplies the target
//! receipt from an admitted retained generation. Its manifest remains authority
//! for every target staging inode after this journal is removed. Unlike new
//! prepared nodes, these historical nodes must survive aborted reactivation;
//! only generation retirement may discard them.
use super::directory_recovery::exact_link;
use super::{
    Entry,
    directory::{Direction, DirectoryIdentity, DirectoryRecord, invalid},
    directory_nodes as nodes,
};
use gripsack_fs::Dir;
use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) fn recover(
    records: &[(PathBuf, DirectoryRecord)],
    entries: &[(PathBuf, Entry)],
    home: &Dir,
) -> io::Result<()> {
    for (_, record) in records {
        let Direction::Exchange { original } = &record.direction else {
            continue;
        };
        let target = &record.transition;
        let parent = target.parent()?;
        let name = Path::new(
            target
                .destination
                .file_name()
                .ok_or_else(|| invalid("missing destination"))?,
        );
        match parent.symlink_metadata(name) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                let live = DirectoryIdentity::observe(&parent, name)?;
                if live == original.directories[Path::new("")] {
                    nodes::staged(target)?;
                    nodes::restore_all(original)?;
                    continue;
                }
                if live != target.directories[Path::new("")] {
                    return Err(invalid("foreign root prevents incarnation recovery"));
                }
                for (_, entry) in entries {
                    let destination = Path::new(&entry.dest);
                    let Ok(relative) = destination.strip_prefix(&target.destination) else {
                        continue;
                    };
                    if !target.leaves.iter().any(|leaf| leaf == relative) {
                        continue;
                    }
                    match target.leaf_parent(destination) {
                        Ok(_) => super::directory_shape::clear_entry(entry, target, home)?,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                nodes::retire_all(target)?;
            }
            Ok(_) if exact_link(&parent, name, &original.predecessor_target)? => {
                gripsack_fs::remove_file(&parent, name)?;
                gripsack_fs::fsync_dir(&parent, Path::new("."))?;
            }
            Ok(_) => {
                return Err(invalid(
                    "foreign parent replacement prevents directory incarnation recovery",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        nodes::staged(target)?;
        nodes::restore_all(original)?;
    }
    Ok(())
}
