//! The run marker and the commit classifier (0019, 0026 §4, 0028):
//! a run declares its target generation before any mutation, and
//! recovery classifies the interrupted run by EXACT transaction
//! identity.

use crate::{GenerationId, generations::SelectionReservation};
use gripsack_fs::Dir;
use gripsack_policy::selection::SelectionIdentity;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) use super::marker_wire::RunMarker;
use super::storage::{Journal, RUN_MARKER};

/// A transaction-specific selection whose run marker is durably published.
/// New flips require this handle; generation equality alone is not authority.
#[derive(Debug)]
pub struct PendingSelection {
    reservation: SelectionReservation,
}

impl PendingSelection {
    pub fn identity(&self) -> &SelectionIdentity {
        self.reservation.identity()
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
pub fn begin_run(
    home: &Dir,
    home_path: &Path,
    previous_generation: Option<GenerationId>,
    target_generation: GenerationId,
    op: RunOp,
) -> io::Result<PendingSelection> {
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
    Journal::prepare(home)?.write(
        Path::new(RUN_MARKER),
        serde_json::to_string(&marker)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )?;
    Ok(PendingSelection { reservation })
}

/// The run completed and the generation flipped: nothing left to
/// recover. Entries, stragglers, and the run marker are gone.
pub fn commit_run(home: &Dir) -> io::Result<()> {
    let Some(journal) = Journal::open(home)? else {
        return Ok(());
    };
    journal.restrict()?;
    let entries = journal.directory.read_dir(".")?;
    let mut entry_paths = Vec::new();
    for entry in entries {
        let name = entry?.file_name();
        let rel = PathBuf::from(&name);
        // the marker is NOT deleted here — cleanup deletes it last
        if name != RUN_MARKER && rel.extension().is_some_and(|e| e == "json") {
            entry_paths.push(rel);
        }
    }
    cleanup(&journal, &entry_paths)
}

/// Two durability barriers (0026 §5): entries deleted and fsync'd
/// FIRST, the marker deleted and fsync'd SECOND — so marker-durably-
/// gone implies entries-durably-gone. A single trailing fsync does
/// not order the deletions against a power loss; resurrected entries
/// with no marker read as an uncommitted run and would restore a
/// committed generation's priors (the 0.19.1 bug class, one level
/// down).
pub(super) fn cleanup(journal: &Journal, entry_paths: &[PathBuf]) -> io::Result<()> {
    for path in entry_paths {
        gripsack_fs::remove_file(&journal.directory, path)?;
    }
    gripsack_fs::fsync_dir(&journal.directory, Path::new("."))?;
    // a run that never mutated has no marker (end_run already
    // removed it) — absent is fine, anything else is real
    match gripsack_fs::remove_file(&journal.directory, Path::new(RUN_MARKER)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    gripsack_fs::fsync_dir(&journal.directory, Path::new("."))
}

/// The run ended without mutating anything (satisfied, empty graph):
/// the marker declared by `begin_run` must not linger — a stale
/// marker with no entries is harmless but noisy, and a marker whose
/// target generation is later than `current` would misread the NEXT
/// crash window.
pub fn end_run(home: &Dir) -> io::Result<()> {
    let Some(journal) = Journal::open(home)? else {
        return Ok(());
    };
    journal.restrict()?;
    // a stale marker misleads the NEXT crash window — its deletion is
    // a durability operation, never `let _ =` (0030 §12)
    match gripsack_fs::remove_file(&journal.directory, Path::new(RUN_MARKER)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    gripsack_fs::fsync_dir(&journal.directory, Path::new("."))
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
