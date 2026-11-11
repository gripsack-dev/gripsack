//! `src/fs/persistence_model.rs`, ported onto `MemFs`.
//!
//! The original checks traces with a dirty-set rule ("no publish before the
//! file barrier, no return before the namespace barrier"). Here the same
//! protocols run against the storage model and the property is stated on
//! outcomes: at every cut, after a kill or any power-loss subset, a
//! published name is absent/old or complete (bytes *and* mode), and once
//! the protocol has returned it survives every power loss. The calibration
//! mutants of the original must still be rejected.

mod support;

use gripsack_host::storage::{
    CrashKinds, CrashPoint, ExploreError, MemDir, MemFs, PendingBound, Permissions, Storage,
    explore,
};
use rustix::io::Errno;
use std::io;
use support::{
    ExistingDirs, STAGED, WriteOrder, atomic_write, create_dir_all, file_at, name, symlink_replace,
    walk,
};

fn explore_all(
    initial: &MemFs,
    protocol: impl Fn(&MemFs) -> io::Result<()>,
    check: impl FnMut(MemFs, &CrashPoint) -> Result<(), String>,
) -> Result<(), ExploreError> {
    explore(
        initial,
        CrashKinds::Both,
        PendingBound::default(),
        protocol,
        check,
    )
    .map(drop)
}

/// The name at `dir/file` is absent (or, after return, must exist) and,
/// when present, has exactly `bytes` and `mode`.
fn published(
    fs: &MemFs,
    point: &CrashPoint,
    dir: &[&str],
    file: &str,
    bytes: &[u8],
    mode: Permissions,
) -> Result<(), String> {
    match file_at(fs, dir, file) {
        None if point.completed => Err(format!("{file} lost after the protocol returned")),
        None => Ok(()),
        Some((found, found_mode)) if found == bytes && found_mode == mode => Ok(()),
        Some((found, found_mode)) => Err(format!(
            "{file} published torn: {} bytes, mode {found_mode}",
            found.len()
        )),
    }
}

fn staged_publication(path: &'static [&'static str], file: &'static str, mode: Permissions) {
    let payload: &[u8] = b"#!/bin/sh\nexit 0\n";
    let run = |order| {
        explore_all(
            &MemFs::new(),
            |fs| {
                let dir = create_dir_all(fs, path, ExistingDirs::Seal)?;
                atomic_write(fs, &dir, file, payload, mode, order)
            },
            |fs, point| published(&fs, point, path, file, payload, mode),
        )
    };
    run(WriteOrder::Shipped).unwrap_or_else(|error| panic!("{error}"));
    for mutant in [WriteOrder::ModeAfterSync, WriteOrder::RenameBeforeSync] {
        let error = run(mutant).expect_err("calibration mutant escaped the model");
        assert!(error.violation().is_some(), "{mutant:?}: {error}");
    }
}

#[test]
fn atomic_file_publication_is_ordered_at_every_cut() {
    // atomic_write_with_mode(nested/file, 0600)
    staged_publication(&["nested"], "file", Permissions::new(0o600));
}

#[test]
fn streamed_executable_publication_is_ordered_at_every_cut() {
    // atomic_copy_with_mode(bin/gripsack, 0755)
    staged_publication(&["bin"], "gripsack", Permissions::new(0o755));
}

#[test]
fn symlink_replacement_is_atomic_at_every_cut() {
    let initial = MemFs::new();
    let root = initial.root().unwrap();
    initial
        .symlink(&root, &name("link"), "old".as_ref())
        .unwrap();
    let initial = initial.settled();
    explore_all(
        &initial,
        |fs| symlink_replace(fs, &fs.root()?, "link", "nested/file"),
        |fs, point| {
            let target = fs
                .read_link(&fs.root().unwrap(), &name("link"))
                .map_err(|e| format!("link lost: {e}"))?;
            match target.to_str() {
                Some("nested/file") => Ok(()),
                Some("old") if !point.completed => Ok(()),
                other => Err(format!("link names {other:?}")),
            }
        },
    )
    .unwrap_or_else(|error| panic!("{error}"));
}

/// `$TMPDIR/stage/sub/file` (0700), built by a fetch and never synced.
fn staging() -> MemFs {
    let fs = MemFs::new();
    let tmp = create_dir_all(&fs, &["tmp", "stage", "sub"], ExistingDirs::Trust).unwrap();
    let mut file = fs
        .create_file(&tmp, &name("file"), Permissions::new(0o700))
        .unwrap();
    fs.write(&mut file, b"payload").unwrap();
    fs.kill() // the builder's process is gone; its writes sit in the cache
}

/// `read_only_files` then `fsync_tree` over the staging tree.
fn seal_staging(fs: &MemFs, stage: &MemDir) -> io::Result<()> {
    let sub = fs.open_dir(stage, &name("sub"))?;
    let file = fs.open_file(&sub, &name("file"))?;
    let mode = fs.metadata(&sub, &name("file"))?.permissions.bits();
    fs.set_mode(&file, Permissions::new(mode & !0o222))?;
    fs.sync_file(&file)?;
    fs.sync_dir(&sub)?;
    fs.sync_dir(stage)
}

