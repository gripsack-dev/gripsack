//! The run marker and the commit classifier (0019, 0026 §4, 0028):
//! a run declares its target generation before any mutation, and
//! recovery classifies the interrupted run by EXACT transaction
//! identity.

use super::mutation::StoreRunHost;
use crate::{GenerationId, generations::SelectionReservation};
use gripsack_fs::Dir;
use gripsack_host::journal_run::RunDriver;
use gripsack_host::storage::RealFs;
use gripsack_kernel::selection::SelectionIdentity;
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

pub(crate) use super::marker_wire::RunMarker;
use super::storage::Journal;
pub(super) use super::storage::RUN_MARKER;

/// A transaction-specific selection whose run marker is durably published.
/// New flips require this handle; generation equality alone is not authority.
#[derive(Debug)]
pub struct JournalRun<'a> {
    home: &'a Dir,
    home_path: &'a Path,
    pub(super) journal: Journal,
    /// The kernel's journal-run machine (plan/0056 F2 step 3): it published
    /// the marker, decides every leaf mutation's protocol and the cleanup.
    machine: Mutex<RunDriver<RealFs>>,
    /// Only records durably published by this run enter this cache. Sharing
    /// read guards avoids rescanning a growing journal for every leaf write.
    pub(super) directories: std::sync::RwLock<Vec<super::directory_wire::DirectoryRecord>>,
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

    pub(crate) fn target(&self) -> &Path {
        self.reservation.target()
    }

    pub(super) fn machine(&self) -> io::Result<MutexGuard<'_, RunDriver<RealFs>>> {
        self.machine
            .lock()
            .map_err(|_| io::Error::other("journal run machine lock poisoned"))
    }

    fn host(&self) -> io::Result<StoreRunHost<'_>> {
        StoreRunHost::new(&self.journal, self.home, self.home_path)
    }

    /// The units are done: the kernel stops at the flip, which the caller
    /// performs and acknowledges.
    pub(crate) fn request_flip(&self) -> io::Result<()> {
        self.machine()?
            .commit(&mut self.host()?)
            .map_err(|error| error.into_io())
    }

    pub(crate) fn flip_failed(&self, error: &io::Error) {
        if let Ok(mut machine) = self.machine() {
            machine.flip_failed(error);
        }
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
    let bytes =
        serde_json::to_vec(&marker).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let machine =
        RunDriver::begin(&mut StoreRunHost::new(&journal, home, home_path)?.with_marker(bytes))
            .map_err(|error| error.into_io())?;
    Ok(JournalRun {
        home,
        home_path,
        journal,
        machine: Mutex::new(machine),
        reservation,
        directories: std::sync::RwLock::new(Vec::new()),
    })
}

/// The run completed and the generation flipped: nothing left to
/// recover. The kernel removes every record, makes that durable, then
/// removes the marker and makes that durable (0026 §5): marker-durably-gone
/// implies records-durably-gone, so a power loss can never resurrect entries
/// without their marker (the 0.19.x bug class).
pub fn commit_run(committed: crate::CommittedSelection<'_>) -> io::Result<()> {
    let run = committed.run();
    run.machine()?
        .flipped(&mut run.host()?)
        .map_err(|error| error.into_io())
}

/// The run ended without mutating anything (satisfied, empty graph):
/// the marker declared by `begin_run` must not linger — a stale
/// marker with no entries is harmless but noisy, and a marker whose
/// target generation is later than `current` would misread the NEXT
/// crash window. The kernel refuses to end a run that journaled a
/// destination.
pub fn end_run(run: JournalRun<'_>) -> io::Result<()> {
    run.machine()?
        .end(&mut run.host()?)
        .map_err(|error| error.into_io())
}

/// The legacy reconcile's cleanup (kept with it behind the debug-only
/// `GRIPSACK_ENGINE=legacy` switch): every record unlinked, one barrier, the
/// marker unlinked, a second barrier. The kernel engines decide this order.
pub(super) fn cleanup_legacy<'a>(
    directory: &Dir,
    entry_paths: impl IntoIterator<Item = &'a Path>,
) -> io::Result<()> {
    for path in entry_paths {
        gripsack_fs::remove_file(directory, path)?;
    }
    gripsack_fs::fsync_dir(directory, Path::new("."))?;
    match gripsack_fs::remove_file(directory, Path::new(RUN_MARKER)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    gripsack_fs::fsync_dir(directory, Path::new("."))
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

/// One exact-selection kernel serves recovery and the Lean proofs
/// (plan/0056).
pub(crate) use gripsack_kernel::{
    Classification,
    selection::{RecoveryFacts, classify},
};
