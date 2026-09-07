use super::marker::*;
use super::*;

fn home() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// The home capability the journal API takes now (plan/0021) —
/// opened on the temp home; assertions below are unchanged.
fn cap(home: &tempfile::TempDir) -> Dir {
    gripsack_fs::open_or_create(home.path()).expect("home capability")
}

/// Raw-string stand-ins for identities in these tests ride the
/// Link variant (an opaque verbatim string) — typed, so only a
/// real FileIdentity fits the file arm.
fn intent(s: &str) -> Intended {
    Intended::Object(ObjectIdentity::Link(s.to_string()))
}

fn file_intent(id: crate::hash::FileIdentity) -> Intended {
    Intended::Object(ObjectIdentity::File(id))
}

/// Capture through the destination's pinned parent capability,
/// as deploy's journaled mutations do (plan/0021).
fn capture_at(dest: &Path, home: &Dir) -> Prior {
    let dir = gripsack_fs::open_or_create(dest.parent().unwrap()).unwrap();
    capture(&dir, Path::new(dest.file_name().unwrap()), dest, home).unwrap()
}

#[test]
fn crash_between_record_and_write_restores_prior() {
    let home = home();
    let dest = home.path().join("rc/.bashrc");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, b"user stuff\n").unwrap();

    let prior = capture_at(&dest, &cap(&home));
    // the intended post-state is recorded up front (0026 §6)
    record(&cap(&home), &dest, &prior, &intent("intended-hash")).unwrap();
    // simulate the crash: mutation never happened, no commit —
    // the file still holds the prior bytes

    let lines = reconcile(&cap(&home), home.path()).unwrap();
    assert_eq!(lines.len(), 1);
    // live IS the prior: the mutation never landed, so there is
    // nothing to restore — the entry is still consumed
    assert!(lines[0].message.contains("unchanged"), "{lines:?}");
    assert_eq!(std::fs::read(&dest).unwrap(), b"user stuff\n");
    assert!(reconcile(&cap(&home), home.path()).unwrap().is_empty());
}

#[test]
fn crash_after_write_restores_prior_bytes() {
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"old\n").unwrap();

    let prior = capture_at(&dest, &cap(&home));
    record(
        &cap(&home),
        &dest,
        &prior,
        &file_intent(crate::hash::canonical_bytes_identity(
            b"deployed half-run content\n",
            std::fs::metadata(&dest)
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                })
                .unwrap_or(0o644),
        )),
    )
    .unwrap();
    std::fs::write(&dest, b"deployed half-run content\n").unwrap();
    // crash: no commit_run

    let lines = reconcile(&cap(&home), home.path()).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
    assert!(lines[0].message.contains("recovered"));
}

#[test]
fn user_edit_after_crash_wins() {
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"old\n").unwrap();

    let prior = capture_at(&dest, &cap(&home));
    record(
        &cap(&home),
        &dest,
        &prior,
        &file_intent(crate::hash::canonical_bytes_identity(
            b"deployed\n",
            std::fs::metadata(&dest)
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                })
                .unwrap_or(0o644),
        )),
    )
    .unwrap();
    std::fs::write(&dest, b"deployed\n").unwrap();
    // the user edits the file AFTER the crash, before the next run
    std::fs::write(&dest, b"my own edit\n").unwrap();

    let lines = reconcile(&cap(&home), home.path()).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"my own edit\n");
    assert!(lines[0].message.contains("kept"));
}

#[test]
fn absent_prior_and_symlink_prior_recover() {
    let home = home();
    let fresh = home.path().join("fresh");
    let link = home.path().join("link");

    let prior = capture_at(&fresh, &cap(&home));
    assert_eq!(prior, Prior::Absent);
    record(
        &cap(&home),
        &fresh,
        &prior,
        &file_intent(crate::hash::canonical_bytes_identity(
            b"crashed write\n",
            std::fs::metadata(&fresh)
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                })
                .unwrap_or(0o644),
        )),
    )
    .unwrap();
    std::fs::write(&fresh, b"crashed write\n").unwrap();

    std::os::unix::fs::symlink("/original/target", &link).unwrap();
    let link_prior = capture_at(&link, &cap(&home));
    record(&cap(&home), &link, &link_prior, &intent("/deployed/target")).unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("/deployed/target", &link).unwrap();

    reconcile(&cap(&home), home.path()).unwrap();
    assert!(!fresh.exists(), "absent prior removes the crashed write");
    assert_eq!(
        std::fs::read_link(&link).unwrap().to_string_lossy(),
        "/original/target"
    );
}

