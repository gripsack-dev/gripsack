//! The run marker and the commit classifier (0019, 0026 §4, 0028):
//! a run declares its target generation before any mutation, and
//! recovery classifies the interrupted run by EXACT transaction
//! identity.

use crate::{GenerationId, generations::SelectionReservation};
use gripsack_fs::Dir;
use gripsack_policy::journal_protocol::{
    CleanupAction, CleanupProgress, CleanupScope, DurableRecord, RecordRole,
};
use gripsack_policy::selection::SelectionIdentity;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) use super::marker_wire::RunMarker;
use super::storage::{Journal, RUN_MARKER};

/// A transaction-specific selection whose run marker is durably published.
/// New flips require this handle; generation equality alone is not authority.
#[derive(Debug)]
pub struct JournalRun<'a> {
    home: &'a Dir,
    home_path: &'a Path,
    pub(super) journal: Journal,
    pub(super) marker_record: DurableRecord,
    reservation: SelectionReservation,
}

impl JournalRun<'_> {
    pub fn identity(&self) -> &SelectionIdentity {
        self.reservation.identity()
    }

    pub fn home(&self) -> &Dir {
        self.home
    }

    pub fn home_path(&self) -> &Path {
        self.home_path
    }

    pub(crate) fn journal_directory(&self) -> &Dir {
        &self.journal.directory
    }

    pub(crate) fn target(&self) -> &Path {
        self.reservation.target()
    }
}

/// What the run is doing, retained for inspection rather than classification.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOp {
    /// The normal case: building the next generation.
    Apply,
    /// Returning to a previous generation.
    Rollback,
}

/// Declare the transaction before any destination mutation. The observed
/// predecessor must still select the caller's admitted generation; the newly
/// reserved target distinguishes even a same-generation rollback.
pub fn begin_run<'a>(
    home: &'a Dir,
    home_path: &'a Path,
    previous_generation: Option<GenerationId>,
    target_generation: GenerationId,
    op: RunOp,
) -> io::Result<JournalRun<'a>> {
    if super::pending_recovery(home)?.is_some() {
        return Err(io::Error::other(
            "unfinished journal must be reconciled before a new transaction",
        ));
    }
    let previous = crate::generations::current_selection_in(home_path, home)?;
    if previous.as_ref().map(SelectionIdentity::generation) != previous_generation {
        return Err(io::Error::other(
            "current selection changed before transaction admission",
        ));
    }
    let reservation = crate::generations::reserve_selection(home, target_generation)?;
    let marker = RunMarker::transaction(previous, *reservation.identity(), op)?;
    let journal = Journal::prepare(home)?;
    let marker_record = journal.write(
        Path::new(RUN_MARKER),
        serde_json::to_string(&marker)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
        RecordRole::RunMarker,
    )?;
    Ok(JournalRun {
        home,
        home_path,
        journal,
        marker_record,
        reservation,
    })
}

/// The run completed and the generation flipped: nothing left to
/// recover. Entries, stragglers, and the run marker are gone.
pub fn commit_run(committed: crate::CommittedSelection<'_>) -> io::Result<()> {
    let directory = committed.journal_directory();
    crate::private_state::restrict_directory(directory, Path::new("journal"))?;
    let entries = directory.read_dir(".")?;
    let mut entry_paths = Vec::new();
    for entry in entries {
        let name = entry?.file_name();
        let rel = PathBuf::from(&name);
        // the marker is NOT deleted here — cleanup deletes it last
        if name != RUN_MARKER && rel.extension().is_some_and(|e| e == "json") {
            entry_paths.push(rel);
        }
    }
    let progress = CleanupProgress::new(CleanupScope::Committed, entry_paths.len());
    cleanup(
        directory,
        entry_paths.iter().map(PathBuf::as_path),
        progress,
    )
}

/// Two durability barriers (0026 §5): entries deleted and fsync'd
/// FIRST, the marker deleted and fsync'd SECOND — so marker-durably-
/// gone implies entries-durably-gone. A single trailing fsync does
/// not order the deletions against a power loss; resurrected entries
/// with no marker read as an uncommitted run and would restore a
/// committed generation's priors (the 0.19.1 bug class, one level
/// down).
pub(super) fn cleanup<'a>(
    directory: &Dir,
    entry_paths: impl IntoIterator<Item = &'a Path>,
    mut progress: CleanupProgress,
) -> io::Result<()> {
    for (index, path) in entry_paths.into_iter().enumerate() {
        cleanup_step(&mut progress, CleanupAction::RemoveEntry { index }, || {
            gripsack_fs::remove_file(directory, path)
        })?;
    }
    cleanup_step(&mut progress, CleanupAction::SyncEntries, || {
        gripsack_fs::fsync_dir(directory, Path::new("."))
    })?;
    cleanup_step(
        &mut progress,
        CleanupAction::RemoveMarker,
        || match gripsack_fs::remove_file(directory, Path::new(RUN_MARKER)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
    )?;
    cleanup_step(&mut progress, CleanupAction::SyncMarker, || {
        gripsack_fs::fsync_dir(directory, Path::new("."))
    })?;
    if progress.action() != CleanupAction::Complete {
        return Err(cleanup_order_error());
    }
    Ok(())
}

pub(super) fn cleanup_order_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "journal cleanup effect order is invalid",
    )
}

fn cleanup_step(
    progress: &mut CleanupProgress,
    action: CleanupAction,
    effect: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    if progress.action() != action {
        return Err(cleanup_order_error());
    }
    match effect() {
        Ok(()) => {
            if progress.acknowledge(action, true) {
                Ok(())
            } else {
                Err(cleanup_order_error())
            }
        }
        Err(error) => {
            progress.acknowledge(action, false);
            Err(error)
        }
    }
}

/// The run ended without mutating anything (satisfied, empty graph):
/// the marker declared by `begin_run` must not linger — a stale
/// marker with no entries is harmless but noisy, and a marker whose
/// target generation is later than `current` would misread the NEXT
/// crash window.
pub fn end_run(run: JournalRun<'_>) -> io::Result<()> {
    run.journal.restrict()?;
    for entry in run.journal.directory.read_dir(".")? {
        let name = entry?.file_name();
        if name != RUN_MARKER
            && Path::new(&name)
                .extension()
                .is_some_and(|value| value == "json")
        {
            return Err(io::Error::other(
                "cannot end a journal run with outstanding destination entries",
            ));
        }
    }
    cleanup(
        &run.journal.directory,
        std::iter::empty(),
        CleanupProgress::new(CleanupScope::Uncommitted, 0),
    )
}

pub(super) fn run_marker(journal: &Journal) -> io::Result<Option<RunMarker>> {
    match journal.read(Path::new(RUN_MARKER)) {
        Ok(bytes) => serde_json::from_slice::<RunMarker>(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        // only NotFound means absent (same rule as capture): an
        // unreadable marker in RECOVERY code is commit evidence we
        // cannot see — error, never pick a branch blind (0025 §F)
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// One exact-selection kernel serves recovery, concrete models and Verus.
pub(crate) use gripsack_policy::{
    Classification,
    selection::{RecoveryFacts, classify},
};

#[cfg(test)]
#[path = "repeated_model.rs"]
mod repeated_model;
