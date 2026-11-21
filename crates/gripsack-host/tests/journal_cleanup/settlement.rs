//! #364: MID is a durable Gripsack write, not a permissible foreign edit.
use super::*;
use gripsack_host::storage::Metadata;

const PRIVATE: Permissions = Permissions::new(0o600);

/// Run the two real leaf commands; kill immediately after the *second*
/// destination rename, before its parent barrier. The v2 record is already
/// durable, TWO is visible, but power can still return MID.
fn interrupted_repeated_leaf() -> MemFs {
    let mut repeated = dest("dest", ONE, TWO);
    repeated.first = Some(MID);
    let scenario = Scenario::one(repeated);
    let fs = home(&scenario);
    let probe = fs.snapshot();
    apply(&probe, &scenario).unwrap();
    let cut = probe
        .trace()
        .iter()
        .enumerate()
        .filter(|(_, e)| e.boundary == Boundary::FilePublish && e.name == Some(name("dest")))
        .nth(1)
        .expect("the second destination rename")
        .0;
    fs.crash_at(cut + 1);
    assert!(MemFs::is_crash(&apply(&fs, &scenario).unwrap_err()));
    let killed = fs.kill();
    assert_eq!(content_at(&killed, "dest"), Ok(TWO));
    let (cold, _) = killed.power_loss(|_| false);
    assert_eq!(content_at(&cold, "dest"), Ok(MID));
    let dir = journal(&cold).unwrap();
    assert_eq!(
        cold.read(&dir, &name("dest.json")).unwrap(),
        encode_entry("dest", ONE, MID, TWO)
    );
    run_recovery(&cold).unwrap();
    assert_eq!(content_at(&cold, "dest"), Ok(ONE));
    killed
}

/// Ordinary writer operations, *not* MemFs::inject (which is durable by
/// contract). Neither the editor's data/mode nor its rename has been synced.
fn foreign_replacement(fs: &MemFs, link: bool) -> Metadata {
    let root = fs.root().unwrap();
    let tmp = name(".editor");
    if link {
        fs.symlink(&root, &tmp, Path::new(USER)).unwrap();
    } else {
        let mut file = fs.create_file(&root, &tmp, DEST_MODE).unwrap();
        fs.write(&mut file, USER.as_bytes()).unwrap();
        fs.set_mode(&file, PRIVATE).unwrap();
    }
    fs.rename(&root, &tmp, &root, &name("dest")).unwrap();
    fs.metadata(&root, &name("dest")).unwrap()
}

fn assert_foreign(fs: &MemFs, expected: Metadata) {
    let root = fs.root().unwrap();
    let dest = name("dest");
    assert_eq!(fs.metadata(&root, &dest).unwrap(), expected);
    match expected.kind {
        ObjectKind::File => {
            assert_eq!(expected.permissions, PRIVATE);
            assert_eq!(fs.read(&root, &dest).unwrap(), USER.as_bytes());
        }
        ObjectKind::Symlink => assert_eq!(fs.read_link(&root, &dest).unwrap(), Path::new(USER)),
        other => panic!("unexpected foreign {other:?}"),
    }
}

#[test]
fn visible_foreign_edit_does_not_authorize_retirement() {
    let fs = interrupted_repeated_leaf();
    let foreign = foreign_replacement(&fs, false);
    assert_foreign(&fs, foreign);
    // Old Preserve acknowledged this observation without an effect. The
    // ordinary two *journal* barriers alone cannot settle the destination.
    let journal = journal(&fs).unwrap();
    cleanup(&fs, &journal, &[name("dest.json")], Cleanup::TwoBarriers).unwrap();
    let (cold, _) = fs.power_loss(|_| false);
    assert!(journal_records(&cold).is_empty());
    run_recovery(&cold).unwrap();
    assert_eq!(content_at(&cold, "dest"), Ok(MID));
}

