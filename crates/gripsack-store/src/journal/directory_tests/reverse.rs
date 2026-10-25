//! Retained incarnations, root lineage and interrupted reverse recovery.
use super::*;

#[test]
fn reverse_recovery_restores_parent_inode_before_child_priors_and_reactivation_reuses_it() {
    let (temporary, home, receipt) = fixture();
    crate::write_manifest(&home, &generation(0, Vec::new())).unwrap();
    let run = begin_run(
        &home,
        temporary.path(),
        None,
        crate::GenerationId::new(1),
        RunOp::Apply,
    )
    .unwrap();
    directory::publish(&run, &receipt).unwrap();
    write_leaf(&run, &receipt);
    crate::write_manifest(&home, &generation(1, vec![receipt.clone()])).unwrap();
    commit_run(crate::flip(run).unwrap()).unwrap();
    for commit in [false, true] {
        let run = begin_run(
            &home,
            temporary.path(),
            Some(crate::GenerationId::new(1)),
            crate::GenerationId::new(0),
            RunOp::Rollback,
        )
        .unwrap();
        directory::bind_existing(&run, &receipt).unwrap();
        let destination = receipt.destination.join("nested/command");
        let (parent, name) = receipt.leaf_parent(&destination).unwrap();
        let expected = intended();
        let captured = capture(&run, parent, name, &destination, expected.as_object()).unwrap();
        record(captured, &Intended::Removed)
            .unwrap()
            .execute(|directory, name| {
                gripsack_fs::remove_file(directory, name)?;
                gripsack_fs::fsync_dir(directory, Path::new("."))
            })
            .unwrap();
        reverse_directory(&run, &receipt).unwrap();
        assert_eq!(
            std::fs::read_link(&receipt.destination).unwrap(),
            receipt.predecessor_target
        );
        if commit {
            commit_run(crate::flip(run).unwrap()).unwrap();
            assert!(receipt.staging.is_dir());
            let reactivation = begin_run(
                &home,
                temporary.path(),
                Some(crate::GenerationId::new(0)),
                crate::GenerationId::new(1),
                RunOp::Rollback,
            )
            .unwrap();
            let expected =
                ObjectIdentity::Link(receipt.predecessor_target.to_str().unwrap().to_owned());
            directory::reactivate(&reactivation, &receipt, Some(&expected)).unwrap();
            write_leaf(&reactivation, &receipt);
            commit_run(crate::flip(reactivation).unwrap()).unwrap();
        } else {
            // Going farther back in history can remove the intermediate old
            // link. Its flat undo must run before the staged inode returns.
            let parent = receipt.parent().unwrap();
            let name = PathBuf::from(receipt.destination.file_name().unwrap());
            let expected =
                ObjectIdentity::Link(receipt.predecessor_target.to_str().unwrap().to_owned());
            let captured =
                capture(&run, parent, name, &receipt.destination, Some(&expected)).unwrap();
            record(captured, &Intended::Removed)
                .unwrap()
                .execute(|directory, name| {
                    gripsack_fs::remove_file(directory, name)?;
                    gripsack_fs::fsync_dir(directory, Path::new("."))
                })
                .unwrap();
            drop(run);
            reconcile(&home, temporary.path()).unwrap();
        }
        assert_eq!(std::fs::read(&destination).unwrap(), b"new command\n");
        receipt.leaf_parent(&destination).unwrap();
        assert!(pending_recovery(&home).unwrap().is_none());
    }
}
#[test]
fn aborted_reactivation_retains_inode_and_restores_immediate_absent_file_or_link_prior() {
    for prior in ["absent", "file", "link"] {
        let (temporary, home, receipt) = fixture();
        crate::write_manifest(&home, &generation(0, Vec::new())).unwrap();
        crate::write_manifest(&home, &generation(1, vec![receipt.clone()])).unwrap();
        std::fs::remove_file(&receipt.destination).unwrap();
        match prior {
            "file" => std::fs::write(&receipt.destination, b"earlier user file").unwrap(),
            "link" => {
                std::os::unix::fs::symlink("earlier-relative-link", &receipt.destination).unwrap()
            }
            _ => {}
        }
        let parent = receipt.parent().unwrap();
        let name = Path::new(receipt.destination.file_name().unwrap());
        let before = live_identity(&parent, name).unwrap();
        let run = begin_run(
            &home,
            temporary.path(),
            None,
            crate::GenerationId::new(1),
            RunOp::Rollback,
        )
        .unwrap();
        directory::reactivate(&run, &receipt, before.as_ref()).unwrap();
        write_leaf(&run, &receipt);
        drop(run);
        reconcile(&home, temporary.path()).unwrap();
        assert_eq!(live_identity(&parent, name).unwrap(), before);
        receipt.directories[Path::new("")]
            .verify(&parent, Path::new(receipt.staging.file_name().unwrap()))
            .unwrap();
        assert!(!receipt.staging.join("nested/command").exists());
        assert!(pending_recovery(&home).unwrap().is_none());
    }
}