/// `copy_into_dir`: per-file create, write, mode, fsync; then the dir.
fn copy_into(fs: &MemFs, stage: &MemDir, sibling: &str, order: WriteOrder) -> io::Result<()> {
    let copy = create_dir_all(fs, &["store", sibling], ExistingDirs::Seal)?;
    let sub = create_dir_all(fs, &["store", sibling, "sub"], ExistingDirs::Seal)?;
    let from = fs.open_dir(stage, &name("sub"))?;
    let bytes = fs.read(&from, &name("file"))?;
    let mode = fs.metadata(&from, &name("file"))?.permissions;
    let mut file = fs.create_file(&sub, &name("file"), STAGED)?;
    fs.write(&mut file, &bytes)?;
    if order != WriteOrder::ModeAfterSync {
        fs.set_mode(&file, mode)?;
    }
    fs.sync_file(&file)?;
    if order == WriteOrder::ModeAfterSync {
        fs.set_mode(&file, mode)?;
    }
    fs.sync_dir(&sub)?;
    fs.sync_dir(&copy)
}

/// `publish_dir`: the same-filesystem rename path, or (`copy`) the EXDEV
/// fallback through a temp sibling in the store.
fn publish_dir(fs: &MemFs, copy: bool, order: WriteOrder) -> io::Result<()> {
    let root = fs.root()?;
    let tmp = fs.open_dir(&root, &name("tmp"))?;
    let stage = fs.open_dir(&tmp, &name("stage"))?;
    seal_staging(fs, &stage)?;
    let store = create_dir_all(fs, &["store"], ExistingDirs::Seal)?;
    if copy {
        copy_into(fs, &stage, ".publish-copied", order)?;
        fs.rename(&store, &name(".publish-copied"), &store, &name("copied"))?;
    } else {
        fs.rename_noreplace(&tmp, &name("stage"), &store, &name("moved"))?;
    }
    fs.sync_dir(&store)
}

#[test]
fn tree_publication_is_ordered_at_every_cut() {
    for copy in [false, true] {
        let dest = if copy { "copied" } else { "moved" };
        let run = |order| {
            explore_all(
                &staging(),
                |fs| publish_dir(fs, copy, order),
                |fs, point| {
                    let root = fs.root().unwrap();
                    match walk(&fs, &root, &["store", dest]) {
                        Err(_) if !point.completed => Ok(()),
                        Err(error) => Err(format!("{dest} lost after return: {error}")),
                        Ok(_) => published(
                            &fs,
                            &CrashPoint {
                                completed: true,
                                ..point.clone()
                            },
                            &["store", dest, "sub"],
                            "file",
                            b"payload",
                            Permissions::new(0o500),
                        ),
                    }
                },
            )
        };
        run(WriteOrder::Shipped).unwrap_or_else(|error| panic!("copy={copy}: {error}"));
        if copy {
            // the pre-0041 EXDEV bug: final mode changed after file.sync_all
            let error = run(WriteOrder::ModeAfterSync).expect_err("mode-after-sync escaped");
            assert!(error.violation().is_some(), "{error}");
        }
    }
}

#[test]
fn observed_directory_birth_is_sealed_before_nested_publication_returns() {
    // an interrupted parent-creation attempt: two mkdirs, no syncs, killed
    let attempt = MemFs::new();
    let root = attempt.root().unwrap();
    attempt
        .create_dir(&root, &name("first"), Permissions::new(0o755))
        .unwrap();
    let first = attempt.open_dir(&root, &name("first")).unwrap();
    attempt
        .create_dir(&first, &name("second"), Permissions::new(0o755))
        .unwrap();
    let successor = attempt.kill();
    let run = |existing| {
        explore_all(
            &successor,
            |fs| {
                let dir = create_dir_all(fs, &["first", "second"], existing)?;
                atomic_write(
                    fs,
                    &dir,
                    "receipt",
                    b"admitted payload",
                    Permissions::new(0o644),
                    WriteOrder::Shipped,
                )
            },
            |fs, point| {
                published(
                    &fs,
                    point,
                    &["first", "second"],
                    "receipt",
                    b"admitted payload",
                    Permissions::new(0o644),
                )
            },
        )
    };
    run(ExistingDirs::Seal).unwrap_or_else(|error| panic!("{error}"));
    let error = run(ExistingDirs::Trust).expect_err("trusting a visible parent escaped");
    let violation = error.violation().expect("a violation");
    assert!(violation.point.completed, "{violation}");
}

#[test]
fn ambient_directory_sync_failure_prevents_consumer_effect() {
    // open_or_create(first/second): seal the existing root path, then act
    let attempt = MemFs::new();
    create_dir_all(&attempt, &["first", "second"], ExistingDirs::Trust).unwrap();
    let initial = attempt.kill();
    let consumer = |fs: &MemFs| -> io::Result<()> {
        let dir = create_dir_all(fs, &["first", "second"], ExistingDirs::Seal)?;
        atomic_write(
            fs,
            &dir,
            "effect",
            b"authorized after sealing",
            Permissions::new(0o644),
            WriteOrder::Shipped,
        )
    };
    let effect = |fs: &MemFs| file_at(fs, &["first", "second"], "effect");

    let refused = initial.snapshot();
    refused.fail_at(refused.effects(), Errno::IO);
    assert!(
        consumer(&refused).is_err(),
        "unsealed root granted authority"
    );
    assert!(effect(&refused).is_none());

    let killed = initial.snapshot();
    killed.crash_at(killed.effects());
    assert!(MemFs::is_crash(&consumer(&killed).unwrap_err()));
    assert!(effect(&killed.kill()).is_none());

    let sealed = initial.snapshot();
    consumer(&sealed).unwrap();
    let (survivor, _) = sealed.power_loss(|_| false);
    assert_eq!(
        effect(&survivor).map(|(bytes, _)| bytes),
        Some(b"authorized after sealing".to_vec())
    );
}
