use super::*;

fn record() -> Receipt {
    Receipt {
        version: 1,
        manifest: "a".repeat(64),
        realized_tree: "b".repeat(64),
        payload: "b-package-payload".into(),
        trust_revision: "c".repeat(64),
        key_id: "publisher".into(),
        conda: None,
        external: BTreeMap::new(),
        environment: None,
    }
}

#[test]
fn generated_record_writer_and_reader_share_one_complete_closure_bound() {
    let temporary = tempfile::tempdir().unwrap();
    let mut expected = record();
    // Exercise a record larger than the previous single-manifest-derived 5 MiB
    // read cap. A production multi-manifest witness lives in runtime_tests.
    expected.key_id = "x".repeat(6 * 1024 * 1024);
    write_record(temporary.path(), &expected).unwrap();
    let directory = gripsack_fs::open(temporary.path()).unwrap();
    let restored = read_record(&directory).unwrap();
    assert_eq!(restored.key_id, expected.key_id);
    assert_eq!(restored.manifest, expected.manifest);
}

#[test]
fn oversized_generated_record_cannot_create_or_replace_a_receipt() {
    let temporary = tempfile::tempdir().unwrap();
    let mut oversized = record();
    oversized.key_id = "x".repeat(MAX_MATERIALIZATION_RECORD_BYTES);
    assert!(
        write_record(temporary.path(), &oversized)
            .unwrap_err()
            .to_string()
            .contains("byte bound")
    );
    assert!(!temporary.path().join(RECORD_FILE).exists());
    let original = record();
    write_record(temporary.path(), &original).unwrap();
    let previous = std::fs::read(temporary.path().join(RECORD_FILE)).unwrap();
    assert!(write_record(temporary.path(), &oversized).is_err());
    assert_eq!(
        std::fs::read(temporary.path().join(RECORD_FILE)).unwrap(),
        previous
    );
}

#[test]
fn retained_record_cap_is_checked_before_reading_or_decoding() {
    let temporary = tempfile::tempdir().unwrap();
    let file = std::fs::File::create(temporary.path().join(RECORD_FILE)).unwrap();
    file.set_len(MAX_MATERIALIZATION_RECORD_BYTES as u64 + 1)
        .unwrap();
    let directory = gripsack_fs::open(temporary.path()).unwrap();
    assert!(
        read_record(&directory)
            .err()
            .unwrap()
            .to_string()
            .contains("bounded regular file")
    );
}