#[test]
fn incarnation_exchange_restores_original_tree_before_original_child_priors() {
    for cut in 0..=4 {
        let (temporary, home, original) = fixture();
        let target = directory::prepare(
            original.destination.clone(),
            original.predecessor_target.clone(),
            crate::GenerationId::new(0),
            original.leaves.clone(),
        )
        .unwrap();
        crate::write_manifest(&home, &generation(0, Vec::new())).unwrap();
        crate::write_manifest(&home, &generation(1, vec![target.clone()])).unwrap();
        let initial = begin_run(
            &home,
            temporary.path(),
            None,
            crate::GenerationId::new(2),
            RunOp::Apply,
        )
        .unwrap();
        directory::publish(&initial, &original).unwrap();
        write_leaf(&initial, &original);
        crate::write_manifest(&home, &generation(2, vec![original.clone()])).unwrap();
        commit_run(crate::flip(initial).unwrap()).unwrap();
        let run = begin_run(
            &home,
            temporary.path(),
            Some(crate::GenerationId::new(2)),
            crate::GenerationId::new(1),
            RunOp::Rollback,
        )
        .unwrap();
        directory::bind_existing(&run, &original).unwrap();
        let destination = original.destination.join("nested/command");
        let (parent, name) = original.leaf_parent(&destination).unwrap();
        let captured = capture(&run, parent, name, &destination, intended().as_object()).unwrap();
        record(captured, &Intended::Removed)
            .unwrap()
            .execute(|directory, name| {
                gripsack_fs::remove_file(directory, name)?;
                gripsack_fs::fsync_dir(directory, Path::new("."))
            })
            .unwrap();
        reverse_directory(&run, &original).unwrap();
        if cut == 0 {
            directory::persist(
                &run,
                &target,
                Direction::Exchange {
                    original: original.clone(),
                },
            )
            .unwrap();
        } else {
            let expected =
                ObjectIdentity::Link(original.predecessor_target.to_str().unwrap().to_owned());
            directory::reactivate(&run, &target, Some(&expected)).unwrap();
            if cut >= 2 {
                let (parent, name) = target.leaf_parent(&destination).unwrap();
                let captured = capture(&run, parent, name, &destination, None).unwrap();
                let replacement = Intended::Object(ObjectIdentity::File(
                    crate::canonical_bytes_identity(b"target incarnation\n", 0o755),
                ));
                record(captured, &replacement)
                    .unwrap()
                    .execute(|directory, name| {
                        gripsack_fs::atomic_write_with_mode(
                            directory,
                            name,
                            b"target incarnation\n",
                            0o755,
                        )
                    })
                    .unwrap();
            }
            if cut == 3 {
                // Another process dies after restoring only the original
                // namespace. The still-present leaf journal must replay next.
                let records = directory::read_records(&run.journal).unwrap();
                let entries = read_uncommitted(&run.journal).unwrap();
                // Even an explicit false preparation-retention flag cannot
                // authorize deletion of a historical exchange incarnation.
                crate::journal::directory_recovery::recover_before_leaves(
                    &records, &entries, &home, false,
                )
                .unwrap();
            }
        }
        drop(run);
        if cut == 4 {
            // Insert after the empty check but before retiring the nested
            // target inode. Rename carries the child to its retained stage.
            let incoming = target.destination.join("nested/foreign");
            let _rename_hook = crate::journal::directory_nodes::before_rename(move || {
                std::fs::write(incoming, b"concurrent user addition").unwrap();
            });
            let staging = &target.directory_staging[Path::new("nested")];
            for _ in 0..2 {
                let error = reconcile(&home, temporary.path()).unwrap_err();
                assert_eq!(error.kind(), std::io::ErrorKind::DirectoryNotEmpty);
                assert!(error.to_string().contains(staging.to_str().unwrap()));
                assert_eq!(
                    std::fs::read(staging.join("foreign")).unwrap(),
                    b"concurrent user addition"
                );
                assert!(pending_recovery(&home).unwrap().is_some());
                target.directories[Path::new("nested")]
                    .verify(
                        &target.parent().unwrap(),
                        Path::new(staging.file_name().unwrap()),
                    )
                    .unwrap();
            }
            let rescued = temporary.path().join("rescued-user-addition");
            std::fs::rename(staging.join("foreign"), &rescued).unwrap();
            reconcile(&home, temporary.path()).unwrap();
            assert_eq!(std::fs::read(rescued).unwrap(), b"concurrent user addition");
        }
        reconcile(&home, temporary.path()).unwrap();
        original.leaf_parent(&destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new command\n");
        // The journal can disappear only because retained generation 1 still
        // owns every target node, not merely the root.
        let retained = crate::read_manifest(temporary.path(), crate::GenerationId::new(1)).unwrap();
        assert_eq!(retained.directory_transitions, vec![target.clone()]);
        for (relative, identity) in &target.directories {
            let staging = crate::journal::directory_nodes::stage(&target, relative).unwrap();
            identity
                .verify(
                    &target.parent().unwrap(),
                    Path::new(staging.file_name().unwrap()),
                )
                .unwrap();
        }
        assert!(pending_recovery(&home).unwrap().is_none());
        assert!(reconcile(&home, temporary.path()).unwrap().is_empty());

        // A later explicit rollback must reuse the historical incarnation;
        // deleting target staging during compensation would make this fail.
        let retry = begin_run(
            &home,
            temporary.path(),
            Some(crate::GenerationId::new(2)),
            crate::GenerationId::new(1),
            RunOp::Rollback,
        )
        .unwrap();
        directory::bind_existing(&retry, &original).unwrap();
        let (parent, name) = original.leaf_parent(&destination).unwrap();
        let captured = capture(&retry, parent, name, &destination, intended().as_object()).unwrap();
        record(captured, &Intended::Removed)
            .unwrap()
            .execute(|directory, name| {
                gripsack_fs::remove_file(directory, name)?;
                gripsack_fs::fsync_dir(directory, Path::new("."))
            })
            .unwrap();
        reverse_directory(&retry, &original).unwrap();
        let expected =
            ObjectIdentity::Link(original.predecessor_target.to_str().unwrap().to_owned());
        directory::reactivate(&retry, &retained.directory_transitions[0], Some(&expected)).unwrap();
        write_leaf(&retry, &target);
        commit_run(crate::flip(retry).unwrap()).unwrap();
        target.leaf_parent(&destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new command\n");
        assert!(pending_recovery(&home).unwrap().is_none());
    }
}
