//! Directory birth is part of publication durability, not just file fsync.
use super::{Boundary, Dir, fsync_dir, open, operation, parent_rel};
use std::{io, path::Path};

pub fn open_or_create(path: &Path) -> io::Result<Dir> {
    match open(path) {
        Ok(dir) => return Ok(dir),
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

pub fn create_dir_all(dir: &Dir, rel: &Path) -> io::Result<()> {
    match dir.metadata(rel) {
        Ok(meta) if meta.is_dir() => return Ok(()),
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

pub fn remove_file(dir: &Dir, path: &Path) -> io::Result<()> {
    operation(Boundary::Unlink, path, || dir.remove_file(path))
}

pub fn rename(from_dir: &Dir, from: &Path, to_dir: &Dir, to: &Path) -> io::Result<()> {
    operation(Boundary::TreePublish, to, || {
        from_dir.rename(from, to_dir, to)
    })
}
