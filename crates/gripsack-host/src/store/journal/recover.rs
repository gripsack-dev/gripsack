//! Operator-facing recovery notes and prior restoration for the delegated
//! directory steps of the journal recovery machine.

use super::PriorSerde;
use crate::fs::Dir;
use std::io;
use std::path::Path;

/// One recovery outcome from [`super::reconcile`], typed so callers render
/// severity instead of parsing message text: a kept post-crash edit
/// is a WARNING (your file was left in a state the manifest doesn't
/// know); restores and no-ops are informational.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryNote {
    pub severity: NoteSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteSeverity {
    Info,
    Warn,
}

impl std::fmt::Display for RecoveryNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub(super) fn restore(
    dest_dir: &Dir,
    dest_name: &Path,
    prior: &PriorSerde,
    home: &Dir,
) -> io::Result<()> {
    match prior {
        PriorSerde::Absent => {
            // remove_file never removes a directory; a dest that grew
            // into one errors here (ENOTDIR) and the journal entry is
            // retained — recovery data is never dropped on a failed
            // removal (0029 §4)
            match crate::fs::remove_file(dest_dir, dest_name) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            crate::fs::fsync_dir(dest_dir, Path::new("."))?;
        }
        PriorSerde::File { hash, mode } => {
            let bytes = crate::store::prior::read_blob(home, hash)?;
            // the mode rides the write (0027 §6): temp → exact mode →
            // fsync → rename, so a restored 0600 secret never exists
            // at a wider mode, not even for the rename's instant
            crate::fs::atomic_write_with_mode(dest_dir, dest_name, &bytes, mode.bits())?
        }
        PriorSerde::Symlink { target } => {
            crate::fs::symlink_replace(dest_dir, dest_name, Path::new(target))?;
        }
    }
    Ok(())
}

/// Seal an observed prior before cleanup, including a partially completed
/// restoration from an earlier process. Regular bytes need their own barrier;
/// absence/link identity needs the pinned parent-directory barrier.
pub(super) fn seal_observed_prior(
    dest_dir: &Dir,
    dest_name: &Path,
    prior: &PriorSerde,
) -> io::Result<()> {
    if matches!(prior, PriorSerde::File { .. }) {
        let file = crate::fs::open_file_nofollow(dest_dir, dest_name)?;
        crate::fs::fault::operation(crate::fs::fault::Boundary::FileSync, dest_name, || {
            file.sync_all()
        })?;
    }
    crate::fs::fsync_dir(dest_dir, Path::new("."))
}
