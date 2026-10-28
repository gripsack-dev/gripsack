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
    dry_run: bool,
) -> Result<BTreeMap<store::GenerationId, store::Generation>, ExecError> {
    let mut manifests = BTreeMap::new();
    for number in directory.inventory().as_slice() {
        let manifest = if dry_run {
            directory.inspect_manifest(home_path, *number)?
        } else {
            directory.admit_manifest(home, home_path, *number)?
        };
        manifests.insert(*number, manifest);
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

pub(super) fn pending(
    directory: &store::generations::GenerationDirectory,
    home: &gripsack_fs::Dir,
    home_path: &Path,
    dry_run: bool,
) -> Result<BTreeMap<store::GenerationId, store::Generation>, ExecError> {
    directory
        .retiring()
        .iter()
        .map(|number| {
            directory
                .retirement_manifest(home, home_path, *number, dry_run)
                .map(|manifest| (*number, manifest))
                .map_err(|error| retirement_error(*number, None, error))
        })
        .collect()
}

pub(super) fn retire(
    directory: &store::generations::GenerationDirectory,
    manifests: &BTreeMap<store::GenerationId, store::Generation>,
    pruned: &BTreeSet<store::GenerationId>,
    pending: &BTreeMap<store::GenerationId, store::Generation>,
) -> Result<(), ExecError> {
    // Pending tombstones protect shared identities too. Release only the
    // receipt being processed; its last remaining owner performs deletion.
    let mut retained: Vec<_> = manifests
        .values()
        .chain(pending.values())
        .flat_map(|generation| &generation.directory_transitions)
        .collect();
    for generation in pending.values().chain(
        manifests
            .values()
            .filter(|generation| pruned.contains(&generation.number)),
    ) {
        retained.retain(|receipt| {
            !generation
                .directory_transitions
                .iter()
                .any(|other| std::ptr::eq(*receipt, other))
        });
        for receipt in &generation.directory_transitions {
            store::journal::retire_directory_staging(receipt, &retained)
                .and_then(|()| {
                    // A previous unlink may have succeeded but failed its sync.
                    // Seal observed absence even when discard was a no-op.
                    gripsack_fs::fsync_pinned_dir(&receipt.parent()?, Path::new("."))
                })
                .map_err(|error| {
                    retirement_error(generation.number, Some(&receipt.destination), error)
                })?;
        }
        directory
            .finish_retirement(generation.number)
            .map_err(|error| retirement_error(generation.number, None, error))?;
    }
    Ok(())
}

fn retirement_error(
    number: store::GenerationId,
    destination: Option<&Path>,
    error: std::io::Error,
) -> ExecError {
    ExecError::Step {
        output: "*".into(),
        step: "gc directory retirement".into(),
        detail: format!(
            "generations/.retiring-{number}{}: {error}; pending retirement/cleanup will be retried by GC",
            destination
                .map(|path| format!(" ({})", path.display()))
                .unwrap_or_default(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(home: &gripsack_fs::Dir, number: u64, predecessor: Option<u64>) {
        store::write_manifest(
            home,
            &store::Generation {
                number: store::GenerationId::new(number),
                deployments: BTreeMap::new(),
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
        let admitted = admit(&directory, &home, temporary.path(), &mut pruned, false).unwrap();
        assert_eq!(admitted.len(), 3);
        assert!(pruned.is_empty());
    }

    #[test]
    fn missing_migration_predecessor_blocks_collection_before_effects() {
        let temporary = tempfile::tempdir().unwrap();
        let home = gripsack_fs::open_or_create(temporary.path()).unwrap();
        publish(&home, 1, Some(0));
        let directory = store::generations::GenerationDirectory::open(&home).unwrap();
        assert!(
            admit(
                &directory,
                &home,
                temporary.path(),
                &mut BTreeSet::new(),
                false
            )
            .is_err()
        );
        assert!(
            temporary
                .path()
                .join("generations/1/manifest.json")
                .is_file()
        );
    }

    fn retirement_fixture() -> (
        tempfile::TempDir,
        gripsack_fs::Dir,
        store::journal::directory::DirectoryTransition,
    ) {
        let temporary = tempfile::tempdir().unwrap();
        let home = gripsack_fs::open_or_create(temporary.path()).unwrap();
        let target = temporary.path().join("store/predecessor");
        let destination = temporary.path().join("managed");
        std::os::unix::fs::symlink(&target, &destination).unwrap();
        let receipt = store::journal::directory::prepare(
            destination,
            target,
            store::GenerationId::new(0),
            vec!["nested/leaf".into()],
        )
        .unwrap();
        publish(&home, 0, None);
        store::write_manifest(
            &home,
            &store::Generation {
                number: store::GenerationId::new(1),
                deployments: BTreeMap::new(),
                directory_transitions: vec![receipt.clone()],
                migration: None,
            },
        )
        .unwrap();
        publish(&home, 2, None);
        std::os::unix::fs::symlink("generations/2", temporary.path().join("current")).unwrap();
        (temporary, home, receipt)
    }

    fn stages(receipt: &store::journal::directory::DirectoryTransition) -> Vec<std::path::PathBuf> {
        let wire = serde_json::to_value(receipt).unwrap();
        std::iter::once(std::path::PathBuf::from(wire["staging"].as_str().unwrap()))
            .chain(receipt.directory_staging.values().cloned())
            .collect()
    }

    #[test]
    fn interrupted_prune_keeps_authority_and_next_gc_resumes_without_a_new_prune() {
        for sealed in [false, true] {
            let (temporary, home, receipt) = retirement_fixture();
            let directory = store::generations::GenerationDirectory::open(&home).unwrap();
            directory
                .admit_manifest(&home, temporary.path(), store::GenerationId::new(1))
                .unwrap();
            if sealed {
                directory.prune(&[store::GenerationId::new(1)]).unwrap();
            } else {
                // The visible rename survived, but the process stopped before
                // acknowledging its parent barrier. Retry must seal it first.
                let generations =
                    gripsack_fs::open_dir_nofollow(&home, Path::new("generations")).unwrap();
                gripsack_fs::rename_noreplace(
                    &generations,
                    Path::new("1"),
                    Path::new(".retiring-1"),
                )
                .unwrap();
            }
            assert!(
                temporary
                    .path()
                    .join("generations/.retiring-1/manifest.json")
                    .is_file()
            );
            assert!(stages(&receipt).iter().all(|path| path.is_dir()));
            let session = crate::LifecycleSession::acquire(temporary.path()).unwrap();
            let preview = super::super::gc(&session, None, true).unwrap();
            assert!(preview.generations_removed.is_empty());
            assert!(
                temporary
                    .path()
                    .join("generations/.retiring-1/manifest.json")
                    .is_file()
            );
            assert!(stages(&receipt).iter().all(|path| path.is_dir()));
            let report = super::super::gc(&session, None, false).unwrap();
            assert!(report.generations_removed.is_empty());
            assert!(stages(&receipt).iter().all(|path| !path.exists()));
            assert!(!temporary.path().join("generations/.retiring-1").exists());
            assert!(!temporary.path().join("generations/.retired-1").exists());
            assert!(
                temporary
                    .path()
                    .join("generations/0/manifest.json")
                    .is_file()
            );
        }
    }

    #[test]
    fn retained_receipt_protects_shared_nodes_until_its_own_durable_prune() {
        let (temporary, home, receipt) = retirement_fixture();
        store::write_manifest(
            &home,
            &store::Generation {
                number: store::GenerationId::new(3),
                deployments: BTreeMap::new(),
                directory_transitions: vec![receipt.clone()],
                migration: None,
            },
        )
        .unwrap();
        let directory = store::generations::GenerationDirectory::open(&home).unwrap();
        directory
            .admit_manifest(&home, temporary.path(), store::GenerationId::new(1))
            .unwrap();
        directory.prune(&[store::GenerationId::new(1)]).unwrap();
        let session = crate::LifecycleSession::acquire(temporary.path()).unwrap();
        super::super::gc(&session, None, false).unwrap();
        assert!(stages(&receipt).iter().all(|path| path.is_dir()));
        assert!(
            temporary
                .path()
                .join("generations/3/manifest.json")
                .is_file()
        );
        // Current remains 2; dropping all noncurrent history now releases 3.
        super::super::gc(&session, Some(0), false).unwrap();
        assert!(stages(&receipt).iter().all(|path| !path.exists()));
    }

    #[test]
    fn partial_retirement_preserves_foreign_objects_and_retries_from_manifest() {
        for replacement in [false, true] {
            let (temporary, _home, receipt) = retirement_fixture();
            let nested = &receipt.directory_staging[Path::new("nested")];
            let saved = temporary.path().join("saved-owned-stage");
            if replacement {
                std::fs::rename(nested, &saved).unwrap();
                std::fs::create_dir(nested).unwrap();
            }
            let foreign = nested.join("foreign");
            std::fs::write(&foreign, b"preserve").unwrap();
            let session = crate::LifecycleSession::acquire(temporary.path()).unwrap();
            let error = super::super::gc(&session, Some(0), false).unwrap_err();
            assert!(error.to_string().contains("generations/.retiring-1"));
            assert_eq!(std::fs::read(&foreign).unwrap(), b"preserve");
            assert!(
                !stages(&receipt)[0].exists(),
                "first node was already retired"
            );
            assert!(
                temporary
                    .path()
                    .join("generations/.retiring-1/manifest.json")
                    .is_file()
            );
            std::fs::remove_file(foreign).unwrap();
            if replacement {
                // Even an empty replacement has no authority.
                assert!(super::super::gc(&session, Some(0), false).is_err());
                assert!(nested.is_dir());
                std::fs::remove_dir(nested).unwrap();
                std::fs::rename(saved, nested).unwrap();
            }
            super::super::gc(&session, Some(0), false).unwrap();
            assert!(stages(&receipt).iter().all(|path| !path.exists()));
            assert!(!temporary.path().join("generations/.retiring-1").exists());
        }
    }

    #[test]
    fn malformed_or_mismatched_retirement_never_authorizes_staging_deletion() {
        for malformed in [false, true] {
            let (temporary, home, receipt) = retirement_fixture();
            let directory = store::generations::GenerationDirectory::open(&home).unwrap();
            directory
                .admit_manifest(&home, temporary.path(), store::GenerationId::new(1))
                .unwrap();
            directory.prune(&[store::GenerationId::new(1)]).unwrap();
            let pending = temporary.path().join("generations/.retiring-1");
            if malformed {
                std::fs::write(pending.join("manifest.json"), b"{\"unknown\":true}").unwrap();
            } else {
                std::fs::rename(&pending, temporary.path().join("generations/.retiring-9"))
                    .unwrap();
            }
            let session = crate::LifecycleSession::acquire(temporary.path()).unwrap();
            for dry_run in [true, false] {
                assert!(super::super::gc(&session, Some(0), dry_run).is_err());
                assert!(stages(&receipt).iter().all(|path| path.is_dir()));
                assert!(
                    temporary
                        .path()
                        .join("generations/0/manifest.json")
                        .is_file()
                );
            }
        }
    }

    #[test]
    fn interrupted_cleanup_does_not_require_an_already_consumed_manifest() {
        let (temporary, home, receipt) = retirement_fixture();
        let directory = store::generations::GenerationDirectory::open(&home).unwrap();
        directory
            .admit_manifest(&home, temporary.path(), store::GenerationId::new(1))
            .unwrap();
        directory.prune(&[store::GenerationId::new(1)]).unwrap();
        store::journal::retire_directory_staging(&receipt, &[]).unwrap();
        let generations = gripsack_fs::open_dir_nofollow(&home, Path::new("generations")).unwrap();
        gripsack_fs::rename_noreplace(
            &generations,
            Path::new(".retiring-1"),
            Path::new(".retired-1"),
        )
        .unwrap();
        gripsack_fs::fsync_pinned_dir(&generations, Path::new("generations")).unwrap();
        std::fs::remove_file(
            temporary
                .path()
                .join("generations/.retired-1/manifest.json"),
        )
        .unwrap();
        let session = crate::LifecycleSession::acquire(temporary.path()).unwrap();
        super::super::gc(&session, None, false).unwrap();
        assert!(!temporary.path().join("generations/.retired-1").exists());
        assert!(stages(&receipt).iter().all(|path| !path.exists()));
    }

    #[test]
    fn another_pending_receipt_protects_shared_identity_during_partial_failure() {
        let (temporary, home, receipt) = retirement_fixture();
        let mut root_only = receipt.clone();
        root_only
            .directories
            .retain(|relative, _| relative.as_os_str().is_empty());
        root_only.directory_staging.clear();
        root_only.leaves = vec!["leaf".into()];
        root_only.validate().unwrap();
        store::write_manifest(
            &home,
            &store::Generation {
                number: store::GenerationId::new(3),
                deployments: BTreeMap::new(),
                directory_transitions: vec![root_only],
                migration: None,
            },
        )
        .unwrap();
        let nested = &receipt.directory_staging[Path::new("nested")];
        std::fs::write(nested.join("foreign"), b"preserve").unwrap();
        let session = crate::LifecycleSession::acquire(temporary.path()).unwrap();
        assert!(super::super::gc(&session, Some(0), false).is_err());
        assert!(
            temporary
                .path()
                .join("generations/.retiring-1/manifest.json")
                .is_file()
        );
        assert!(
            temporary
                .path()
                .join("generations/.retiring-3/manifest.json")
                .is_file()
        );
        assert!(stages(&receipt).iter().all(|path| path.is_dir()));
        std::fs::remove_file(nested.join("foreign")).unwrap();
        super::super::gc(&session, Some(0), false).unwrap();
        assert!(stages(&receipt).iter().all(|path| !path.exists()));
        assert!(!temporary.path().join("generations/.retiring-1").exists());
        assert!(!temporary.path().join("generations/.retiring-3").exists());
    }

    #[test]
    fn partial_batch_removes_descendants_before_their_retained_predecessors() {
        let temporary = tempfile::tempdir().unwrap();
        let home = gripsack_fs::open_or_create(temporary.path()).unwrap();
        publish(&home, 0, None);
        publish(&home, 1, Some(0));
        publish(&home, 2, Some(1));
        publish(&home, 3, None);
        std::os::unix::fs::symlink("generations/3", temporary.path().join("current")).unwrap();
        let directory = store::generations::GenerationDirectory::open(&home).unwrap();
        let ids = [
            store::GenerationId::new(0),
            store::GenerationId::new(1),
            store::GenerationId::new(2),
        ];
        let mut pruned = BTreeSet::from(ids);
        admit(&directory, &home, temporary.path(), &mut pruned, false).unwrap();
        // Deterministically interrupt the second rename with no-clobber
        // failure, after inventory admission but before any retirement.
        let collision = temporary.path().join("generations/.retiring-1");
        std::fs::create_dir(&collision).unwrap();
        std::fs::write(collision.join("foreign"), b"preserve").unwrap();
        assert!(directory.prune(&ids).is_err());
        assert_eq!(
            std::fs::read(collision.join("foreign")).unwrap(),
            b"preserve"
        );
        assert!(
            temporary
                .path()
                .join("generations/0/manifest.json")
                .is_file()
        );
        assert!(
            temporary
                .path()
                .join("generations/1/manifest.json")
                .is_file()
        );
        assert!(
            temporary
                .path()
                .join("generations/.retiring-2/manifest.json")
                .is_file()
        );
        std::fs::remove_file(collision.join("foreign")).unwrap();
        std::fs::remove_dir(collision).unwrap();
        let session = crate::LifecycleSession::acquire(temporary.path()).unwrap();
        super::super::gc(&session, Some(0), false).unwrap();
        assert!(!temporary.path().join("generations/0").exists());
        assert!(!temporary.path().join("generations/1").exists());
        assert!(!temporary.path().join("generations/.retiring-2").exists());
        assert!(
            temporary
                .path()
                .join("generations/3/manifest.json")
                .is_file()
        );
    }
}
