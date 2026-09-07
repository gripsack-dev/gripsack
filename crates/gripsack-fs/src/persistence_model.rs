//! The model consumes traces from the shipped primitives, not a shadow writer.
//! At any cut before directory sync a name may be old or new, but a published
//! new file must already have durable bytes AND mode. A synced directory must
//! never make a dirty child's contents authoritative. This assumes fsync's OS
//! contract; it is not a claim to simulate physical disks or controllers.

use super::*;
use fault::{Boundary as B, Edge, Event};
use std::collections::BTreeSet;

fn parent(path: &Path) -> PathBuf {
    parent_rel(path).to_owned()
}

fn check(trace: &[Event], complete: bool) -> Result<(), String> {
    let mut dirty_files = BTreeSet::new();
    let mut dirty_dirs = BTreeSet::new();
    for event in trace {
        if event.edge == Edge::Before {
            if matches!(event.boundary, B::FilePublish | B::TreePublish) && !dirty_files.is_empty()
            {
                return Err(format!("publish before file durability: {dirty_files:?}"));
            }
            if event.boundary == B::TreePublish && !dirty_dirs.is_empty() {
                return Err(format!(
                    "tree publish before child namespace durability: {dirty_dirs:?}"
                ));
            }
            continue;
        }
        match event.boundary {
            B::Write => {
                dirty_files.insert(event.path.clone());
                dirty_dirs.insert(parent(&event.path));
            }
            B::Mode => {
                dirty_files.insert(event.path.clone());
            }
            B::FileSync => {
                dirty_files.remove(&event.path);
            }
            B::DirSync => {
                dirty_dirs.remove(&event.path);
            }
            B::Mkdir | B::Symlink | B::Unlink | B::FilePublish | B::TreePublish => {
                dirty_dirs.insert(parent(&event.path));
            }
        }
    }
    if complete && (!dirty_files.is_empty() || !dirty_dirs.is_empty()) {
        return Err(format!(
            "primitive returned before durability: files={dirty_files:?}, dirs={dirty_dirs:?}"
        ));
    }
    Ok(())
}

fn every_cut(trace: &[Event]) {
    assert!(
        !trace.is_empty(),
        "the shipped primitive must actually emit operations"
    );
    for cut in 0..=trace.len() {
        check(&trace[..cut], false).unwrap();
    }
    check(trace, true).unwrap();
}

#[test]
fn atomic_file_link_and_tree_publication_are_ordered_at_every_cut() {
    let home = tempfile::tempdir().unwrap();
    let cap = open(home.path()).unwrap();
    let (result, trace) = fault::capture(false, || {
        atomic_write_with_mode(&cap, Path::new("nested/file"), b"payload", 0o600)
    });
    result.unwrap();
    every_cut(&trace);
    let (result, trace) = fault::capture(false, || {
        symlink_replace(&cap, Path::new("link"), Path::new("nested/file"))
    });
    result.unwrap();
    every_cut(&trace);
    for copy in [false, true] {
        let stage = tempfile::tempdir().unwrap();
        std::fs::create_dir(stage.path().join("sub")).unwrap();
        std::fs::write(stage.path().join("sub/file"), b"payload").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                stage.path().join("sub/file"),
                std::fs::Permissions::from_mode(0o700),
            )
            .unwrap();
        }
        let dest = PathBuf::from(if copy { "store/copied" } else { "store/moved" });
        let (result, trace) = fault::capture(copy, || publish_dir(&cap, stage.path(), &dest));
        result.unwrap();
        every_cut(&trace);
        assert_eq!(
            std::fs::read(home.path().join(&dest).join("sub/file")).unwrap(),
            b"payload"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(home.path().join(&dest).join("sub/file"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o500
            );
        }
        if copy {
            // Calibrate against the pre-0041 EXDEV bug: final mode changed
            // after file.sync_all. The model must reject that exact ordering.
            let mut mutant = trace.clone();
            let mode = mutant
                .iter()
                .position(|e| {
                    e.boundary == B::Mode && e.edge == Edge::After && e.path.is_relative()
                })
                .unwrap();
            let change = mutant.remove(mode);
            let sync = mutant
                .iter()
                .position(|e| {
                    e.boundary == B::FileSync && e.edge == Edge::After && e.path == change.path
                })
                .unwrap();
            mutant.insert(sync + 1, change);
            assert!(
                check(&mutant, true).is_err(),
                "mode-after-sync mutation escaped the model"
            );
        }
    }
}

#[test]
fn streamed_executable_publication_is_ordered_at_every_cut() {
    let home = tempfile::tempdir().unwrap();
    let cap = open(home.path()).unwrap();
    let payload = b"#!/bin/sh\nexit 0\n";
    let (result, trace) = fault::capture(false, || {
        let mut source: &[u8] = payload;
        atomic_copy_with_mode(&cap, Path::new("bin/gripsack"), &mut source, 0o755)
    });
    result.unwrap();
    every_cut(&trace);
    assert_eq!(
        std::fs::read(home.path().join("bin/gripsack")).unwrap(),
        payload
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(home.path().join("bin/gripsack"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    // Calibrated mutants — single events relocated, not an exact-sequence
    // assertion: a mode set AFTER the file fsync would let the rename
    // publish durable bytes whose mode the fsync never covered, and a
    // rename issued BEFORE the file fsync publishes dirty bytes outright.
    // The cut model must reject both orderings, not merely record them.
    let mode = trace
        .iter()
        .position(|e| e.boundary == B::Mode && e.edge == Edge::After)
        .unwrap();
    let mut mode_after_fsync = trace.clone();
    let change = mode_after_fsync.remove(mode);
    let sync = mode_after_fsync
        .iter()
        .position(|e| e.boundary == B::FileSync && e.edge == Edge::After && e.path == change.path)
        .unwrap();
    mode_after_fsync.insert(sync + 1, change);
    assert!(
        check(&mode_after_fsync, true).is_err(),
        "mode-after-fsync mutation escaped the model"
    );
    let mut premature_rename = trace.clone();
    let publish: Vec<usize> = premature_rename
        .iter()
        .enumerate()
        .filter(|(_, e)| e.boundary == B::FilePublish)
        .map(|(i, _)| i)
        .collect();
    let rename: Vec<Event> = publish
        .iter()
        .rev()
        .map(|&i| premature_rename.remove(i))
        .collect();
    assert_eq!(rename.len(), 2, "one rename: before and after edges");
    let fsync = premature_rename
        .iter()
        .position(|e| {
            e.boundary == B::FileSync && e.edge == Edge::Before && e.path == rename[0].path
        })
        .unwrap();
    for (k, event) in rename.into_iter().enumerate() {
        premature_rename.insert(fsync + k, event);
    }
    assert!(
        check(&premature_rename, true).is_err(),
        "premature-rename mutation escaped the model"
    );
}
