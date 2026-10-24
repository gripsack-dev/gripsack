//! Strict versioned directory records and durable mutation authority.
use super::{
    JournalRun,
    directory::{DirectoryTransition, invalid},
    storage::Journal,
};
use gripsack_policy::journal_protocol::{RecordRole, admit_mutation};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

const DIRECTORY_WIRE_VERSION: u8 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DirectoryRecord {
    pub v: u8,
    pub direction: Direction,
    pub transition: DirectoryTransition,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Direction {
    Forward,
    Reverse,
    Bound,
    Reactivate { prior: super::PriorSerde },
    Exchange { original: DirectoryTransition },
    Evolve { original: DirectoryTransition },
}
pub(super) fn record_name(transition: &DirectoryTransition) -> PathBuf {
    PathBuf::from(format!(
        "directory-{}.json",
        crate::hash::hex_sha256(transition.destination.as_os_str().as_encoded_bytes())
    ))
}

pub(super) fn read_records(journal: &Journal) -> io::Result<Vec<(PathBuf, DirectoryRecord)>> {
    let mut records = Vec::new();
    for entry in journal.directory.read_dir(".")? {
        let name = PathBuf::from(entry?.file_name());
        if !name.to_string_lossy().starts_with("directory-") {
            continue;
        }
        let bytes = journal.read(&name)?;
        let parsed = serde_json::from_slice::<DirectoryRecord>(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
            .and_then(|record| {
                if record.v != DIRECTORY_WIRE_VERSION {
                    return Err(invalid(
                        "unsupported directory journal version (requires v3)",
                    ));
                }
                record.transition.validate()?;
                if let Direction::Exchange { original } = &record.direction {
                    original.validate()?;
                    if original.destination != record.transition.destination
                        || original.staging == record.transition.staging
                        || original.directories[Path::new("")]
                            == record.transition.directories[Path::new("")]
                        || original.parent_identity != record.transition.parent_identity
                    {
                        return Err(invalid(
                            "directory exchange identities do not share one logical parent",
                        ));
                    }
                }
                if let Direction::Evolve { original } = &record.direction {
                    original.validate()?;
                    if original.destination != record.transition.destination
                        || original.staging != record.transition.staging
                        || original.parent_identity != record.transition.parent_identity
                        || original.directories[Path::new("")]
                            != record.transition.directories[Path::new("")]
                    {
                        return Err(invalid("directory evolution changed root incarnation"));
                    }
                }
                if record_name(&record.transition) != name {
                    return Err(invalid("directory journal key mismatch"));
                }
                Ok(record)
            });
        match parsed {
            Ok(record) => records.push((name, record)),
            Err(error) => {
                journal.quarantine(&name)?;
                return Err(error);
            }
        }
    }
    let roots: std::collections::BTreeSet<&Path> = records
        .iter()
        .map(|(_, record)| record.transition.destination.as_path())
        .collect();
    if roots.iter().any(|root| {
        root.ancestors()
            .skip(1)
            .any(|ancestor| roots.contains(ancestor))
    }) {
        for (name, _) in &records {
            journal.quarantine(name)?;
        }
        return Err(invalid("overlapping directory journal roots quarantined"));
    }
    Ok(records)
}

pub(super) fn persist(
    run: &JournalRun<'_>,
    transition: &DirectoryTransition,
    direction: Direction,
) -> io::Result<()> {
    transition.validate()?;
    let record = DirectoryRecord {
        v: DIRECTORY_WIRE_VERSION,
        direction,
        transition: transition.clone(),
    };
    let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
    let durable = run
        .journal
        .write(&record_name(transition), &bytes, RecordRole::MutationEntry)?;
    admit_mutation(&run.marker_record, durable)
        .ok_or_else(|| invalid("directory mutation lacks durable run authority"))?;
    let mut bindings = run
        .directories
        .write()
        .map_err(|_| invalid("directory authority lock poisoned"))?;
    if let Some(existing) = bindings
        .iter_mut()
        .find(|existing| existing.transition.destination == transition.destination)
    {
        *existing = record;
    } else {
        bindings.push(record);
    }
    Ok(())
}
