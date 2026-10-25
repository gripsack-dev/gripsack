//! Forward durable boundaries, foreign preservation and repeated restoration.
use super::*;

#[test]
fn every_forward_durable_boundary_restores_exact_link_without_touching_store() {
    for cut in 0..=5 {
        let (temporary, home, receipt) = fixture();
        let run = begin_run(
            &home,
            temporary.path(),
            None,
            crate::GenerationId::new(1),
            RunOp::Apply,
        )
        .unwrap();
        let parent = receipt.parent().unwrap();
        let name = Path::new(receipt.destination.file_name().unwrap());
        let stage = Path::new(receipt.staging.file_name().unwrap());
        if cut >= 1 {
            directory::persist(&run, &receipt, Direction::Forward).unwrap();
        }
        if cut >= 2 {
            gripsack_fs::remove_file(&parent, name).unwrap();
            gripsack_fs::fsync_dir(&parent, Path::new(".")).unwrap();
        }
        if cut >= 3 {
            gripsack_fs::rename_noreplace(&parent, stage, name).unwrap();
            gripsack_fs::fsync_dir(&parent, Path::new(".")).unwrap();
        }
        if cut >= 4 {
            crate::journal::directory_nodes::install_all(&receipt).unwrap();
            let destination = receipt.destination.join("nested/command");
            let (parent, name) = receipt.leaf_parent(&destination).unwrap();
            let captured = capture(&run, parent, name, &destination, None).unwrap();
            let intent = intended();
            let permit = record(captured, &intent).unwrap();
            if cut == 5 {
                permit
                    .execute(|directory, name| {
                        gripsack_fs::atomic_write_with_mode(
                            directory,
                            name,
                            b"new command\n",
                            0o755,
                        )
                    })
                    .unwrap();
            }
        }
        drop(run);
        reconcile(&home, temporary.path()).unwrap();
        assert_eq!(
            std::fs::read_link(&receipt.destination).unwrap(),
            receipt.predecessor_target,
            "cut {cut}"
        );
        assert_eq!(
            std::fs::read(receipt.predecessor_target.join("untouched")).unwrap(),
            b"retained payload"
        );
        assert!(!receipt.predecessor_target.join("nested").exists());
        assert!(pending_recovery(&home).unwrap().is_none());
        if cut == 0 {
            retire_directory_staging(&receipt, &[]).unwrap();
        }
        assert!(reconcile(&home, temporary.path()).unwrap().is_empty());
    }
}

#[test]
fn foreign_child_blocks_parent_removal_and_keeps_recovery_evidence() {
    let (temporary, home, receipt) = fixture();
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
    let foreign = receipt.destination.join("nested/foreign");
    std::fs::write(&foreign, b"user addition").unwrap();
    assert!(reconcile(&home, temporary.path()).is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"user addition");
    assert!(pending_recovery(&home).unwrap().is_some());
    assert!(!receipt.destination.join("nested/command").exists());
    std::fs::remove_file(&foreign).unwrap();
    reconcile(&home, temporary.path()).unwrap();
    assert_eq!(
        std::fs::read_link(&receipt.destination).unwrap(),
        receipt.predecessor_target
    );
}

#[test]
fn child_inserted_during_install_is_preserved_and_explicit_resolution_unblocks_recovery() {
    let (temporary, home, receipt) = fixture();
    let run = begin_run(
        &home,
        temporary.path(),
        None,
        crate::GenerationId::new(1),
        RunOp::Apply,
    )
    .unwrap();
    let incoming = receipt.staging.join("foreign");
    let _rename_hook = crate::journal::directory_nodes::before_rename(move || {
        std::fs::write(incoming, b"concurrent user addition").unwrap();
    });
    let error = directory::publish(&run, &receipt).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::DirectoryNotEmpty);
    assert!(
        error
            .to_string()
            .contains(receipt.destination.to_str().unwrap())
    );
    let foreign = receipt.destination.join("foreign");
    let parent = receipt.parent().unwrap();
    receipt.directories[Path::new("")]
        .verify(&parent, Path::new(receipt.destination.file_name().unwrap()))
        .unwrap();
    drop(run);
    for _ in 0..2 {
        assert!(reconcile(&home, temporary.path()).is_err());
        assert_eq!(
            std::fs::read(&foreign).unwrap(),
            b"concurrent user addition"
        );
        assert!(pending_recovery(&home).unwrap().is_some());
    }
    // The operator moves only the child out, not the owned directory inode.
    let rescued = temporary.path().join("rescued-user-addition");
    std::fs::rename(&foreign, &rescued).unwrap();
    reconcile(&home, temporary.path()).unwrap();
    assert_eq!(
        std::fs::read_link(&receipt.destination).unwrap(),
        receipt.predecessor_target
    );
    assert_eq!(
        std::fs::read(&rescued).unwrap(),
        b"concurrent user addition"
    );
    assert_eq!(
        std::fs::read(receipt.predecessor_target.join("untouched")).unwrap(),
        b"retained payload"
    );
    assert!(pending_recovery(&home).unwrap().is_none());
    assert!(reconcile(&home, temporary.path()).unwrap().is_empty());
}

#[test]
fn substituted_ancestor_cannot_redirect_child_capture_or_recovery() {
    let (temporary, home, receipt) = fixture();
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
    let moved = temporary.path().join("moved-owned");
    std::fs::rename(receipt.destination.join("nested"), &moved).unwrap();
    let foreign = temporary.path().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("command"), b"foreign bytes").unwrap();
    std::os::unix::fs::symlink(&foreign, receipt.destination.join("nested")).unwrap();
    assert!(
        receipt
            .leaf_parent(&receipt.destination.join("nested/command"))
            .is_err()
    );
    assert!(reconcile(&home, temporary.path()).is_err());
    assert_eq!(
        std::fs::read(foreign.join("command")).unwrap(),
        b"foreign bytes"
    );
    assert_eq!(
        std::fs::read(moved.join("command")).unwrap(),
        b"new command\n"
    );
    assert!(pending_recovery(&home).unwrap().is_some());
}
#[test]
fn repeated_forward_recovery_never_walks_back_into_predecessor_store() {
    for restored_parent in [false, true] {
        let (temporary, home, receipt) = fixture();
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
        std::fs::remove_file(receipt.destination.join("nested/command")).unwrap();
        std::fs::remove_dir(receipt.destination.join("nested")).unwrap();
        if restored_parent {
            std::fs::remove_dir(&receipt.destination).unwrap();
            std::os::unix::fs::symlink(&receipt.predecessor_target, &receipt.destination).unwrap();
        }
        reconcile(&home, temporary.path()).unwrap();
        assert_eq!(
            std::fs::read_link(&receipt.destination).unwrap(),
            receipt.predecessor_target
        );
        assert_eq!(
            std::fs::read(receipt.predecessor_target.join("untouched")).unwrap(),
            b"retained payload"
        );
        assert!(!receipt.predecessor_target.join("nested").exists());
    }
}
