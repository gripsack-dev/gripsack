//! Retained tree rollback releases only admitted leaves, evolves shared root
//! shapes, and exchanges distinct incarnations without moving foreign contents.
use super::DestMap;
use crate::ExecError;
use gripsack_store::{
    self as store,
    journal::{ObjectIdentity, directory::DirectoryTransition},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(super) struct Prepared {
    pub released: BTreeSet<store::OwnershipKey>,
    pub notes: Vec<store::journal::RecoveryNote>,
}

pub(super) fn prepare(
    journal: &store::journal::JournalRun<'_>,
    current: &DestMap<'_>,
    current_directories: &[DirectoryTransition],
    target: &store::Generation,
) -> Result<Prepared, ExecError> {
    for receipt in current_directories {
        store::journal::directory::bind_existing(journal, receipt)?;
    }
    let target_keys: BTreeSet<_> = target
        .modules
        .iter()
        .flat_map(|(owner, state)| {
            state
                .entries
                .iter()
                .map(move |entry| entry.ownership_key(owner))
        })
        .collect();
    let mut result = Prepared {
        released: BTreeSet::new(),
        notes: Vec::new(),
    };
    for receipt in &target.directory_transitions {
        let previous = current_directories
            .iter()
            .find(|previous| previous.destination == receipt.destination);
        let same_root = previous.is_some_and(|previous| {
            previous.directories.get(Path::new("")) == receipt.directories.get(Path::new(""))
        });
        if same_root {
            receipt.verify_borrowed()?;
        }
        let changing: Vec<PathBuf> = previous
            .into_iter()
            .flat_map(|previous| previous.directories.iter())
            .filter(|(relative, identity)| {
                !relative.as_os_str().is_empty()
                    && receipt.directories.get(*relative) != Some(*identity)
            })
            .map(|(relative, _)| receipt.destination.join(relative))
            .collect();
        let needed_nodes: BTreeSet<_> = receipt
            .directories
            .keys()
            .map(|relative| receipt.destination.join(relative))
            .collect();
        if previous.is_some() {
            let mut children: Vec<_> = current
                .iter()
                .filter(|(key, _)| {
                    key.destination() != receipt.destination
                        && key.destination().starts_with(&receipt.destination)
                })
                .collect();
            children
                .sort_by_key(|(key, _)| std::cmp::Reverse(key.destination().components().count()));
            for (key, (owner, entry, source)) in children {
                let structural = !same_root
                    || changing
                        .iter()
                        .any(|root| key.destination().starts_with(root))
                    || needed_nodes.contains(key.destination());
                if !structural && target_keys.contains(key) {
                    continue;
                }
                result.released.insert(key.clone());
                if entry.preserved_drift {
                    if structural {
                        return Err(failure(
                            "preserved child prevents directory shape replacement",
                        ));
                    }
                    continue;
                }
                if let Some(operation) =
                    crate::ops::plan_remove_op(owner, entry, source, journal.home())?
                {
                    if matches!(operation.kind(), crate::ops::OpKind::Preserved) {
                        if structural {
                            return Err(failure(
                                "foreign child prevents directory shape replacement",
                            ));
                        }
                        result.notes.push(store::journal::RecoveryNote {
                            severity: store::journal::NoteSeverity::Warn,
                            message: format!(
                                "kept {}: changed child is outside rollback ownership",
                                key.destination().display()
                            ),
                        });
                    } else {
                        crate::ops::execute_op(journal, operation.as_executable()?)?;
                    }
                }
            }
        }
        if same_root {
            store::journal::directory::restore_shape(journal, receipt)?;
            continue;
        }
        let expected = if let Some(previous) = previous {
            store::journal::reverse_directory(journal, previous)?;
            Some(ObjectIdentity::Link(
                previous
                    .predecessor_target
                    .to_str()
                    .ok_or_else(|| failure("non-UTF-8 predecessor"))?
                    .to_owned(),
            ))
        } else {
            let key = store::OwnershipKey::new(receipt.destination.clone(), None);
            match current.get(&key) {
                None => restored_origin(journal, receipt)?,
                Some((owner, entry, source)) => {
                    if entry.preserved_drift {
                        return Err(failure("preserved root prevents directory reactivation"));
                    }
                    match crate::ops::plan_remove_op(owner, entry, source, journal.home())? {
                        None => None,
                        Some(operation)
                            if matches!(operation.kind(), crate::ops::OpKind::Preserved) =>
                        {
                            return Err(failure("foreign root prevents directory reactivation"));
                        }
                        Some(operation) => operation.observed().cloned(),
                    }
                }
            }
        };
        store::journal::directory::reactivate(journal, receipt, expected.as_ref())?;
    }
    Ok(result)
}

fn restored_origin(
    journal: &store::journal::JournalRun<'_>,
    receipt: &DirectoryTransition,
) -> Result<Option<ObjectIdentity>, ExecError> {
    let predecessor =
        store::generations::admit_manifest(journal.home_path(), receipt.predecessor_generation)?;
    let (state, entry) = predecessor
        .modules
        .values()
        .find_map(|state| {
            state
                .entries
                .iter()
                .find(|entry| entry.key() == receipt.destination)
                .map(|entry| (state, entry))
        })
        .ok_or_else(|| failure("retained directory origin is unavailable"))?;
    if entry.preserved_drift
        || entry.ownership.policy() != gripsack_ir::Ownership::Owned
        || state.store_path.join(&entry.from) != receipt.predecessor_target
    {
        return Err(failure(
            "retained directory origin differs from ownership receipt",
        ));
    }
    match &entry.prior {
        None => Ok(None),
        Some(store::Prior::Symlink { target }) => Ok(Some(ObjectIdentity::Link(target.clone()))),
        Some(store::Prior::File { hash, mode }) => {
            Ok(Some(ObjectIdentity::File(store::canonical_bytes_identity(
                &store::prior::read_blob(journal.home(), hash)?,
                mode.bits(),
            ))))
        }
    }
}

fn failure(message: &str) -> ExecError {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}
