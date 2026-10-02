//! One pinned directory owns journal IO, including quarantine and cleanup.
use crate::private_state;
use gripsack_fs::Dir;
use gripsack_policy::journal_protocol::{
    DurableRecord, PublicationAction, RecordPublication, RecordRole,
};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(super) const RUN_MARKER: &str = "run.json";

#[derive(Debug)]
pub(super) struct Journal {
    pub(super) directory: Dir,
}

impl Journal {
    /// Read-only opening; GC admission must not chmod or create metadata.
    pub(super) fn open(home: &Dir) -> io::Result<Option<Self>> {
        match gripsack_fs::open_dir_nofollow(home, Path::new("journal")) {
            Ok(directory) => Ok(Some(Self { directory })),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(super) fn prepare(home: &Dir) -> io::Result<Self> {
        Ok(Self {
            directory: private_state::ensure_directory(home, Path::new("journal"))?,
        })
    }

    pub(super) fn restrict(&self) -> io::Result<()> {
        private_state::restrict_directory(&self.directory, Path::new("journal"))
    }

    pub(super) fn read(&self, name: &Path) -> io::Result<Vec<u8>> {
        let mut file = gripsack_fs::open_file_nofollow(&self.directory, name)?;
        private_state::restrict_file(&file, name)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub(super) fn write(
        &self,
        name: &Path,
        bytes: &[u8],
        role: RecordRole,
    ) -> io::Result<DurableRecord> {
        let mut publication = RecordPublication::new(role);
        let staged = publication_step(&mut publication, PublicationAction::Stage, || {
            gripsack_fs::StagedFileWrite::with_mode(&self.directory, name, bytes, 0o600)
        })?;
        let synced = publication_step(&mut publication, PublicationAction::SyncFile, || {
            staged.sync_file()
        })?;
        let visible = publication_step(&mut publication, PublicationAction::PublishName, || {
            synced.publish()
        })?;
        let _durable =
            publication_step(&mut publication, PublicationAction::SyncNamespace, || {
                visible.sync_parent()
            })?;
        publication.finish().ok_or_else(publication_order_error)
    }

    pub(super) fn quarantine_directory(&self) -> io::Result<Option<Dir>> {
        match gripsack_fs::open_dir_nofollow(&self.directory, Path::new("quarantine")) {
            Ok(directory) => Ok(Some(directory)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Legacy quarantine files are made private without interpreting them.
    /// A nonempty quarantine remains a recovery blocker, not discarded state.
    pub(super) fn secure_quarantine(&self) -> io::Result<usize> {
        let Some(directory) = self.quarantine_directory()? else {
            return Ok(0);
        };
        private_state::restrict_directory(&directory, Path::new("journal/quarantine"))?;
        let mut count = 0;
        for entry in directory.read_dir(".")? {
            let name = PathBuf::from(entry?.file_name());
            if directory.symlink_metadata(&name)?.is_file() {
                let file = gripsack_fs::open_file_nofollow(&directory, &name)?;
                private_state::restrict_file(&file, &name)?;
            }
            count += 1;
        }
        Ok(count)
    }

    pub(super) fn quarantine(&self, name: &Path) -> io::Result<()> {
        let quarantine = private_state::ensure_directory(&self.directory, Path::new("quarantine"))?;
        gripsack_fs::rename(&self.directory, name, &quarantine, name)?;
        gripsack_fs::fsync_dir(&quarantine, Path::new("."))?;
        gripsack_fs::fsync_dir(&self.directory, Path::new("."))
    }
}

fn publication_order_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "journal publication effect order is invalid",
    )
}

fn publication_step<T>(
    publication: &mut RecordPublication,
    action: PublicationAction,
    effect: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    if publication.action() != action {
        return Err(publication_order_error());
    }
    match effect() {
        Ok(value) => {
            if publication.acknowledge(action, true) {
                Ok(value)
            } else {
                Err(publication_order_error())
            }
        }
        Err(error) => {
            publication.acknowledge(action, false);
            Err(error)
        }
    }
}
