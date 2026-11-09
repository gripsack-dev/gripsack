//! The MemFs model itself, one assertion per storage-model clause.
use super::*;

fn name(text: &str) -> Name {
    Name::new(text).unwrap()
}

fn exists(fs: &MemFs, dir: &MemDir, at: &str) -> bool {
    fs.metadata(dir, &name(at)).is_ok()
}

/// A file `f` in root, created and written but nothing synced.
fn unsynced_file(fs: &MemFs) -> MemFile {
    let root = fs.root().unwrap();
    let mut file = fs
        .create_file(&root, &name("f"), Permissions::new(0o600))
        .unwrap();
    fs.write(&mut file, b"bytes").unwrap();
    file
}

fn all_lost(fs: &MemFs) -> MemFs {
    fs.power_loss(|_| false).0
}

#[test]
fn s2_file_sync_makes_bytes_durable_but_not_the_name() {
    let fs = MemFs::new();
    let file = unsynced_file(&fs);
    fs.sync_file(&file).unwrap();
    let after = all_lost(&fs);
    let root = after.root().unwrap();
    assert!(
        !exists(&after, &root, "f"),
        "name needs the directory barrier"
    );

    let root = fs.root().unwrap();
    fs.sync_dir(&root).unwrap();
    let after = all_lost(&fs);
    assert_eq!(after.read(&root, &name("f")).unwrap(), b"bytes");
}

#[test]
fn s2_dir_sync_alone_publishes_an_empty_file() {
    let fs = MemFs::new();
    let _file = unsynced_file(&fs);
    let root = fs.root().unwrap();
    fs.sync_dir(&root).unwrap();
    let after = all_lost(&fs);
    assert_eq!(after.read(&root, &name("f")).unwrap(), b"");
}

#[test]
fn s3_kill_keeps_the_page_cache_and_its_pending_updates() {
    let fs = MemFs::new();
    let _file = unsynced_file(&fs);
    let killed = fs.kill();
    let root = killed.root().unwrap();
    assert_eq!(killed.read(&root, &name("f")).unwrap(), b"bytes");
    assert_eq!(killed.pending_updates(), fs.pending_updates());
    // a later power loss can still drop what the dead process left unsynced
    assert!(!exists(&all_lost(&killed), &root, "f"));
}

#[test]
fn s3_power_loss_is_not_prefix_closed() {
    let fs = MemFs::new();
    let root = fs.root().unwrap();
    for at in ["a", "b"] {
        fs.create_dir(&root, &name(at), Permissions::new(0o755))
            .unwrap();
    }
    // keep only the second update: `b` survives although `a` came first
    let (after, lost) = fs.power_loss(|index| index == 1);
    assert!(!exists(&after, &root, "a"));
    assert!(exists(&after, &root, "b"));
    assert_eq!(lost, vec![0]);
}

#[test]
fn s1_same_directory_rename_is_one_update() {
    let fs = MemFs::new();
    let root = fs.root().unwrap();
    let file = unsynced_file(&fs);
    fs.sync_file(&file).unwrap();
    fs.sync_dir(&root).unwrap();
    fs.rename(&root, &name("f"), &root, &name("g")).unwrap();
    assert_eq!(fs.pending_updates(), 1);
    for keep in [false, true] {
        let (after, _) = fs.power_loss(|_| keep);
        // never both names, never neither
        assert_eq!(exists(&after, &root, "f"), !keep);
        assert_eq!(exists(&after, &root, "g"), keep);
    }
}

#[test]
fn cross_directory_rename_needs_both_barriers() {
    let fs = MemFs::new();
    let root = fs.root().unwrap();
    fs.create_dir(&root, &name("d"), Permissions::new(0o755))
        .unwrap();
    let file = unsynced_file(&fs);
    fs.sync_file(&file).unwrap();
    fs.sync_dir(&root).unwrap();
    let d = fs.open_dir(&root, &name("d")).unwrap();
    fs.rename(&root, &name("f"), &d, &name("f")).unwrap();
    assert_eq!(fs.pending_updates(), 2);
    // the unlink half alone survives: the file is reachable nowhere
    let (after, _) = fs.power_loss(|index| index == 1);
    assert!(!exists(&after, &root, "f"));
    assert!(!exists(&after, &d, "f"));
    fs.sync_dir(&d).unwrap();
    fs.sync_dir(&root).unwrap();
    let after = all_lost(&fs);
    assert!(exists(&after, &d, "f"));
}

#[test]
fn remove_tree_is_a_sequence_of_unlinks() {
    let fs = MemFs::new();
    let root = fs.root().unwrap();
    fs.create_dir(&root, &name("t"), Permissions::new(0o755))
        .unwrap();
    let t = fs.open_dir(&root, &name("t")).unwrap();
    for at in ["x", "y"] {
        let file = fs
            .create_file(&t, &name(at), Permissions::new(0o600))
            .unwrap();
        fs.sync_file(&file).unwrap();
    }
    let fs = fs.settled();
    let effects = fs.effects();
    fs.crash_at(effects + 1);
    assert!(MemFs::is_crash(
        &fs.remove_tree(&root, &name("t")).unwrap_err()
    ));
    let killed = fs.kill();
    assert!(exists(&killed, &root, "t"));
    assert_eq!(killed.read_dir(&t).unwrap(), vec![name("y")]);
}

#[test]
fn a_crashed_process_cannot_observe_or_act() {
    let fs = MemFs::new();
    fs.crash_at(0);
    let root = MemDir { ino: ROOT };
    assert!(MemFs::is_crash(
        &fs.create_dir(&root, &name("a"), Permissions::new(0o755))
            .unwrap_err()
    ));
    assert!(fs.crashed());
    assert!(MemFs::is_crash(&fs.read_dir(&root).unwrap_err()));
    let killed = fs.kill();
    assert!(killed.read_dir(&root).unwrap().is_empty());
}

#[test]
fn an_injected_failure_has_no_effect_and_no_crash() {
    let fs = MemFs::new();
    let root = fs.root().unwrap();
    fs.fail_at(0, Errno::IO);
    let error = fs
        .create_dir(&root, &name("a"), Permissions::new(0o755))
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(Errno::IO.raw_os_error()));
    assert!(!exists(&fs, &root, "a"));
    fs.create_dir(&root, &name("a"), Permissions::new(0o755))
        .unwrap();
    assert_eq!(fs.effects(), 1);
}

#[test]
fn explore_counts_cuts_and_images() {
    let initial = MemFs::new();
    let exploration = explore(
        &initial,
        CrashKinds::Both,
        PendingBound::default(),
        |fs| {
            let root = fs.root()?;
            fs.create_dir(&root, &name("a"), Permissions::new(0o755))?;
            fs.sync_dir(&root)
        },
        |_, _| Ok(()),
    )
    .unwrap();
    // cuts 0,1,2; images: kill each, plus 1 + 2 + 1 power-loss subsets
    assert_eq!(exploration.cuts, 3);
    assert_eq!(exploration.images, 3 + 4);
}

#[test]
fn explore_reports_returned_before_durability() {
    let initial = MemFs::new();
    let error = explore(
        &initial,
        CrashKinds::PowerLoss,
        PendingBound::default(),
        |fs| {
            let root = fs.root()?;
            fs.create_dir(&root, &name("a"), Permissions::new(0o755))
        },
        |fs, point| {
            let root = fs.root().unwrap();
            if point.completed && !exists(&fs, &root, "a") {
                Err("returned, then lost".into())
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    let violation = error.violation().expect("a violation");
    assert!(violation.point.completed);
    assert_eq!(violation.point.crash, Crash::PowerLoss { lost: vec![0] });
}
