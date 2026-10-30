//! Real common-runner checks; no manager, evaluator, network or user's schedules.
use super::*;
use crate::workspace::schedules::capture;

fn fixture(root: &Path, executable: &Path) -> Ir {
    serde_json::from_value(serde_json::json!({
        "ir_version":gripsack_ir::IR_VERSION,
        "host":{"os":"linux","arch":"x86_64"},
        "workspace":{"span":{"file":"captured.ts","line":1},"outputs":[{
            "kind":"task","name":"once","span":{"file":"captured.ts","line":2},
            "context":{"kind":"host","mutable_paths":[]},
            "steps":[{"command":{"kind":"exec","span":{"file":"captured.ts","line":3},
                "argv":[{"kind":"literal","value":executable},{"kind":"literal","value":"captured"}],
                "cwd":{"kind":"host","path":root}}}]
        }]}
    })).unwrap()
}
fn schedule() -> task_report::ScheduledInvocation {
    task_report::ScheduledInvocation {
        job: gripsack_store::jobs::JobId::parse(&"a".repeat(64)).unwrap(),
        revision: gripsack_store::jobs::JobRevision::parse(&"b".repeat(64)).unwrap(),
    }
}

#[test]
fn captured_task_executes_each_occasion_without_reading_checkout_source() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let ctx = capture::context(&home, temporary.path(), "once").unwrap();
    let ir = fixture(temporary.path(), Path::new("/bin/echo"));
    let environment = gripsack_process::OperatorEnvironment::capture().unwrap();
    let options = BuildOptions {
        environment: &environment,
        deadline: Instant::now() + std::time::Duration::from_secs(30),
    };
    let realization = realize::Realization {
        packages: BTreeMap::new(),
        inputs: BTreeMap::new(),
    };
    let staging = tempfile::tempdir().unwrap();
    let expected = capture_programs(&ir, &ctx, &options, &realization, staging.path()).unwrap();
    std::fs::write(
        temporary.path().join("gripsack.ts"),
        "throw new Error('must never evaluate timer source')",
    )
    .unwrap();
    let mut ids = Vec::new();
    for _ in 0..2 {
        let session = LifecycleSession::acquire(&home).unwrap();
        let report = run_captured(
            &ir,
            &ctx,
            &options,
            session,
            &realization,
            BTreeSet::new(),
            &expected,
            schedule(),
        )
        .unwrap();
        assert_eq!(report.status, TaskStatus::Succeeded);
        assert!(!report.root_retained);
        assert!(report.schedule.is_some());
        assert_eq!(
            task_report::read_task_log(&home, &report.invocation).unwrap(),
            b"captured\n"
        );
        ids.push(report.invocation);
    }
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn changed_external_executable_blocks_before_effects_and_releases_unlaunched_root() {
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("external");
    std::fs::copy("/bin/echo", &executable).unwrap();
    let home = temporary.path().join("home");
    let ctx = capture::context(&home, temporary.path(), "once").unwrap();
    let ir = fixture(temporary.path(), &executable);
    let environment = gripsack_process::OperatorEnvironment::capture().unwrap();
    let options = BuildOptions {
        environment: &environment,
        deadline: Instant::now() + std::time::Duration::from_secs(30),
    };
    let realization = realize::Realization {
        packages: BTreeMap::new(),
        inputs: BTreeMap::new(),
    };
    let staging = tempfile::tempdir().unwrap();
    let expected = capture_programs(&ir, &ctx, &options, &realization, staging.path()).unwrap();
    // BusyBox applet paths can contain identical bytes. Change the actual
    // captured image, not merely the source pathname used for the fixture copy.
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&executable)
        .unwrap()
        .write_all(b"changed captured image")
        .unwrap();
    let session = LifecycleSession::acquire(&home).unwrap();
    let report = run_captured(
        &ir,
        &ctx,
        &options,
        session,
        &realization,
        BTreeSet::new(),
        &expected,
        schedule(),
    )
    .unwrap();
    assert_eq!(report.status, TaskStatus::Blocked);
    assert!(report.nodes.is_empty());
    assert!(!report.root_retained);
}