#[test]
fn crash_after_flip_but_before_cleanup_reads_committed() {
    // review 5.1: the crash lands between the flip and journal
    // cleanup. The run marker names the target generation and
    // `current` reached it — the deployed content STANDS.
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"old\n").unwrap();

    begin_run(&cap(&home), None, 1, RunOp::Apply).unwrap();
    let prior = capture_at(&dest, &cap(&home));
    record(
        &cap(&home),
        &dest,
        &prior,
        &file_intent(crate::hash::canonical_bytes_identity(
            b"deployed\n",
            std::fs::metadata(&dest)
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                })
                .unwrap_or(0o644),
        )),
    )
    .unwrap();
    std::fs::write(&dest, b"deployed\n").unwrap();
    // the flip: generation 1 becomes current; commit_run never ran
    let manifest = crate::generations::Generation {
        number: 1,
        modules: Default::default(),
    };
    crate::generations::write_manifest(&cap(&home), &manifest).unwrap();
    crate::generations::flip(&cap(&home), home.path(), 1).unwrap();

    let lines = reconcile(&cap(&home), home.path()).unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.message.contains("already activated")),
        "{lines:?}"
    );
    // the committed generation's content is NOT rolled back
    assert_eq!(std::fs::read(&dest).unwrap(), b"deployed\n");
}

#[test]
fn crash_before_flip_restores() {
    // the mirror case: the marker names generation 1 but current
    // is still nothing — restore the prior
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"old\n").unwrap();
    begin_run(&cap(&home), None, 1, RunOp::Apply).unwrap();
    let prior = capture_at(&dest, &cap(&home));
    record(
        &cap(&home),
        &dest,
        &prior,
        &file_intent(crate::hash::canonical_bytes_identity(
            b"half\n",
            std::fs::metadata(&dest)
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                })
                .unwrap_or(0o644),
        )),
    )
    .unwrap();
    std::fs::write(&dest, b"half\n").unwrap();

    reconcile(&cap(&home), home.path()).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
}

#[test]
fn malformed_entries_fail_closed_into_quarantine() {
    // review 5.2: corrupt recovery metadata is quarantined and
    // BLOCKS mutation — never silently deleted
    let home = home();
    std::fs::create_dir_all(dir(home.path())).unwrap();
    std::fs::write(dir(home.path()).join("deadbeef.json"), b"{ not json").unwrap();
    let err = reconcile(&cap(&home), home.path()).unwrap_err();
    assert!(err.to_string().contains("quarantine"), "{err}");
    assert!(dir(home.path()).join("quarantine/deadbeef.json").exists());
}

#[test]
fn commit_run_clears_the_window() {
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"old\n").unwrap();
    let prior = capture_at(&dest, &cap(&home));
    record(
        &cap(&home),
        &dest,
        &prior,
        &file_intent(crate::hash::canonical_bytes_identity(
            b"new\n",
            std::fs::metadata(&dest)
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                })
                .unwrap_or(0o644),
        )),
    )
    .unwrap();
    std::fs::write(&dest, b"new\n").unwrap();

    commit_run(&cap(&home)).unwrap();
    // the flip happened: recovery must NOT undo the deploy
    assert!(reconcile(&cap(&home), home.path()).unwrap().is_empty());
    assert_eq!(std::fs::read(&dest).unwrap(), b"new\n");
}
fn manifest(n: u64) -> crate::generations::Generation {
    crate::generations::Generation {
        number: n,
        modules: Default::default(),
    }
}

