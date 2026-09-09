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
#[derive(Debug, serde::Serialize)]
pub(crate) struct RunMarker {
    /// The generation current pointed at when the run began (0026 §4):
    /// reconcile decides by EXACT equality — current == target is
    /// committed, current == previous is uncommitted, anything else is
    /// ambiguous and blocks. Numeric inequalities misclassify a
    /// crashed roll-FORWARD (current < target before the flip).
    /// Null is a fresh machine's first run. The KEY is required on
    /// the wire (0045 F2): serde would silently read a missing key as
    /// `None`, so deserialization is manual below — a marker missing
    /// it is torn or corrupt and fails closed, never mistaken for a
    /// fresh run.
    pub(crate) previous_generation: Option<u64>,
    pub(crate) target_generation: u64,
    /// Apply builds a NEWER generation (committed once `current`
    /// reaches the target); rollback returns to an OLDER one, so its
    /// commit condition inverts (committed once `current` comes back
    /// DOWN to the target) — 0025 §A.
    pub(crate) op: RunOp,
}

// Manual `Deserialize` (0045 F2): the derived impl cannot express
// "key must be present, value may be null" — serde fills a missing
// `Option` field with `None`, which would read a torn marker as a
// fresh machine's first run and misclassify recovery. Unknown keys
// stay tolerated (a newer grip's marker is inspected, not executed,
// by an older one); duplicates, wrong types and oversized numbers are
// rejected.
impl<'de> serde::Deserialize<'de> for RunMarker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{Error, IgnoredAny, MapAccess, Visitor};

        enum Field {
            PreviousGeneration,
            TargetGeneration,
            Op,
            Unknown,
        }

        impl<'de> serde::Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                Ok(match <&str>::deserialize(deserializer)? {
                    "previous_generation" => Field::PreviousGeneration,
                    "target_generation" => Field::TargetGeneration,
                    "op" => Field::Op,
                    _ => Field::Unknown,
                })
            }
        }

        struct MarkerVisitor;

        impl<'de> Visitor<'de> for MarkerVisitor {
            type Value = RunMarker;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a run marker with previous_generation, target_generation and op")
            }

            fn visit_map<A>(self, mut map: A) -> Result<RunMarker, A::Error>
            where
                A: MapAccess<'de>,
            {
                // None = key not seen; Some(None) = seen, null (fresh
                // machine); Some(Some(n)) = seen, a generation.
                let mut previous: Option<Option<u64>> = None;
                let mut target: Option<u64> = None;
                let mut op: Option<RunOp> = None;
                while let Some(field) = map.next_key()? {
                    match field {
                        Field::PreviousGeneration => {
                            if previous.is_some() {
                                return Err(A::Error::duplicate_field("previous_generation"));
                            }
                            previous = Some(map.next_value()?);
                        }
                        Field::TargetGeneration => {
                            if target.is_some() {
                                return Err(A::Error::duplicate_field("target_generation"));
                            }
                            target = Some(map.next_value()?);
                        }
                        Field::Op => {
                            if op.is_some() {
                                return Err(A::Error::duplicate_field("op"));
                            }
                            op = Some(map.next_value()?);
                        }
                        Field::Unknown => {
                            let _ = map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(RunMarker {
                    previous_generation: previous
                        .ok_or_else(|| A::Error::missing_field("previous_generation"))?,
                    target_generation: target
                        .ok_or_else(|| A::Error::missing_field("target_generation"))?,
                    op: op.ok_or_else(|| A::Error::missing_field("op"))?,
                })
            }
        }

        deserializer.deserialize_map(MarkerVisitor)
    }
}

/// What the run is doing — the reconcile commit decision differs by
/// direction (see RunMarker).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOp {
    /// The normal case: building the next generation.
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
        gripsack_fs::remove_file(home, path)?;
    }
    gripsack_fs::fsync_dir(home, Path::new("journal"))?;
    // a run that never mutated has no marker (end_run already
    // removed it) — absent is fine, anything else is real
    match gripsack_fs::remove_file(home, &run_marker_rel()) {
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
    match gripsack_fs::remove_file(home, &run_marker_rel()) {
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
/// is uncommitted, anything else is ambiguous and blocks. A marker
/// without `previous_generation` never reaches here — the field is
/// required on the wire and a torn marker fails closed at parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Classification {
    Committed,
    Uncommitted,
    Ambiguous,
}

/// The classifier's whole input, named: the facts a run marker
/// carries, plus the live `current` read at recovery time. (A struct,
/// not positional `Option<u64>`-flavored arguments.)
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecoveryFacts {
    /// The generation the run started from — None is a fresh
    /// machine's first run.
    pub(crate) previous: Option<u64>,
    /// The generation the run was building toward.
    pub(crate) target: u64,
    /// `current` on disk when recovery ran.
    pub(crate) current: Option<u64>,
}

pub(crate) fn classify(facts: &RecoveryFacts) -> Classification {
    match (facts.previous, facts.current) {
        (Some(_), Some(c)) if c == facts.target => Classification::Committed,
        (Some(prev), Some(c)) if c == prev => Classification::Uncommitted,
        (Some(_), _) => Classification::Ambiguous,
        // a fresh machine's first run: current at the target means the
        // flip landed; absent means it never did
        (None, Some(c)) if c == facts.target => Classification::Committed,
        (None, None) => Classification::Uncommitted,
        (None, Some(_)) => Classification::Ambiguous,
    }
}

#[cfg(test)]
#[path = "repeated_model.rs"]
mod repeated_model;
