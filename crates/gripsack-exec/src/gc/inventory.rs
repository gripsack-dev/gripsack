//! Collection admission and effects share the same directory capability.
//! Path strings are identities/reporting only, never ambient deletion authority.
use super::{GcReport, utf8_path};
use crate::ExecError;
use gripsack_fs::{Dir, open_dir_nofollow};
use std::{io, path::Path};

pub(super) struct ObjectInventory {
    directory: Option<Dir>,
    candidates: Vec<String>,
}

pub(super) struct CollectionPlan<'a> {
    directory: Option<&'a Dir>,
    paths: Vec<&'a str>,
    bytes: u64,
}

impl ObjectInventory {
    pub(super) fn open(home: &Dir, home_path: &Path, name: &str) -> Result<Self, ExecError> {
        let directory = match open_dir_nofollow(home, Path::new(name)) {
            Ok(directory) => Some(directory),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let mut candidates = Vec::new();
        if let Some(directory) = &directory {
            let root = home_path.join(name);
            for entry in directory.entries()? {
                candidates.push(utf8_path(&root.join(entry?.file_name()))?);
            }
        }
        Ok(Self {
            directory,
            candidates,
        })
    }

    /// Finish all fallible inventory/size reads before pruning any generation.
    pub(super) fn plan(&self, roots: &[&str]) -> Result<CollectionPlan<'_>, ExecError> {
        let candidates: Vec<&str> = self.candidates.iter().map(String::as_str).collect();
        let paths = gripsack_policy::retention::plan_delete(roots, &candidates);
        let mut bytes = 0;
        if let Some(directory) = &self.directory {
            for path in &paths {
                bytes = add_size(bytes, object_size(directory, entry_name(path))?)?;
            }
        }
        Ok(CollectionPlan {
            directory: self.directory.as_ref(),
            paths,
            bytes,
        })
    }
}

impl CollectionPlan<'_> {
    pub(super) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(super) fn collect(self, dry_run: bool, report: &mut GcReport) -> io::Result<()> {
        if let Some(directory) = self.directory {
            for path in self.paths {
                if !dry_run {
                    let name = entry_name(path);
                    if directory.symlink_metadata(name)?.is_dir() {
                        directory.remove_dir_all(name)?;
                    } else {
                        // Includes symlinks: unlink the entry, never its target.
                        gripsack_fs::remove_file(directory, name)?;
                    }
                }
                report.store_removed.push(path.into());
            }
        }
        Ok(())
    }
}

fn entry_name(path: &str) -> &Path {
    Path::new(
        Path::new(path)
            .file_name()
            .expect("inventoried direct child"),
    )
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
