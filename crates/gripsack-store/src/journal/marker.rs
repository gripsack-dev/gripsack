//! The run marker and the commit classifier (0019, 0026 §4, 0028):
//! a run declares its target generation before any mutation, and
//! recovery classifies the interrupted run by EXACT transaction
//! identity.

use gripsack_fs::Dir;
use std::io;
use std::path::{Path, PathBuf};

use super::run_marker_rel;

/// The run marker: which generation this journal's entries belong to.
/// Written before the first mutation; the flip makes it true.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct RunMarker {
    /// The generation current pointed at when the run began (0026 §4):
    /// reconcile decides by EXACT equality — current == target is
    /// committed, current == previous is uncommitted, anything else is
    /// ambiguous and blocks. Numeric inequalities misclassify a
    /// crashed roll-FORWARD (current < target before the flip).
    /// None only in pre-0.23 markers, which reconcile by the 0.22
    /// direction rule.
    #[serde(default)]
    pub(crate) previous_generation: Option<u64>,
    pub(crate) target_generation: u64,
    /// 2 for markers written since 0.26 (0030 §11): distinguishes
    /// "no previous generation" (a fresh machine) from a pre-0.23
    /// marker that never recorded one — the latter refuses to
    /// reconcile, the former classifies exactly
    #[serde(default)]
    pub(crate) format: u8,
    /// Apply builds a NEWER generation (committed once `current`
    /// reaches the target); rollback returns to an OLDER one, so its
    /// commit condition inverts (committed once `current` comes back
    /// DOWN to the target) — 0025 §A. Default keeps pre-0.22 markers
    /// readable as apply runs.
    #[serde(default)]
    pub(crate) op: RunOp,
}

/// What the run is doing — the reconcile commit decision differs by
/// direction (see RunMarker).
#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOp {
    /// The normal case: building the next generation.
    #[default]
    Apply,
    /// Returning to a previous generation.
    Rollback,
}

/// Declare the generation this run is building, BEFORE any mutation:
/// recovery compares it against `current` — a crash between the flip
/// and journal cleanup must NOT restore priors the committed
/// generation now owns (the post-commit window, review finding 5.1).
pub fn begin_run(
    home: &Dir,
    previous_generation: Option<u64>,
    target_generation: u64,
    op: RunOp,
) -> io::Result<()> {
    let marker = RunMarker {
        previous_generation,
        target_generation,
        op,
        format: 2,
    };
    gripsack_fs::atomic_write(
        home,
        &run_marker_rel(),
        serde_json::to_string(&marker)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )
}

/// The run completed and the generation flipped: nothing left to
/// recover. Entries, stragglers, and the run marker are gone.
pub fn commit_run(home: &Dir) -> io::Result<()> {
    let entries = match home.read_dir("journal") {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut entry_paths = Vec::new();
    for entry in entries {
        let name = entry?.file_name();
        let rel = Path::new("journal").join(&name);
        // the marker is NOT deleted here — cleanup deletes it last
        if name != "run.json" && rel.extension().is_some_and(|e| e == "json") {
            entry_paths.push(rel);
        }
    }
    cleanup(home, &entry_paths)
}

/// Two durability barriers (0026 §5): entries deleted and fsync'd
/// FIRST, the marker deleted and fsync'd SECOND — so marker-durably-
/// gone implies entries-durably-gone. A single trailing fsync does
/// not order the deletions against a power loss; resurrected entries
/// with no marker read as an uncommitted run and would restore a
/// committed generation's priors (the 0.19.1 bug class, one level
/// down).
pub(crate) fn cleanup(home: &Dir, entry_paths: &[PathBuf]) -> io::Result<()> {
    for path in entry_paths {
        home.remove_file(path)?;
    }
    gripsack_fs::fsync_dir(home, Path::new("journal"))?;
    // a run that never mutated has no marker (end_run already
    // removed it) — absent is fine, anything else is real
    match home.remove_file(run_marker_rel()) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    gripsack_fs::fsync_dir(home, Path::new("journal"))
}

/// The run ended without mutating anything (satisfied, empty graph):
/// the marker declared by `begin_run` must not linger — a stale
/// marker with no entries is harmless but noisy, and a marker whose
/// target generation is later than `current` would misread the NEXT
/// crash window.
pub fn end_run(home: &Dir) -> io::Result<()> {
    // a stale marker misleads the NEXT crash window — its deletion is
    // a durability operation, never `let _ =` (0030 §12)
    match home.remove_file(run_marker_rel()) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    gripsack_fs::fsync_dir(home, Path::new("journal"))
}

pub(crate) fn run_marker(home: &Dir) -> io::Result<Option<RunMarker>> {
    match home.read(run_marker_rel()) {
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

/// The commit classification as a pure function (0028), exact
/// equality only: current == target is committed, current == previous
/// is uncommitted, anything else is ambiguous and blocks. Pre-0.23
/// markers carry no previous generation; those refuse (never
/// auto-classify by the model-proven-unsound direction rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Classification {
    Committed,
    Uncommitted,
    Ambiguous,
    /// A pre-0.23 run marker (no previous_generation): the commit
    /// state is unknowable - refuse, never auto-classify (0030 §11)
    Legacy,
}

/// The classifier's whole input, named: the facts a run marker
/// carries, plus the live `current` read at recovery time. (A struct,
/// not five positional `Option<u64>`-flavored arguments - the model
/// harness passes the same shape to its legacy counterexample.)
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecoveryFacts {
    /// The generation the run started from. None for a fresh
    /// machine's first run - and for pre-0.23 markers, which never
    /// recorded it (told apart by `format`).
    pub(crate) previous: Option<u64>,
    /// The generation the run was building toward.
    pub(crate) target: u64,
    /// `current` on disk when recovery ran.
    pub(crate) current: Option<u64>,
    /// Marker schema: 2+ carries exact transaction identity.
    pub(crate) format: u8,
}

pub(crate) fn classify(facts: &RecoveryFacts) -> Classification {
    match (facts.previous, facts.current) {
        (Some(_), Some(c)) if c == facts.target => Classification::Committed,
        (Some(prev), Some(c)) if c == prev => Classification::Uncommitted,
        (Some(_), _) => Classification::Ambiguous,
        // a fresh machine's run (no previous generation): exact too -
        // distinguishable from legacy by the format field
        (None, Some(c)) if facts.format >= 2 && c == facts.target => Classification::Committed,
        (None, None) if facts.format >= 2 => Classification::Uncommitted,
        (None, Some(_)) if facts.format >= 2 => Classification::Ambiguous,
        // pre-0.23 marker (no format field): the directional rule is
        // proven unsound (0028's kept counterexample) - refuse with
        // guidance rather than guess (0030 §11)
        _ => Classification::Legacy,
    }
}
