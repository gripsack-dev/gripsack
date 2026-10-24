//! Directory rollback receipts retain the exact predecessor generation and its
//! lineage, independently of the ordinary history window.
use crate::ExecError;
use gripsack_store as store;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(super) fn admit(
    directory: &store::generations::GenerationDirectory,
    home: &gripsack_fs::Dir,
    home_path: &Path,
    pruned: &mut BTreeSet<store::GenerationId>,
) -> Result<BTreeMap<store::GenerationId, store::Generation>, ExecError> {
    let mut manifests = BTreeMap::new();
    for number in directory.inventory().as_slice() {
        manifests.insert(*number, directory.admit_manifest(home, home_path, *number)?);
    }
    loop {
        let dependencies: Vec<_> = manifests
            .iter()
            .filter(|(number, _)| !pruned.contains(*number))
            .flat_map(|(_, generation)| {
                generation
                    .directory_transitions
                    .iter()
                    .map(|receipt| receipt.predecessor_generation)
                    .chain(
                        generation
                            .migration
                            .iter()
                            .map(|receipt| receipt.predecessor),
                    )
            })
            .collect();
        let mut changed = false;
        for predecessor in dependencies {
            if !manifests.contains_key(&predecessor) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "directory receipt predecessor generation is missing; refusing GC",
                )
                .into());
            }
            changed |= pruned.remove(&predecessor);
        }
        if !changed {
            break;
        }
    }
    Ok(manifests)
}

pub(super) fn retire(
    manifests: &BTreeMap<store::GenerationId, store::Generation>,
    pruned: &BTreeSet<store::GenerationId>,
) -> Result<(), ExecError> {
    let retained: Vec<_> = manifests
        .iter()
        .filter(|(number, _)| !pruned.contains(*number))
        .flat_map(|(_, generation)| &generation.directory_transitions)
        .collect();
    for generation in manifests
        .values()
        .filter(|generation| pruned.contains(&generation.number))
    {
        for receipt in &generation.directory_transitions {
            store::journal::retire_directory_staging(receipt, &retained)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(home: &gripsack_fs::Dir, number: u64, predecessor: Option<u64>) {
        store::write_manifest(
            home,
            &store::Generation {
                number: store::GenerationId::new(number),
                modules: BTreeMap::new(),
                directory_transitions: Vec::new(),
                migration: predecessor.map(|number| store::generations::MigrationReceipt {
                    predecessor: store::GenerationId::new(number),
                }),
            },
        )
        .unwrap();
    }

    #[test]
    fn migration_recovery_roots_close_transitively_outside_the_keep_window() {
        let temporary = tempfile::tempdir().unwrap();
        let home = gripsack_fs::open_or_create(temporary.path()).unwrap();
        publish(&home, 0, None);
        publish(&home, 1, Some(0));
        publish(&home, 2, Some(1));
        let directory = store::generations::GenerationDirectory::open(&home).unwrap();
        let mut pruned = BTreeSet::from([store::GenerationId::new(0), store::GenerationId::new(1)]);
        let admitted = admit(&directory, &home, temporary.path(), &mut pruned).unwrap();
        assert_eq!(admitted.len(), 3);
        assert!(pruned.is_empty());
    }

    #[test]
    fn missing_migration_predecessor_blocks_collection_before_effects() {
        let temporary = tempfile::tempdir().unwrap();
        let home = gripsack_fs::open_or_create(temporary.path()).unwrap();
        publish(&home, 1, Some(0));
        let directory = store::generations::GenerationDirectory::open(&home).unwrap();
        assert!(admit(&directory, &home, temporary.path(), &mut BTreeSet::new()).is_err());
        assert!(
            temporary
                .path()
                .join("generations/1/manifest.json")
                .is_file()
        );
    }
}