#[test]
fn recovery_restores_the_exact_mode() {
    // 0027 §6: a 0600 secret replaced by a symlink mid-run, then a
    // crash — the live object at recovery is a LINK, so mode
    // preservation by copy is impossible; the mode must come from
    // the journal entry itself
    use std::os::unix::fs::PermissionsExt;
    let home = home();
    let dest = home.path().join("secret");
    std::fs::write(&dest, b"hunter2\n").unwrap();
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600)).unwrap();

    let prior = capture_at(&dest, &cap(&home));
    record(&cap(&home), &dest, &prior, &intent("/store/x")).unwrap();
    // the mutation: dest becomes an owned symlink
    std::fs::remove_file(&dest).unwrap();
    std::os::unix::fs::symlink("/store/x", &dest).unwrap();
    // crash before commit

    reconcile(&cap(&home), home.path()).unwrap();
    let meta = std::fs::metadata(&dest).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"hunter2\n");
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
}

#[test]
fn post_crash_edit_beats_the_landed_mutation() {
    // 0026 §6: intent is recorded BEFORE the mutation, so an edit
    // made after the crash is distinguishable from the mutation —
    // the user's bytes win even when the mutation fully landed
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"old\n").unwrap();

    let prior = capture_at(&dest, &cap(&home));
    record(
        &cap(&home),
        &dest,
        &prior,
        &file_intent(crate::hash::canonical_bytes_identity(
            b"deployed\n",
            std::fs::metadata(&dest)
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                })
                .unwrap_or(0o644),
        )),
    )
    .unwrap();
    std::fs::write(&dest, b"deployed\n").unwrap();
    // crash; THEN the user edits
    std::fs::write(&dest, b"post-crash edit\n").unwrap();

    let lines = reconcile(&cap(&home), home.path()).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"post-crash edit\n");
    assert!(lines[0].message.contains("kept"), "{lines:?}");
}

#[test]
fn crashed_rollback_restores_priors() {
    // 0025 §A: a rollback's target is OLDER than current — the
    // pre-0025 commit rule (current >= target) would have misread
    // a crashed rollback as committed and skipped restoration.
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"new\n").unwrap();
    crate::generations::write_manifest(&cap(&home), &manifest(2)).unwrap();
    crate::generations::write_manifest(&cap(&home), &manifest(3)).unwrap();
    crate::generations::flip(&cap(&home), home.path(), 3).unwrap();

    // rolling back 3 → 2: the restore lands, the flip never does
    begin_run(&cap(&home), Some(3), 2, RunOp::Rollback).unwrap();
    let prior = capture_at(&dest, &cap(&home));
    record(
        &cap(&home),
        &dest,
        &prior,
        &file_intent(crate::hash::canonical_bytes_identity(
            b"old\n",
            std::fs::metadata(&dest)
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                })
                .unwrap_or(0o644),
        )),
    )
    .unwrap();
    std::fs::write(&dest, b"old\n").unwrap();
    // crash: current is still 3, target was 2 — uncommitted

    let lines = reconcile(&cap(&home), home.path()).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"new\n");
    assert!(lines[0].message.contains("recovered"), "{lines:?}");
}

#[test]
fn completed_rollback_reads_committed() {
    // the flip landed (current back DOWN to the target) but
    // cleanup never ran: the restored content STANDS
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"new\n").unwrap();
    crate::generations::write_manifest(&cap(&home), &manifest(2)).unwrap();
    crate::generations::write_manifest(&cap(&home), &manifest(3)).unwrap();
    crate::generations::flip(&cap(&home), home.path(), 3).unwrap();

    begin_run(&cap(&home), Some(3), 2, RunOp::Rollback).unwrap();
    let prior = capture_at(&dest, &cap(&home));
    record(
        &cap(&home),
        &dest,
        &prior,
        &file_intent(crate::hash::canonical_bytes_identity(
            b"old\n",
            std::fs::metadata(&dest)
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                })
                .unwrap_or(0o644),
        )),
    )
    .unwrap();
    std::fs::write(&dest, b"old\n").unwrap();
    crate::generations::flip(&cap(&home), home.path(), 2).unwrap();
    // crash between the flip and commit_run

    let lines = reconcile(&cap(&home), home.path()).unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.message.contains("already activated")),
        "{lines:?}"
    );
    assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
}
