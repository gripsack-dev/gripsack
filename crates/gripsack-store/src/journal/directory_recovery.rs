//! Original directory shapes return before leaf priors; new forward shapes
//! retire only after their owned leaves are undone. All unknown objects survive.
use super::{
    directory::{Direction, DirectoryRecord, invalid},
    directory_nodes as nodes,
};
use gripsack_fs::Dir;
use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) fn absent(parent: &Dir, name: &Path) -> io::Result<bool> {
    match parent.symlink_metadata(name) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

pub(super) fn exact_link(parent: &Dir, name: &Path, target: &Path) -> io::Result<bool> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Ok(parent.read_link_contents(name)?.as_os_str() == target.as_os_str())
        }
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn unreachable_leaf(
    records: &[(PathBuf, DirectoryRecord)],
    destination: &Path,
    prior: &super::PriorSerde,
    home: &Dir,
    entries: &[(PathBuf, super::Entry)],
) -> io::Result<bool> {
    for (_, record) in records {
        let transition = &record.transition;
        let Ok(relative) = destination.strip_prefix(&transition.destination) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        if let Direction::Evolve { original } | Direction::Exchange { original } = &record.direction
        {
            if let Some(identity) = original.directories.get(relative) {
                if !matches!(prior, super::PriorSerde::Absent) {
                    return Err(invalid(
                        "directory restoration conflicts with a non-absent flat prior",
                    ));
                }
                let (parent, name) = original.leaf_parent(destination)?;
                identity.verify(&parent, &name)?;
                return Ok(true);
            }
            let parent = relative.parent().unwrap_or(Path::new(""));
            if original.directories.contains_key(parent)
                || original.borrowed_directories.contains_key(parent)
                || transition.borrowed_directories.contains_key(parent)
            {
                return Ok(false);
            }
            if !matches!(prior, super::PriorSerde::Absent) {
                return Err(invalid("unreachable evolved leaf has a non-absent prior"));
            }
            super::directory_shape_observe::original_namespace(
                original, transition, parent, entries, home,
            )?;
            return Ok(true);
        }
        if !matches!(
            record.direction,
            Direction::Forward | Direction::Reactivate { .. }
        ) {
            continue;
        }
        let parent = transition.parent()?;
        let name = Path::new(
            transition
                .destination
                .file_name()
                .ok_or_else(|| invalid("missing destination"))?,
        );
        if absent(&parent, name)? || matches_origin(record, &parent, name, home)? {
            if !matches!(prior, super::PriorSerde::Absent) {
                return Err(invalid("unreachable directory leaf has a non-absent prior"));
            }
            gripsack_fs::fsync_dir(&parent, Path::new("."))?;
            return Ok(true);
        }
        match transition.leaf_parent(destination) {
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && matches!(prior, super::PriorSerde::Absent) =>
            {
                return Ok(true);
            }
            Err(error) => return Err(error),
            Ok(_) => {}
        }
    }
    Ok(false)
}

pub(super) fn recover_before_leaves(
    records: &[(PathBuf, DirectoryRecord)],
    entries: &[(PathBuf, super::Entry)],
    home: &Dir,
    retain_prepared: bool,
) -> io::Result<()> {
    super::directory_exchange::recover(records, entries, home)?;
    for (_, record) in records {
        if let Direction::Evolve { original } = &record.direction {
            super::directory_shape::recover(
                original,
                &record.transition,
                entries,
                home,
                retain_prepared,
            )?;
            continue;
        }
        if record.direction != Direction::Reverse {
            continue;
        }
        let transition = &record.transition;
        let parent = transition.parent()?;
        let name = Path::new(
            transition
                .destination
                .file_name()
                .ok_or_else(|| invalid("missing destination"))?,
        );
        match parent.symlink_metadata(name) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                transition.directories[Path::new("")].verify(&parent, name)?;
            }
            Ok(_) if exact_link(&parent, name, &transition.predecessor_target)? => {
                gripsack_fs::remove_file(&parent, name)?;
                gripsack_fs::fsync_dir(&parent, Path::new("."))?;
            }
            Ok(_) => {
                return Err(invalid(
                    "foreign root replacement prevents original directory restoration",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        nodes::restore_all(transition)?;
    }
    Ok(())
}

pub(super) fn recover_after_leaves(
    records: &[(PathBuf, DirectoryRecord)],
    home: &Dir,
    retain_prepared: bool,
) -> io::Result<()> {
    for (_, record) in records.iter().rev() {
        if !matches!(
            record.direction,
            Direction::Forward | Direction::Reactivate { .. }
        ) {
            continue;
        }
        let transition = &record.transition;
        let parent = transition.parent()?;
        let name = Path::new(
            transition
                .destination
                .file_name()
                .ok_or_else(|| invalid("missing destination"))?,
        );
        let retain = retain_prepared || matches!(record.direction, Direction::Reactivate { .. });
        let original_visible = matches_origin(record, &parent, name, home)?;
        if !original_visible && !absent(&parent, name)? {
            transition.directories[Path::new("")].verify(&parent, name)?;
            for relative in nodes::ordered(transition, true) {
                match nodes::retire(transition, relative) {
                    Ok(()) => {}
                    Err(error) if !retain && error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        if !original_visible {
            match &record.direction {
                Direction::Reactivate { prior } => {
                    super::recover::restore(&parent, name, prior, home)?
                }
                _ => gripsack_fs::symlink_replace(&parent, name, &transition.predecessor_target)?,
            }
        }
        if retain {
            nodes::staged(transition)?;
        } else {
            for relative in transition.directories.keys() {
                nodes::discard(transition, relative)?;
            }
        }
        if let Direction::Reactivate { prior } = &record.direction {
            super::recover::seal_observed_prior(&parent, name, prior)?;
        }
        gripsack_fs::fsync_dir(&parent, Path::new("."))?;
    }
    Ok(())
}

fn matches_origin(
    record: &DirectoryRecord,
    parent: &Dir,
    name: &Path,
    home: &Dir,
) -> io::Result<bool> {
    let Direction::Reactivate { prior } = &record.direction else {
        return exact_link(parent, name, &record.transition.predecessor_target);
    };
    if parent
        .symlink_metadata(name)
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return Ok(false);
    }
    Ok(super::live_identity(parent, name)? == super::prior_identity(prior, home)?)
}

pub(super) fn is_reverse_root(records: &[(PathBuf, DirectoryRecord)], destination: &Path) -> bool {
    records.iter().any(|(_, record)| {
        record.direction == Direction::Reverse && record.transition.destination == destination
    })
}

pub(super) fn recovered_reverse_root(
    records: &[(PathBuf, DirectoryRecord)],
    destination: &Path,
) -> io::Result<bool> {
    for (_, record) in records {
        if record.direction != Direction::Reverse || record.transition.destination != destination {
            continue;
        }
        let parent = record.transition.parent()?;
        let name = Path::new(
            destination
                .file_name()
                .ok_or_else(|| invalid("missing destination"))?,
        );
        match parent.symlink_metadata(name) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                record.transition.directories[Path::new("")].verify(&parent, name)?;
                gripsack_fs::fsync_dir(&parent, Path::new("."))?;
                return Ok(true);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}