#[test]
fn preserved_file_or_link_survives_cleanup_and_power_loss() {
    for link in [false, true] {
        let fs = interrupted_repeated_leaf();
        let foreign = foreign_replacement(&fs, link);
        let original_record = encode_entry("dest", ONE, MID, TWO);
        explore(
            &fs,
            CrashKinds::Both,
            PendingBound::default(),
            |image| {
                let result = run_recovery(image).map_err(RecoverError::into_io)?;
                assert!(matches!(result.notes.as_slice(), [note] if note.settled == Settled::Kept));
                Ok(())
            },
            |image, cut| {
                let records = journal_records(&image);
                if cut.completed {
                    assert!(records.is_empty());
                }
                if let Some((_, bytes)) = records.iter().find(|(at, _)| at == &name("dest.json")) {
                    assert_eq!(bytes, &original_record);
                } else {
                    // Every cut that loses the entry must have the exact
                    // user's inode, not MID or an empty/torn foreign file.
                    assert_foreign(&image, foreign);
                }
                Ok(())
            },
        )
        .unwrap();
    }
}

#[test]
fn failed_or_interrupted_preserve_keeps_original_evidence() {
    let image = interrupted_repeated_leaf();
    let foreign = foreign_replacement(&image, false);
    let evidence = journal_records(&image);
    let start = image.effects();
    let probe = image.snapshot();
    run_recovery(&probe).unwrap();
    let trace = probe.trace();
    assert_eq!(trace[start].boundary, Boundary::FileSync);
    assert_eq!(trace[start + 1].boundary, Boundary::DirSync);
    assert_eq!(trace[start + 2].name, Some(name("dest.json")));
    assert_eq!(trace[start + 2].boundary, Boundary::Unlink);

    for barrier in [start, start + 1] {
        let fs = image.snapshot();
        fs.fail_at(barrier, Errno::IO);
        let error = run_recovery(&fs).unwrap_err();
        assert_eq!(
            error.io().unwrap().raw_os_error(),
            Some(Errno::IO.raw_os_error())
        );
        assert_foreign(&fs, foreign);
        assert_eq!(journal_records(&fs), evidence);
        // Another process can still see the volatile edit after kill. A
        // subsequent power loss instead exposes MID: the record must work.
        let killed = fs.kill();
        assert_foreign(&killed, foreign);
        let (cold, _) = killed.power_loss(|_| false);
        assert_eq!(content_at(&cold, "dest"), Ok(MID));
        assert_eq!(journal_records(&cold), evidence);
        run_recovery(&cold).unwrap();
        assert_eq!(content_at(&cold, "dest"), Ok(ONE));
        assert!(journal_records(&cold).is_empty());
    }
    // Lost process after both settlement barriers, before the first unlink.
    image.crash_at(start + 2);
    let error = run_recovery(&image).unwrap_err();
    assert!(MemFs::is_crash(error.io().unwrap()));
    let killed = image.kill();
    for restart in [killed.snapshot(), killed.power_loss(|_| false).0] {
        assert_eq!(journal_records(&restart), evidence);
        assert_foreign(&restart, foreign);
        run_recovery(&restart).unwrap();
        assert_foreign(&restart, foreign);
        assert!(journal_records(&restart).is_empty());
    }
}

#[test]
fn syncing_a_displaced_descriptor_cannot_settle_its_replacement() {
    let fs = interrupted_repeated_leaf();
    foreign_replacement(&fs, false);
    let evidence = journal_records(&fs);
    let root = fs.root().unwrap();
    let probe = fs.snapshot();
    let start = probe.operations().len();
    run_recovery(&probe).unwrap();
    let sync = probe.operations()[start..]
        .iter()
        .position(|op| op.boundary == Some(Boundary::FileSync))
        .unwrap();
    // Identical bytes and mode, different inode: syncing the open fd is not
    // evidence about the file which now owns the name.
    fs.inject(
        start + sync,
        ExternalWrite {
            dir: root,
            name: name("dest"),
            bytes: USER.as_bytes().to_vec(),
            permissions: PRIVATE,
        },
    );
    assert!(run_recovery(&fs).is_err());
    assert!(!fs.injection_pending());
    assert_eq!(journal_records(&fs), evidence);
    assert_eq!(fs.read(&root, &name("dest")).unwrap(), USER.as_bytes());
}
