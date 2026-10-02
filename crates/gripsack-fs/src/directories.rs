//! Directory birth is part of publication durability, not just file fsync.
use super::{Boundary, Dir, open, operation, parent_rel};
use std::{io, path::Path};

/// A real read-only directory descriptor, never Linux's unsyncable O_PATH
/// capability handle. Construction establishes the directory-only role.
struct SyncableDirectory(rustix::fd::OwnedFd);

impl SyncableDirectory {
    fn open(dir: &Dir, relative: &Path) -> io::Result<Self> {
        rustix::fs::openat(
            dir,
            relative,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map(Self)
        .map_err(io::Error::from)
    }

    fn synchronize(self, label: &Path) -> io::Result<()> {
        operation(Boundary::DirSync, label, || {
            rustix::fs::fsync(self.0).map_err(io::Error::from)
        })
    }
}

/// Seal directory metadata at a path relative to the pinned capability.
pub fn fsync_dir(dir: &Dir, relative: &Path) -> io::Result<()> {
    SyncableDirectory::open(dir, relative)?.synchronize(relative)
}

/// Seal this exact admitted directory, without resolving its display name.
/// `label` identifies the operation in diagnostics/fault traces, not authority.
pub fn fsync_pinned_dir(dir: &Dir, label: &Path) -> io::Result<()> {
    SyncableDirectory::open(dir, Path::new("."))?.synchronize(label)
}

/// Open a durably reachable ambient root. Existing names can be remnants of a
/// failed mkdir attempt, so observing them does not replace namespace sealing.
/// Resolve operator-selected aliases at this ambient boundary; later effects
/// retain the returned capability. Concurrent external namespace replacement
/// remains outside the portable observer/effect contract.
pub fn open_or_create(path: &Path) -> io::Result<Dir> {
    match open_existing_durable(path) {
        Ok(directory) => return Ok(directory),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "directory has no name"))?;
    let dir = open_or_create(parent)?;
    create_dir_all(&dir, Path::new(name))?;
    dir.open_dir(name)
}

pub(super) fn open_existing_durable(path: &Path) -> io::Result<Dir> {
    let resolved = path.canonicalize()?;
    let directory = open(&resolved)?;
    fsync_dir(&directory, Path::new("."))?;
    for ancestor in resolved.ancestors().skip(1) {
        let parent = open(ancestor)?;
        fsync_dir(&parent, Path::new("."))?;
    }
    Ok(directory)
}

/// Ensure every relative namespace component is durable before returning.
/// The input capability is the admitted root; `"."` creates no new name.
pub fn create_dir_all(dir: &Dir, rel: &Path) -> io::Result<()> {
    match dir.metadata(rel) {
        Ok(meta) if meta.is_dir() => return seal_existing_path(dir, rel),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "directory path is not a directory",
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let parent = parent_rel(rel);
    create_dir_all(dir, parent)?;
    operation(Boundary::Mkdir, rel, || match dir.create_dir(rel) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && dir.metadata(rel)?.is_dir() => Ok(()),
        Err(e) => Err(e),
    })?;
    fsync_dir(dir, rel)?;
    fsync_dir(dir, parent)
}

fn seal_existing_path(dir: &Dir, rel: &Path) -> io::Result<()> {
    if rel.as_os_str().is_empty() || rel == Path::new(".") {
        return Ok(());
    }
    let mut current = rel;
    loop {
        fsync_dir(dir, current)?;
        if current == Path::new(".") {
            return Ok(());
        }
        current = parent_rel(current);
    }
}

pub fn remove_file(dir: &Dir, path: &Path) -> io::Result<()> {
    operation(Boundary::Unlink, path, || dir.remove_file(path))
}

pub fn rename(from_dir: &Dir, from: &Path, to_dir: &Dir, to: &Path) -> io::Result<()> {
    operation(Boundary::TreePublish, to, || {
        from_dir.rename(from, to_dir, to)
    })
}
