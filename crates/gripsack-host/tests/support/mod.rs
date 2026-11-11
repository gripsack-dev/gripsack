//! The shipped `gripsack_host::fs` primitives, restated as `Storage` operations in
//! the order the real code issues them, so `MemFs` can explore them. Each
//! helper names the function it mirrors; the `*Order` enums add the
//! calibration mutants (orders a past release shipped or a review feared).
#![allow(dead_code)] // each test crate uses a different subset

use gripsack_host::storage::{MemFs, Name, ObjectKind, Permissions, Storage};
use std::io;

pub fn name(text: &str) -> Name {
    Name::new(text).unwrap()
}

/// Walk `components` from `root`, never following symlinks.
pub fn walk<S: Storage>(fs: &S, root: &S::Dir, components: &[&str]) -> io::Result<S::Dir> {
    let mut dir = fs.open_dir(root, &name(components[0]))?;
    for component in &components[1..] {
        dir = fs.open_dir(&dir, &name(component))?;
    }
    Ok(dir)
}

fn open_path<S: Storage>(fs: &S, components: &[&str]) -> io::Result<S::Dir> {
    let root = fs.root()?;
    if components.is_empty() {
        Ok(root)
    } else {
        walk(fs, &root, components)
    }
}

/// How `create_dir_all` treats a directory that already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingDirs {
    /// Shipped: an existing name may be the remnant of a crashed mkdir, so
    /// it is sealed leaf-up to the root (`seal_existing_path`).
    Seal,
    /// Mutant: trust what is visible.
    Trust,
}

/// `gripsack_host::fs::create_dir_all`: every component durable before return.
pub fn create_dir_all<S: Storage>(
    fs: &S,
    components: &[&str],
    existing: ExistingDirs,
) -> io::Result<S::Dir> {
    if components.is_empty() {
        return fs.root();
    }
    match open_path(fs, components) {
        Ok(leaf) => {
            if existing == ExistingDirs::Seal {
                for depth in (0..=components.len()).rev() {
                    fs.sync_dir(&open_path(fs, &components[..depth])?)?;
                }
            }
            Ok(leaf)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let (last, parents) = components.split_last().unwrap();
            let parent = create_dir_all(fs, parents, existing)?;
            match fs.create_dir(&parent, &name(last), Permissions::new(0o755)) {
                Ok(()) => {}
                Err(error)
                    if error.kind() == io::ErrorKind::AlreadyExists
                        && fs.metadata(&parent, &name(last))?.kind == ObjectKind::Dir => {}
                Err(error) => return Err(error),
            }
            let leaf = fs.open_dir(&parent, &name(last))?;
            fs.sync_dir(&leaf)?;
            fs.sync_dir(&parent)?;
            Ok(leaf)
        }
        Err(error) => Err(error),
    }
}

/// Order of the staged-file publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrder {
    /// Shipped `StagedFileWrite`: stage, write, mode, fsync file, rename,
    /// fsync parent.
    Shipped,
    /// The pre-0041 EXDEV bug: mode changed after the file fsync.
    ModeAfterSync,
    /// Rename issued before the file fsync: dirty bytes published.
    RenameBeforeSync,
}

/// Creation mode of a staged temp: `0o666` under a `022` umask.
pub const STAGED: Permissions = Permissions::new(0o644);

/// `StagedFileWrite::with_mode(..).sync_file().publish().sync_parent()`
/// (`atomic_write_with_mode`, `Journal::write`, `atomic_copy_with_mode`).
pub fn atomic_write<S: Storage>(
    fs: &S,
    dir: &S::Dir,
    at: &str,
    bytes: &[u8],
    mode: Permissions,
    order: WriteOrder,
) -> io::Result<()> {
    // The real name carries pid + counter; a crashed process's leftover can
    // still collide, and `create_temp` replaces it (remove, retry).
    let temporary = name(&format!(".tmp-write-{at}"));
    let mut file = match fs.create_file(dir, &temporary, STAGED) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs.unlink(dir, &temporary)?;
            fs.create_file(dir, &temporary, STAGED)?
        }
        created => created?,
    };
    fs.write(&mut file, bytes)?;
    if order != WriteOrder::ModeAfterSync {
        fs.set_mode(&file, mode)?;
    }
    if order == WriteOrder::RenameBeforeSync {
        fs.rename(dir, &temporary, dir, &name(at))?;
    }
    fs.sync_file(&file)?;
    if order == WriteOrder::ModeAfterSync {
        fs.set_mode(&file, mode)?;
    }
    if order != WriteOrder::RenameBeforeSync {
        fs.rename(dir, &temporary, dir, &name(at))?;
    }
    fs.sync_dir(dir)
}

/// `gripsack_host::fs::symlink_replace`: temp link, rename over, fsync parent.
pub fn symlink_replace<S: Storage>(fs: &S, dir: &S::Dir, at: &str, target: &str) -> io::Result<()> {
    let temporary = name(&format!(".tmp-link-{at}"));
    match fs.unlink(dir, &temporary) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    fs.symlink(dir, &temporary, std::path::Path::new(target))?;
    fs.rename(dir, &temporary, dir, &name(at))?;
    fs.sync_dir(dir)
}

/// A regular file's bytes and mode at `components/file`, if reachable.
pub fn file_at(fs: &MemFs, components: &[&str], file: &str) -> Option<(Vec<u8>, Permissions)> {
    let dir = open_path(fs, components).ok()?;
    let meta = fs.metadata(&dir, &name(file)).ok()?;
    let bytes = fs.read(&dir, &name(file)).ok()?;
    Some((bytes, meta.permissions))
}
