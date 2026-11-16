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

#[test]
fn captured_selectors_replay_without_live_versions_and_keep_historical_braces() {
    use crate::workspace::{BuildOptions, consumer, paths, schedules::capture};
    use gripsack_ir::workspace_model::{PackageOutput, WorkspaceOutput, lock::ResolvedPinFields};
    use std::{
        collections::BTreeSet,
        sync::Arc,
        time::{Duration, Instant},
    };

    // Exercise the real retained-package reader and native captured runner,
    // without installing a timer or claiming systemd-manager coverage.
    for (authored, expected) in [(true, "resolved\n"), (false, "literal\n")] {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().join("home");
        let checkout = temporary.path().join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        let ctx = capture::context(&home, &checkout, "one").unwrap();
        let session = crate::LifecycleSession::acquire(&home).unwrap();
        let source = temporary.path().join("source");
        let selector = "dir-{version.bare}/{os}";
        for (directory, bytes) in [
            ("dir-7/linux", b"resolved\n".as_slice()),
            (selector, b"literal\n".as_slice()),
        ] {
            std::fs::create_dir_all(source.join(directory)).unwrap();
            std::fs::write(source.join(directory).join("value"), bytes).unwrap();
        }
        let tree = store::canonical_tree_hash(&source).unwrap();
        let root = store::content_path(&home, "workspace-source", tree.as_str());
        crate::source::publish(&ctx, "workspace-source", &source, &root).unwrap();
        let span = serde_json::json!({"file":"captured.ts","line":1});
        let target = serde_json::json!({"os":"linux","arch":"x86_64"});
        let declaration: PackageOutput = serde_json::from_value(serde_json::json!({
            "name":"library","span":span,
            "producer":{"kind":"provider","provider":{"kind":"fetch",
                "span":span,"fetch":{"kind":"file","path":source}}},
            "commands":{},"target":target,"layout":{"kind":"relocatable"}
        }))
        .unwrap();
        let pin = ResolvedPinFields {
            version: Some("v7".into()),
            tree256: Some(tree.to_string()),
            ..Default::default()
        };
        let package = artifact::publish_package(
            &ctx,
            &session,
            &declaration,
            Arc::new(artifact::Artifact {
                root: root.clone(),
                payload: root.clone(),
                tree,
                retention: BTreeSet::from([root]),
            }),
            artifact::ProducerIdentity {
                pin: &pin,
                conda: None,
            },
            Vec::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        let binding = if authored {
            paths::SelectorBinding::LockedSource
        } else {
            paths::SelectorBinding::Concrete
        };
        let live = Realization {
            packages: BTreeMap::from([("library", Arc::clone(&package))]),
            inputs: BTreeMap::new(),
            selector_binding: binding,
        };
        let artifact_file = serde_json::json!({
            "kind":"artifact","output":"library","selector":format!("{selector}/value")
        });
        let artifact_directory = serde_json::json!({
            "kind":"artifact","output":"library","selector":selector
        });
        let mut ir: Ir = serde_json::from_value(serde_json::json!({
            "ir_version":gripsack_ir::IR_VERSION,"host":{"os":"linux","arch":"x86_64"},
            "workspace":{"span":span,"outputs":[
                {"kind":"environment","name":"env","span":span,"packages":[],"target":target,
                 "env":{"VALUE":artifact_file}},
                {"kind":"task","name":"one","span":span,"environment":"env",
                 "context":{"kind":"host","mutable_paths":[]},"checks":["verify"],
                 "steps":[{"command":{"kind":"exec","span":span,"argv":[
                     {"kind":"literal","value":"/bin/sh"},{"kind":"literal","value":"-c"},
                     {"kind":"literal","value":"cat \"$VALUE\" \"$1\" value"},
                     {"kind":"literal","value":"captured"},artifact_file],
                     "cwd":artifact_directory}}]},
                {"kind":"check","name":"verify","span":span,"subject":"library",
                 "run":{"kind":"exec","span":span,"argv":[
                     {"kind":"literal","value":"/bin/sh"},{"kind":"literal","value":"-c"},
                     {"kind":"literal","value":"cat \"$CHECK\" value"}],
                     "env":{"CHECK":{"kind":"source","selector":format!("{selector}/value")}},
                     "cwd":{"kind":"source","selector":selector}}}
            ]}
        }))
        .unwrap();
        ir.workspace
            .outputs
            .push(WorkspaceOutput::Package(declaration));
        paths::freeze_selectors(&mut ir, &live).unwrap();
        let environment = gripsack_host::process::OperatorEnvironment::capture().unwrap();
        let options = BuildOptions {
            environment: &environment,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        let staging = tempfile::tempdir().unwrap();
        let programs =
            consumer::capture_programs(&ir, &ctx, &options, &live, staging.path()).unwrap();
        let (mut snapshot, file, _, _) = fixture(&home);
        snapshot.ir = ir;
        snapshot.programs = programs;
        snapshot.checkout = checkout.clone();
        snapshot.packages.insert("library".into(), package.identity);
        artifact::hook::capture(&home, &package, &mut snapshot.receipts).unwrap();
        let mut closure = BTreeSet::new();
        package.retain_into(&mut closure);
        for root in &closure {
            snapshot
                .roots
                .insert(root.clone(), store::canonical_tree_hash(root).unwrap());
        }
        let bytes = snapshot.encode().unwrap();
        let revision = JobRevision::parse(&store::hash::hex_sha256(&bytes)).unwrap();
        std::fs::write(&file, bytes).unwrap();
        let snapshot = read(&home, &file, &snapshot.job, &revision).unwrap();
        let restored = snapshot.realization(&home).unwrap();
        std::fs::write(
            checkout.join("gripsack.ts"),
            "throw new Error('do not evaluate')",
        )
        .unwrap();
        std::fs::write(checkout.join("gripsack.lock"), "not a live lock").unwrap();
        let report = consumer::run_captured(
            &snapshot.ir,
            &ctx,
            &options,
            session,
            &restored,
            closure,
            &snapshot.programs,
            consumer::ScheduledInvocation {
                job: snapshot.job.clone(),
                revision,
            },
        )
        .unwrap();
        assert_eq!(
            report.status,
            crate::TaskStatus::Succeeded,
            "{:?}",
            report.error
        );
        assert_eq!(
            crate::read_task_log(&home, &report.invocation).unwrap(),
            expected.repeat(5).as_bytes(),
        );
        assert!(!report.root_retained);
    }
}
