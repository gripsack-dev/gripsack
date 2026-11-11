//! The collectable entries: every child of `store/` and `prior/`, and every
//! stale `generations/.staging-*`. Inventory and removal share the same pinned
//! directory capabilities; path strings are identities and reporting only,
//! never ambient deletion authority.
use super::utf8_path;
use crate::ExecError;
use gripsack_host::fs::cap_std::fs::{Metadata, MetadataExt as _};
use gripsack_host::fs::{Dir, open_dir_nofollow};
use gripsack_host::store;
use gripsack_kernel::retention::{Candidate, Scope, Stamp};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

struct Entry {
    scope: Scope,
    name: OsString,
    path: PathBuf,
    stamp: Stamp,
}

pub(super) struct Collectable {
    store: Option<Dir>,
    prior: Option<Dir>,
    generations: Option<Dir>,
    entries: Vec<Entry>,
}

impl Collectable {
    pub(super) fn open(home: &Dir, home_path: &Path) -> Result<Self, ExecError> {
        let mut entries = Vec::new();
        let store = list(
            home,
            home_path,
            store::STORE_DIR,
            Scope::Store,
            &mut entries,
        )?;
        let prior = list(home, home_path, "prior", Scope::Prior, &mut entries)?;
        let generations = list(
            home,
            home_path,
            store::GENERATIONS_DIR,
            Scope::Staging,
            &mut entries,
        )?;
        Ok(Self {
            store,
            prior,
            generations,
            entries,
        })
    }

    /// The candidates as the kernel sees them, in inventory order.
    pub(super) fn candidates(&self) -> Result<Vec<Candidate>, ExecError> {
        self.entries
            .iter()
            .map(|entry| {
                Ok(Candidate {
                    scope: entry.scope,
                    path: utf8_path(&entry.path)?.into_bytes(),
                    stamp: entry.stamp,
                })
            })
            .collect()
    }

    pub(super) fn scope(&self, candidate: usize) -> Scope {
        self.entries[candidate].scope
    }

    pub(super) fn path(&self, candidate: usize) -> &Path {
        &self.entries[candidate].path
    }

    fn directory(&self, scope: Scope) -> io::Result<&Dir> {
        match scope {
            Scope::Store => self.store.as_ref(),
            Scope::Prior => self.prior.as_ref(),
            Scope::Staging => self.generations.as_ref(),
        }
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    /// Bytes the candidate holds (a directory recursively, a symlink as 0).
    pub(super) fn size(&self, candidate: usize) -> io::Result<u64> {
        let entry = &self.entries[candidate];
        object_size(self.directory(entry.scope)?, Path::new(&entry.name))
    }

    /// Remove the candidate's name after re-checking that it still holds the
    /// inventoried object. `Ok(false)`: it does not, and nothing was touched.
    pub(super) fn remove(&self, candidate: usize, expect: Stamp) -> io::Result<bool> {
        let entry = &self.entries[candidate];
        let directory = self.directory(entry.scope)?;
        let metadata = directory.symlink_metadata(&entry.name)?;
        if stamp(&metadata) != expect {
            return Ok(false);
        }
        if metadata.is_dir() {
            directory.remove_dir_all(&entry.name)?;
        } else {
            // Includes symlinks: unlink the entry, never its target.
            gripsack_host::fs::remove_file(directory, Path::new(&entry.name))?;
        }
        Ok(true)
    }
}

fn stamp(metadata: &Metadata) -> Stamp {
    Stamp {
        filesystem: metadata.dev(),
        inode: metadata.ino(),
    }
}

/// Inventory one directory's collectable children; `generations/` offers only
/// its canonical staging names.
fn list(
    home: &Dir,
    home_path: &Path,
    name: &str,
    scope: Scope,
    entries: &mut Vec<Entry>,
) -> Result<Option<Dir>, ExecError> {
    let directory = match open_dir_nofollow(home, Path::new(name)) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let root = home_path.join(name);
    for entry in directory.entries()? {
        let child = entry?.file_name();
        let path = root.join(&child);
        // fail closed (0046): a non-UTF-8 name cannot be compared honestly
        utf8_path(&path)?;
        if matches!(scope, Scope::Staging)
            && !child
                .to_str()
                .is_some_and(store::generations::is_staging_name)
        {
            continue;
        }
        let metadata = directory.symlink_metadata(&child)?;
        entries.push(Entry {
            scope,
            name: child,
            path,
            stamp: stamp(&metadata),
        });
    }
    Ok(Some(directory))
}

fn add_size(total: u64, size: u64) -> io::Result<u64> {
    total
        .checked_add(size)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "GC byte count overflow"))
}

fn object_size(parent: &Dir, name: &Path) -> io::Result<u64> {
    let metadata = parent.symlink_metadata(name)?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    // Refuse a directory replaced by a symlink between metadata and open.
    let directory = open_dir_nofollow(parent, name)?;
    let mut total = 0;
    for entry in directory.entries()? {
        total = add_size(
            total,
            object_size(&directory, Path::new(&entry?.file_name()))?,
        )?;
    }
    Ok(total)
}

/// Sum candidate sizes, refusing overflow.
pub(super) fn total(sizes: impl IntoIterator<Item = io::Result<u64>>) -> io::Result<u64> {
    sizes
        .into_iter()
        .try_fold(0, |sum, size| add_size(sum, size?))
}
