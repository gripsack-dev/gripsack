//! One atomic-file implementation with explicit file and namespace barriers.
use crate::{
    Boundary, Dir, create_dir_all, create_temp, fsync_dir, operation, parent_rel, remove_file,
    temp_name,
};
use std::io;
use std::path::{Path, PathBuf};

struct PendingWrite<'a> {
    directory: &'a Dir,
    name: &'a Path,
    temporary: PathBuf,
    file: cap_std::fs::File,
    published: bool,
}

impl Drop for PendingWrite<'_> {
    fn drop(&mut self) {
        if !self.published {
            // Only our successfully created staging file is disposable. Keep
            // the original write/sync error if best-effort removal also fails.
            let _ = remove_file(self.directory, &self.temporary);
        }
    }
}

/// Complete staged bytes/mode, not yet durable and not publishable.
#[must_use]
pub struct StagedFileWrite<'a>(PendingWrite<'a>);

/// The staged file's bytes and mode have passed their file barrier.
#[must_use]
pub struct SyncedFileWrite<'a>(PendingWrite<'a>);

/// The new name is visible; its parent-directory barrier is still required.
#[must_use]
pub struct VisibleFileWrite<'a> {
    directory: &'a Dir,
    name: &'a Path,
}

/// Both file and parent-directory barriers completed for this publication.
/// This is an IO receipt, not a proof of hardware/filesystem behavior.
#[must_use]
pub struct DurableFileWrite<'a> {
    directory: &'a Dir,
    name: &'a Path,
}

impl<'a> StagedFileWrite<'a> {
    pub fn preserving_mode(
        directory: &'a Dir,
        name: &'a Path,
        contents: &[u8],
    ) -> io::Result<Self> {
        Self::stage(directory, name, contents, |file| {
            crate::preserve_mode(directory, name, file)
        })
    }

    #[cfg(unix)]
    pub fn with_mode(
        directory: &'a Dir,
        name: &'a Path,
        contents: &[u8],
        mode: u32,
    ) -> io::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        Self::stage(directory, name, contents, |file| {
            file.set_permissions(cap_std::fs::Permissions::from_std(
                std::fs::Permissions::from_mode(mode),
            ))
        })
    }

    fn stage(
        directory: &'a Dir,
        name: &'a Path,
        contents: &[u8],
        set_mode: impl FnOnce(&cap_std::fs::File) -> io::Result<()>,
    ) -> io::Result<Self> {
        let parent = parent_rel(name);
        create_dir_all(directory, parent)?;
        let temporary = parent.join(temp_name("tmp-write", name));
        let file = create_temp(directory, &temporary)?;
        let mut pending = PendingWrite {
            directory,
            name,
            temporary,
            file,
            published: false,
        };
        operation(Boundary::Write, name, || {
            io::Write::write_all(&mut pending.file, contents)
        })?;
        operation(Boundary::Mode, name, || set_mode(&pending.file))?;
        Ok(Self(pending))
    }

    pub fn sync_file(self) -> io::Result<SyncedFileWrite<'a>> {
        operation(Boundary::FileSync, self.0.name, || self.0.file.sync_all())?;
        Ok(SyncedFileWrite(self.0))
    }
}

impl<'a> SyncedFileWrite<'a> {
    pub fn publish(mut self) -> io::Result<VisibleFileWrite<'a>> {
        operation(Boundary::FilePublish, self.0.name, || {
            self.0
                .directory
                .rename(&self.0.temporary, self.0.directory, self.0.name)
        })?;
        self.0.published = true;
        Ok(VisibleFileWrite {
            directory: self.0.directory,
            name: self.0.name,
        })
    }
}

impl<'a> VisibleFileWrite<'a> {
    pub fn sync_parent(self) -> io::Result<DurableFileWrite<'a>> {
        fsync_dir(self.directory, parent_rel(self.name))?;
        Ok(DurableFileWrite {
            directory: self.directory,
            name: self.name,
        })
    }
}

impl DurableFileWrite<'_> {
    pub fn directory(&self) -> &Dir {
        self.directory
    }
    pub fn name(&self) -> &Path {
        self.name
    }
}
