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

// --- 0045 F2: the marker parser distinguishes missing from null ---

fn parse_marker(json: &str) -> Result<RunMarker, serde_json::Error> {
    serde_json::from_str(json)
}

#[test]
fn marker_missing_previous_generation_is_rejected() {
    // The F2 defect: derived serde read a MISSING key as None, so a
    // torn marker looked like a fresh machine's first run.
    let err = parse_marker(r#"{"target_generation": 2, "op": "apply"}"#)
        .expect_err("a marker without previous_generation is torn");
    assert!(
        err.to_string().contains("previous_generation"),
        "the rejection names the missing field: {err}"
    );
}

#[test]
fn marker_null_previous_generation_is_a_fresh_machine() {
    let marker =
        parse_marker(r#"{"previous_generation": null, "target_generation": 1, "op": "apply"}"#)
            .expect("explicit null is the legitimate initial transaction");
    assert_eq!(marker.previous_generation, None);
    assert_eq!(marker.target_generation, 1);
    assert_eq!(marker.op, RunOp::Apply);

    let marker =
        parse_marker(r#"{"previous_generation": 3, "target_generation": 4, "op": "rollback"}"#)
            .expect("an explicit generation parses");
    assert_eq!(marker.previous_generation, Some(3));
    assert_eq!(marker.op, RunOp::Rollback);
}

#[test]
fn marker_rejects_duplicates_wrong_types_and_overflow() {
    for json in [
        // duplicate key
        r#"{"previous_generation": null, "previous_generation": 1, "target_generation": 2, "op": "apply"}"#,
        // wrong types
        r#"{"previous_generation": "two", "target_generation": 2, "op": "apply"}"#,
        r#"{"previous_generation": {"n": 1}, "target_generation": 2, "op": "apply"}"#,
        r#"{"previous_generation": 1, "target_generation": "2", "op": "apply"}"#,
        // u64 overflow
        r#"{"previous_generation": 18446744073709551616, "target_generation": 2, "op": "apply"}"#,
        // unknown op spelling
        r#"{"previous_generation": 1, "target_generation": 2, "op": "Apply"}"#,
        // missing the other required keys
        r#"{"previous_generation": 1, "op": "apply"}"#,
        r#"{"previous_generation": 1, "target_generation": 2}"#,
    ] {
        parse_marker(json).expect_err(&format!("must be rejected: {json}"));
    }
}

#[test]
fn marker_round_trips_through_the_wire() {
    for previous in [None, Some(7)] {
        let marker = RunMarker {
            previous_generation: previous,
            target_generation: 9,
            op: RunOp::Apply,
        };
        let bytes = serde_json::to_vec(&marker).expect("serialize");
        let back = serde_json::from_slice::<RunMarker>(&bytes).expect("own writes parse");
        assert_eq!(back.previous_generation, previous);
        assert_eq!(back.target_generation, 9);
    }
}

// --- 0045 F1: tagged identities, typed admission ---

use super::recover::{RecoveryDecision, decide_from};

#[test]
fn tagged_entries_round_trip_through_the_wire() {
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"old\n").unwrap();
    let prior = capture_at(&dest, &cap(&home));
    let after = file_intent(crate::hash::canonical_bytes_identity(b"new\n", 0o644));
    record(&cap(&home), &dest, &prior, &after).unwrap();

    let bytes = std::fs::read(home.path().join(entry_rel(&dest))).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains(r#""v":1"#), "versioned wire: {text}");
    assert!(text.contains(r#""kind":"file""#), "tagged intent: {text}");

    let entry = Entry::from_wire(&bytes).expect("own writes decode");
    assert_eq!(entry.dest, dest.to_str().unwrap());
    assert_eq!(Intended::from_wire(&entry.after), after);
}

#[test]
fn wire_admission_rejects_legacy_versions_and_malformed() {
    // the pre-0.40 shape: bare-string `after`, no version — never
    // reinterpreted (a link target could spell the removal sentinel)
    assert_eq!(
        Entry::from_wire(br#"{"dest":"/x","prior":{"kind":"absent"},"after":"gripsack:removed"}"#),
        Err(WireRejection::Legacy)
    );
    assert_eq!(
        Entry::from_wire(
            br#"{"v":2,"dest":"/x","prior":{"kind":"absent"},"after":{"kind":"removed"}}"#
        ),
        Err(WireRejection::UnsupportedVersion(2))
    );
    for malformed in [
        &br#"{"dest":"/x","prior":{"kind":"absent"},"after":{"kind":"removed"}}"#[..], // no v
        br#"{"v":1,"dest":"/x","prior":{"kind":"absent"},"after":{"kind":"dir"}}"#, // unknown kind
        br#"{"v":1,"prior":{"kind":"absent"},"after":{"kind":"removed"}}"#,         // no dest
        b"not json",
    ] {
        assert!(
            matches!(
                Entry::from_wire(malformed),
                Err(WireRejection::Malformed(_))
            ),
            "rejected as malformed: {}",
            String::from_utf8_lossy(malformed)
        );
    }
}

#[test]
fn cross_variant_identities_never_compare_equal() {
    // The F1 collision class, reduced to the kernel: a symlink whose
    // target SPELLS a file identity or the old removal sentinel is
    // still a link — typed equality keeps the variants disjoint.
    let file_id = crate::hash::canonical_bytes_identity(b"payload\n", 0o644);
    let link_spelling_file = ObjectIdentity::Link(file_id.to_string());
    let file_intended = Intended::Object(ObjectIdentity::File(file_id));
    assert_eq!(
        decide_from(Some(&link_spelling_file), &file_intended, None),
        RecoveryDecision::Keep,
        "a link spelling the intended file identity is a foreign edit"
    );
    let link_spelling_sentinel = ObjectIdentity::Link("gripsack:removed".to_string());
    assert_eq!(
        decide_from(Some(&link_spelling_sentinel), &Intended::Removed, None),
        RecoveryDecision::Keep,
        "a link spelling the old removal sentinel is a user object, never a landed removal"
    );
}

#[test]
fn post_crash_sentinel_symlink_is_kept() {
    // The handoff's semantic witness, end to end through reconcile:
    // the run intended a removal; after the crash the user creates a
    // symlink whose target is the pre-0.40 sentinel string.
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"old\n").unwrap();
    let prior = capture_at(&dest, &cap(&home));
    record(&cap(&home), &dest, &prior, &Intended::Removed).unwrap();
    std::fs::remove_file(&dest).unwrap(); // the removal landed
    std::os::unix::fs::symlink("gripsack:removed", &dest).unwrap(); // post-crash user edit

    let lines = reconcile(&cap(&home), home.path()).unwrap();
    assert!(lines[0].message.contains("kept"), "{lines:?}");
    assert_eq!(
        std::fs::read_link(&dest).unwrap(),
        Path::new("gripsack:removed"),
        "the user's symlink is untouched"
    );
}

#[test]
fn legacy_untagged_entries_fail_closed_with_evidence_kept() {
    let home = home();
    let dest = home.path().join("config");
    std::fs::write(&dest, b"old\n").unwrap();
    std::fs::create_dir_all(dir(home.path())).unwrap();
    let legacy = format!(
        r#"{{"dest": "{}", "prior": {{"kind": "file", "hash": "{}", "mode": 420}}, "after": "gripsack:removed"}}"#,
        dest.display(),
        "ab".repeat(32)
    );
    std::fs::write(home.path().join(entry_rel(&dest)), legacy).unwrap();

    let err = reconcile(&cap(&home), home.path())
        .expect_err("legacy entries block recovery instead of being guessed");
    assert!(err.to_string().contains("pre-0.40"), "{err}");
    assert_eq!(std::fs::read(&dest).unwrap(), b"old\n");
    let quarantined: Vec<_> = std::fs::read_dir(dir(home.path()).join("quarantine"))
        .unwrap()
        .collect();
    assert_eq!(quarantined.len(), 1, "the evidence is retained");
}

#[test]
fn non_utf8_link_targets_are_refused_not_lossy_compared() {
    use std::os::unix::ffi::OsStrExt;
    let home = home();
    let dest = home.path().join("link");
    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(b"\xff\xfe"), &dest).unwrap();
    let err = live_identity(&cap(&home), Path::new("link"))
        .expect_err("a non-UTF-8 target refuses observation");
    assert!(err.to_string().contains("non-UTF-8"), "{err}");
    assert!(
        std::fs::symlink_metadata(&dest)
            .unwrap()
            .file_type()
            .is_symlink(),
        "refusal preserves the object"
    );
}

#[test]
fn non_utf8_destinations_are_refused_at_record() {
    use std::os::unix::ffi::OsStrExt;
    let home = home();
    let dest = PathBuf::from(std::ffi::OsStr::from_bytes(b"/non-\xff-utf8"));
    let err = record(&cap(&home), &dest, &Prior::Absent, &Intended::Removed)
        .expect_err("a non-UTF-8 destination cannot be journaled");
    assert!(err.to_string().contains("UTF-8"), "{err}");
    assert!(
        pending_recovery(&cap(&home)).unwrap().is_none(),
        "no journal state was created"
    );
}
