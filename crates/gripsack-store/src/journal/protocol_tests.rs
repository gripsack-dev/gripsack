//! Consumer-visible authority and error boundaries through the real journal.
use super::{
    Intended, ObjectIdentity, RunOp, begin_run, capture, end_run, live_identity, pending_recovery,
    reconcile, record,
};
use gripsack_fs::Dir;
use std::io;
use std::path::{Path, PathBuf};

fn fixture() -> (tempfile::TempDir, Dir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let home = gripsack_fs::open_or_create(temporary.path()).unwrap();
    let destination = temporary.path().join("parent/destination");
    gripsack_fs::atomic_write_with_mode(
        &home,
        Path::new("parent/destination"),
        b"original\n",
        0o600,
    )
    .unwrap();
    (temporary, home, destination)
}

fn replacement() -> Intended {
    Intended::Object(ObjectIdentity::File(crate::canonical_bytes_identity(
        b"deployed\n",
        0o640,
    )))
}

#[test]
fn changed_destination_cannot_gain_a_mutation_permit() {
    let (temporary, home, destination) = fixture();
    let run = begin_run(
        &home,
        temporary.path(),
        None,
        crate::GenerationId::new(1),
        RunOp::Apply,
    )
    .unwrap();
    let parent = gripsack_fs::open(destination.parent().unwrap()).unwrap();
    let name = PathBuf::from("destination");
    let expected = live_identity(&parent, &name).unwrap();
    gripsack_fs::atomic_write_with_mode(&parent, &name, b"foreign\n", 0o600).unwrap();
    let result = capture(&run, parent, name, &destination, expected.as_ref());
    assert!(
        result.is_err(),
        "changed_destination_gained_mutation_authority"
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"foreign\n");
    let pending = pending_recovery(&home).unwrap().unwrap();
    assert!(pending.run_marker);
    assert_eq!(pending.entries, 0);
}

#[test]
fn permitted_mutation_uses_the_captured_parent() {
    let (temporary, home, destination) = fixture();
    let run = begin_run(
        &home,
        temporary.path(),
        None,
        crate::GenerationId::new(1),
        RunOp::Apply,
    )
    .unwrap();
    let parent = gripsack_fs::open(destination.parent().unwrap()).unwrap();
    let name = PathBuf::from("destination");
    let expected = live_identity(&parent, &name).unwrap();
    let captured = capture(&run, parent, name, &destination, expected.as_ref()).unwrap();
    let retained = temporary.path().join("retained-parent");
    std::fs::rename(destination.parent().unwrap(), &retained).unwrap();
    std::fs::create_dir(destination.parent().unwrap()).unwrap();
    std::fs::write(&destination, b"foreign\n").unwrap();
    let intended = replacement();
    let result = record(captured, &intended)
        .unwrap()
        .execute(|directory, name| {
            gripsack_fs::atomic_write_with_mode(directory, name, b"deployed\n", 0o640)
        });
    assert!(
        result.is_ok(),
        "captured_parent_lost_mutation_authority: {result:?}"
    );
    assert_eq!(
        std::fs::read(retained.join("destination")).unwrap(),
        b"deployed\n"
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"foreign\n");
    assert_eq!(pending_recovery(&home).unwrap().unwrap().entries, 1);
}

#[test]
fn mutation_error_retains_evidence_and_recovers_the_original() {
    let (temporary, home, destination) = fixture();
    let run = begin_run(
        &home,
        temporary.path(),
        None,
        crate::GenerationId::new(1),
        RunOp::Apply,
    )
    .unwrap();
    let parent = gripsack_fs::open(destination.parent().unwrap()).unwrap();
    let name = PathBuf::from("destination");
    let expected = live_identity(&parent, &name).unwrap();
    let captured = capture(&run, parent, name, &destination, expected.as_ref()).unwrap();
    let intended = replacement();
    let result = record(captured, &intended)
        .unwrap()
        .execute(|directory, name| {
            gripsack_fs::atomic_write_with_mode(directory, name, b"deployed\n", 0o640)?;
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fixture effect error",
            ))
        });
    assert!(
        matches!(result, Err(ref error) if error.kind() == io::ErrorKind::PermissionDenied),
        "mutation_error_was_lost: {result:?}"
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"deployed\n");
    let pending = pending_recovery(&home).unwrap().unwrap();
    assert!(pending.run_marker);
    assert_eq!(pending.entries, 1);
    drop(run);
    reconcile(&home, temporary.path()).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"original\n");
    assert!(pending_recovery(&home).unwrap().is_none());
}

#[test]
fn unfinished_mutation_cannot_end_the_run() {
    let (temporary, home, destination) = fixture();
    let run = begin_run(
        &home,
        temporary.path(),
        None,
        crate::GenerationId::new(1),
        RunOp::Apply,
    )
    .unwrap();
    let parent = gripsack_fs::open(destination.parent().unwrap()).unwrap();
    let name = PathBuf::from("destination");
    let expected = live_identity(&parent, &name).unwrap();
    let captured = capture(&run, parent, name, &destination, expected.as_ref()).unwrap();
    drop(record(captured, &replacement()).unwrap());
    let marker = temporary.path().join("journal/run.json");
    let before = std::fs::read(&marker).unwrap();
    assert!(
        end_run(run).is_err(),
        "unfinished_mutation_lost_its_run_marker"
    );
    assert_eq!(std::fs::read(marker).unwrap(), before);
    assert_eq!(std::fs::read(&destination).unwrap(), b"original\n");
    assert_eq!(pending_recovery(&home).unwrap().unwrap().entries, 1);
    reconcile(&home, temporary.path()).unwrap();
    assert!(pending_recovery(&home).unwrap().is_none());
}
