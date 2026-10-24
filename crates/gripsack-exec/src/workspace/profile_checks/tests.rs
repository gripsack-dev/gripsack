use super::*;
use gripsack_ir::workspace_model::{WorkspaceArg, WorkspaceCommand, WorkspacePath};
use serde_json::json;

fn span() -> Span {
    Span {
        file: "files.ts".into(),
        line: 17,
        col: Some(3),
    }
}
fn command(selector: &str) -> WorkspaceCommand {
    WorkspaceCommand::Exec {
        span: span(),
        argv: vec![
            WorkspaceArg::Literal {
                value: "/usr/bin/test".into(),
            },
            WorkspaceArg::Source {
                selector: selector.into(),
            },
            WorkspaceArg::Literal {
                value: "/same/path".into(),
            },
        ],
        env: BTreeMap::from([(
            "INPUT".into(),
            WorkspaceArg::Source {
                selector: selector.into(),
            },
        )]),
        cwd: None,
    }
}

#[test]
fn file_subject_replaces_only_typed_positions() {
    let bound = bindings::bind(&command("."), Path::new("/captured/file")).unwrap();
    let WorkspaceCommand::Exec { argv, env, .. } = bound else {
        unreachable!()
    };
    assert_eq!(
        argv[1],
        WorkspaceArg::Literal {
            value: "/captured/file".into()
        }
    );
    assert_eq!(
        argv[2],
        WorkspaceArg::Literal {
            value: "/same/path".into()
        }
    );
    assert_eq!(env["INPUT"], argv[1]);
}

#[test]
fn file_subject_rejects_children_and_directory_context() {
    assert!(bindings::bind(&command("child"), Path::new("/file")).is_err());
    let mut value = command(".");
    let WorkspaceCommand::Exec { cwd, .. } = &mut value else {
        unreachable!()
    };
    *cwd = Some(WorkspacePath::Source {
        selector: ".".into(),
    });
    assert!(bindings::bind(&value, Path::new("/file")).is_err());
}

fn profile(subject: &str, stage: &str, source: bool) -> ProfileOutput {
    let mut file = json!({"span":span(), "content":{"kind":"literal","text":"value"},
        "destination":{"kind":"tracked_copy","path":"~/.checked"},
        "checks":[{"check":"validate","subject":subject,"stage":stage,"span":span()}]});
    if source {
        file["source"] = json!({"kind":"repo_file","path":"template"});
    }
    serde_json::from_value(json!({"name":"personal","span":span(),"files":[file]})).unwrap()
}

#[test]
fn stage_admission_preserves_render_before_deploy_contract() {
    for stage in ["post_link", "post_activate"] {
        assert!(admit(&profile("rendered", stage, true)).is_err());
        assert!(admit(&profile("source", stage, true)).is_ok());
        assert!(admit(&profile("deployed", stage, true)).is_ok());
    }
    assert!(admit(&profile("rendered", "pre_flip", false)).is_ok());
    assert!(admit(&profile("source", "pre_flip", false)).is_err());
    assert!(admit(&profile("deployed", "pre_flip", false)).is_ok());
}

#[test]
fn captured_subject_binds_bytes_and_source_attribution() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source"), "original").unwrap();
    let inputs = FileCheckInputs {
        check: "validate".into(),
        subject: FileCheckSubject::Source,
        stage: FileCheckStage::PostActivate,
        span: span(),
        file_span: span(),
        captured: "source".into(),
        command_span: span(),
        captured_digest: gripsack_store::canonical_file_hash(&root.path().join("source")).unwrap(),
        destination: root.path().join("destination"),
    };
    assert_eq!(
        inputs.path(root.path()).unwrap(),
        root.path().join("source")
    );
    let wire = serde_json::to_vec(&inputs).unwrap();
    let retained: FileCheckInputs = serde_json::from_slice(&wire).unwrap();
    assert_eq!(retained.file_span, span());
    std::fs::write(root.path().join("source"), "changed").unwrap();
    assert!(retained.observe(root.path()).is_err());
}

#[cfg(unix)]
#[test]
fn mode_change_and_destination_retarget_invalidate_observation() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::write(&source, "original").unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&source, &destination).unwrap();
    let inputs = FileCheckInputs {
        check: "validate".into(),
        subject: FileCheckSubject::Deployed,
        stage: FileCheckStage::PreFlip,
        span: span(),
        file_span: span(),
        captured: "source".into(),
        command_span: span(),
        captured_digest: gripsack_store::canonical_file_hash(&source).unwrap(),
        destination: destination.clone(),
    };
    let observation = inputs.observe(root.path()).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(observation.unchanged().is_err());
    assert!(inputs.observe(root.path()).is_err());
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    let observation = inputs.observe(root.path()).unwrap();
    let other = root.path().join("other");
    std::fs::write(&other, "original").unwrap();
    std::fs::remove_file(&destination).unwrap();
    symlink(other, &destination).unwrap();
    assert!(observation.unchanged().is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn native_file_checks_preserve_operation_deadline() {
    use gripsack_process::{Control, ProcessDisposition};
    use std::time::Duration;
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source"), "captured").unwrap();
    let environment = OperatorEnvironment::capture().unwrap();
    let detected = crate::facts::detect();
    let facts = gripsack_ir::HostFacts {
        os: detected.os.into(),
        arch: detected.arch.into(),
        libc: detected.libc.clone(),
        tags: Vec::new(),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let host = consumer::admit::NativeContext::new(&facts, home.path(), deadline).unwrap();
    let realization = Realization {
        recipes: BTreeMap::new(),
        packages: BTreeMap::new(),
        inputs: BTreeMap::new(),
    };
    let options = super::super::BuildOptions {
        environment: &environment,
        bridge: None,
        worker: Default::default(),
        deadline,
    };
    let command = WorkspaceCommand::Exec {
        span: span(),
        argv: ["/bin/sleep", "30"]
            .into_iter()
            .map(|value| WorkspaceArg::Literal {
                value: value.into(),
            })
            .collect(),
        env: BTreeMap::new(),
        cwd: None,
    };
    let inputs = FileCheckInputs {
        check: "bounded".into(),
        subject: FileCheckSubject::Source,
        stage: FileCheckStage::PreFlip,
        span: span(),
        file_span: span(),
        captured: "source".into(),
        command_span: span(),
        captured_digest: gripsack_store::canonical_file_hash(&root.path().join("source")).unwrap(),
        destination: root.path().join("destination"),
    };
    let context = hooks::snapshot::capture_file(
        &command,
        &command,
        inputs,
        &realization,
        home.path(),
        &host,
        &options,
    )
    .unwrap();
    let result = hooks::run_context(
        &context,
        root.path(),
        home.path(),
        None,
        &environment,
        Instant::now() + Duration::from_millis(100),
        Some(gripsack_process::ProcessLeases {
            worker: None,
            retention: Some(std::fs::File::open(root.path()).unwrap()),
        }),
        |_| Control::Continue,
    )
    .unwrap();
    assert!(!result.success);
    assert_eq!(result.receipt.disposition, ProcessDisposition::Deadline);
}
