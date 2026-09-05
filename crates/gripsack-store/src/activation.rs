//! The activation pending record (plan/0032): post-activation intents
//! (cache refreshes, services, custom hooks) are durable state.
//!
//! `apply` writes the record BEFORE the flip — a kill anywhere leaves
//! either a committed generation with a pending record (the next run
//! resumes the adapters) or a record naming a generation that never
//! became current (the next run discards it). The pre-0.28 shape ran
//! adapters with no record at all: a kill silently skipped them (the
//! TLA+ counterexample in `specs/Activation.tla`'s header).

use gripsack_fs::Dir;
use std::io;
use std::path::Path;

/// One declared activation intent with its owning module (reports
/// name modules; the pending record must stand alone if the repo has
/// moved on by resume time).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PendingIntent {
    pub module: String,
    pub action: gripsack_ir::Action,
}

/// The intents awaiting execution for a committed generation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PendingActivation {
    pub generation: u64,
    pub intents: Vec<PendingIntent>,
}

const REL: &str = "activation.json";

/// Persist the pending record atomically (temp + rename + fsync).
pub fn write_pending(home: &Dir, pending: &PendingActivation) -> io::Result<()> {
    gripsack_fs::atomic_write(
        home,
        Path::new(REL),
        serde_json::to_string(pending)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )
}

/// The pending record, if any. Corrupt records fail closed (recovery
/// metadata is never ignored — same rule as the journal, 0020 §5.2).
pub fn read_pending(home: &Dir) -> io::Result<Option<PendingActivation>> {
    match home.read(Path::new(REL)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Drop the record — after the intents ran, or when the named
/// generation is no longer current (superseded or rolled back).
pub fn clear_pending(home: &Dir) -> io::Result<()> {
    match home.remove_file(Path::new(REL)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    gripsack_fs::fsync_dir(home, Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_record_round_trips_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let home = gripsack_fs::open_or_create(dir.path()).unwrap();
        assert!(read_pending(&home).unwrap().is_none());
        let pending = PendingActivation {
            generation: 3,
            intents: vec![PendingIntent {
                module: "demo".into(),
                action: gripsack_ir::Action::Fonts,
            }],
        };
        write_pending(&home, &pending).unwrap();
        assert_eq!(read_pending(&home).unwrap(), Some(pending));
        clear_pending(&home).unwrap();
        assert!(read_pending(&home).unwrap().is_none());
        // clearing twice is fine (resume after a partial clear)
        clear_pending(&home).unwrap();
    }

    #[test]
    fn a_corrupt_record_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let home = gripsack_fs::open_or_create(dir.path()).unwrap();
        std::fs::write(dir.path().join(REL), b"{torn").unwrap();
        assert!(read_pending(&home).is_err());
    }
}
