use super::*;

fn fixture(home: &Path) -> (Snapshot, PathBuf, Vec<u8>, JobRevision) {
    let runner_root = store::content_path(home, "job-runner", &"1".repeat(64));
    std::fs::create_dir_all(&runner_root).unwrap();
    std::fs::write(runner_root.join("grip"), b"retained-runner").unwrap();
    let capture_root = store::content_path(home, "workspace-files", &"2".repeat(64));
    std::fs::create_dir_all(&capture_root).unwrap();
    let ir = serde_json::from_value(serde_json::json!({
        "ir_version":gripsack_ir::IR_VERSION,"host":{"os":"linux","arch":"x86_64"},
        "workspace":{"span":{"file":"fixture.ts","line":1},"outputs":[]}
    }))
    .unwrap();
    let snapshot = Snapshot {
        version: CAPTURE_VERSION,
        job: JobId::parse(&"a".repeat(64)).unwrap(),
        task: "one".into(),
        trigger: WorkspaceCalendar::Daily {
            time: "09:00".into(),
        },
        runner: runner_root.join("grip"),
        unit_directory: home.join("config/systemd/user"),
        checkout: home.to_owned(),
        ir,
        programs: Vec::new(),
        packages: BTreeMap::new(),
        receipts: BTreeMap::new(),
        inputs: BTreeMap::new(),
        roots: BTreeMap::from([(
            runner_root.clone(),
            store::canonical_tree_hash(&runner_root).unwrap(),
        )]),
    };
    let bytes = snapshot.encode().unwrap();
    let revision = JobRevision::parse(&store::hash::hex_sha256(&bytes)).unwrap();
    let path = capture_root.join("job.json");
    std::fs::write(&path, &bytes).unwrap();
    (snapshot, path, bytes, revision)
}

#[test]
fn capture_bytes_and_retained_runner_are_independently_bound() {
    let temporary = tempfile::tempdir().unwrap();
    let (snapshot, path, bytes, revision) = fixture(temporary.path());
    assert!(read(temporary.path(), &path, &snapshot.job, &revision).is_ok());
    let mut altered = bytes.clone();
    altered.push(b' ');
    std::fs::write(&path, &altered).unwrap();
    assert!(read(temporary.path(), &path, &snapshot.job, &revision).is_err());
    std::fs::write(&path, &bytes).unwrap();
    std::fs::write(&snapshot.runner, b"replaced-runner").unwrap();
    assert!(read(temporary.path(), &path, &snapshot.job, &revision).is_err());
}

#[test]
fn stale_job_identity_and_unretained_bindings_cannot_admit() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut snapshot, path, _, revision) = fixture(temporary.path());
    let other = JobId::parse(&"b".repeat(64)).unwrap();
    assert!(read(temporary.path(), &path, &other, &revision).is_err());
    snapshot
        .inputs
        .insert("escape".into(), PathBuf::from("/outside/retention"));
    let bytes = snapshot.encode().unwrap();
    let revision = JobRevision::parse(&store::hash::hex_sha256(&bytes)).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert!(read(temporary.path(), &path, &snapshot.job, &revision).is_err());
}

#[test]
fn oversized_capture_refuses_before_json_admission() {
    let temporary = tempfile::tempdir().unwrap();
    let (snapshot, path, _, revision) = fixture(temporary.path());
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(MAX_CAPTURE_BYTES + 1)
        .unwrap();
    assert!(read(temporary.path(), &path, &snapshot.job, &revision).is_err());
}
