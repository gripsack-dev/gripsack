use super::*;
use crate::facts::HostFacts;
use crate::semantic::{Difference, capture_evaluated, capture_ir, capture_profile, compare};
use gripsack_ir::workspace_model::lock::{WORKSPACE_LOCK_VERSION, WorkspaceLock};
use gripsack_store::{
    source_bundle::{SourceBundle, SourceCapturePolicy},
    trust::evaluation::EvaluationId,
};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

struct Fixture {
    _temporary: tempfile::TempDir,
    repo: PathBuf,
    frontend: PathBuf,
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        let frontend = temporary.path().join("frontend");
        let home = temporary.path().join("home");
        for directory in [&repo, &frontend, &home] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(
            repo.join("gripsack.ts"),
            "// immutable declaration source\n",
        )
        .unwrap();
        fs::write(frontend.join("driver.ts"), "// captured compiler\n").unwrap();
        fs::write(repo.join("selected"), "selected input bytes").unwrap();
        fs::write(repo.join("unrelated"), "unrelated input bytes").unwrap();
        Self {
            _temporary: temporary,
            repo,
            frontend,
            home,
        }
    }
    fn repository(&self, receipt: char) -> Repository {
        let sources = SourceBundle::capture(
            &self.repo,
            &self.frontend,
            None,
            &self.home,
            SourceCapturePolicy::default(),
        )
        .unwrap();
        Repository::evaluated_with_inputs(
            Arc::new(sources),
            EvaluationId::parse(&receipt.to_string().repeat(64)).unwrap(),
            HostFacts {
                os: "linux",
                arch: "x86_64",
                libc: Some("glibc-2.36".into()),
                hostname: "trusted-fixture".into(),
            },
            BTreeMap::from([("available".into(), true), ("absent".into(), false)]),
        )
    }
}
fn ir() -> Ir {
    let span = json!({"file":"gripsack.ts","line":7,"col":3});
    serde_json::from_value(json!({"ir_version":gripsack_ir::IR_VERSION,"host":{"os":"linux","arch":"x86_64"},
        "workspace":{"span":span,"inputs":[
            {"name":"selected","span":span,"origin":{"kind":"repo_file","path":"selected"}},
            {"name":"unrelated","span":span,"origin":{"kind":"repo_file","path":"unrelated"}},
            {"name":"missing","span":span,"origin":{"kind":"repo_file","path":"missing"}}
        ],"outputs":[
            {"kind":"hook","name":"use-selected","span":span,"trigger":"post_link",
             "run":{"kind":"exec","span":span,"argv":[{"kind":"literal","value":"/bin/cat"},{"kind":"input","input":"selected"}]}},
            {"kind":"profile","name":"chosen","span":span,"hooks":["use-selected"]},
            {"kind":"profile","name":"other","span":span,"files":[{"span":span,
             "content":{"kind":"literal","text":"1234"},"destination":{"kind":"tracked_copy","path":"~/.private"}}]}
        ]}})).unwrap()
}
fn node<'a>(snapshot: &'a SemanticSnapshot, id: &str) -> &'a SnapshotNode {
    snapshot
        .document()
        .nodes
        .iter()
        .find(|node| node.id.0 == id)
        .unwrap()
}
fn fact<'a>(snapshot: &'a SemanticSnapshot, id: &str, path: &str) -> &'a Fact {
    node(snapshot, id)
        .facts
        .iter()
        .find(|fact| fact.path.0 == path)
        .unwrap()
}

#[test]
fn trusted_observations_and_immutable_inputs_replace_only_available_unknowns() {
    let fixture = Fixture::new();
    let repository = fixture.repository('a');
    fs::write(
        fixture.repo.join("selected"),
        "mutated live file must not be read",
    )
    .unwrap();
    let ir = ir();
    let captured = capture_evaluated(&ir, &repository, None, SnapshotOrigin::Recorded).unwrap();
    for path in [
        "/facts",
        "/facts/libc",
        "/probes/available",
        "/probes/absent",
        "/definitions/frontend",
        "/source_bundle",
    ] {
        assert!(
            fact(&captured, "workspace", path).knowledge.is_known(),
            "{path}"
        );
    }
    assert_eq!(
        fact(&captured, "input/selected", "/captured_bytes").knowledge,
        KnowledgeState::Known {
            digest: DefinitionDigest::parse(
                &gripsack_process::Sha256Digest::of(b"selected input bytes").to_string()
            )
            .unwrap()
        }
    );
    assert!(matches!(
        fact(&captured, "input/missing", "/captured_bytes").knowledge,
        KnowledgeState::Unknown {
            reason: UnknownReason::MissingInput
        }
    ));
    let direct = capture_ir(&ir, None, SnapshotOrigin::Recorded).unwrap();
    for (id, path) in [
        ("workspace", "/facts"),
        ("workspace", "/probes"),
        ("workspace", "/definitions"),
        ("input/selected", "/captured_bytes"),
    ] {
        assert!(!fact(&direct, id, path).knowledge.is_known());
    }
    let Repository::Evaluated {
        sources, receipt, ..
    } = repository
    else {
        unreachable!()
    };
    let incomplete = capture_evaluated(
        &ir,
        &Repository::evaluated(sources, receipt),
        None,
        SnapshotOrigin::Recorded,
    )
    .unwrap();
    assert!(
        !fact(&incomplete, "workspace", "/facts")
            .knowledge
            .is_known()
    );
    assert!(
        !fact(&incomplete, "workspace", "/probes")
            .knowledge
            .is_known()
    );
}

#[test]
fn profile_scope_contains_only_transitive_dependencies_and_their_inputs() {
    let fixture = Fixture::new();
    let repository = fixture.repository('a');
    let pins = crate::workspace::pins::WorkspacePins::read(&repository).unwrap();
    let ir = ir();
    let snapshot = capture_profile(&ir, &repository, &pins, "chosen", None).unwrap();
    let names: Vec<_> = snapshot
        .document()
        .nodes
        .iter()
        .map(|node| node.id.0.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "input/selected",
            "output/chosen",
            "output/use-selected",
            "workspace"
        ]
    );
    assert!(
        fact(&snapshot, "workspace", "/lock_presence")
            .knowledge
            .is_known()
    );
    assert!(
        !snapshot
            .document()
            .completeness
            .unknown_reasons
            .contains(&UnknownReason::MissingLock)
    );
    assert!(!snapshot.document().completeness.redacted);
    assert!(
        !snapshot
            .document()
            .completeness
            .unknown_reasons
            .contains(&UnknownReason::MissingInput)
    );
    assert!(
        !snapshot
            .document()
            .completeness
            .unknown_reasons
            .contains(&UnknownReason::MissingProbe)
    );
    let mut changed = serde_json::to_value(&ir).unwrap();
    changed["workspace"]["outputs"][2]["files"][0]["content"]["text"] =
        json!("changed unrelated profile");
    changed["workspace"]["inputs"][1]["origin"]["path"] = json!("different-unrelated-input");
    let after = capture_profile(
        &serde_json::from_value(changed).unwrap(),
        &repository,
        &pins,
        "chosen",
        None,
    )
    .unwrap();
    assert_eq!(snapshot.document().nodes, after.document().nodes);
    assert_eq!(snapshot.document().edges, after.document().edges);
}

#[test]
fn complete_selected_profile_does_not_inherit_unknown_or_redacted_unselected_evidence() {
    let fixture = Fixture::new();
    let repository = fixture.repository('a');
    let pins = crate::workspace::pins::WorkspacePins::read(&repository).unwrap();
    let mut ir = ir();
    let gripsack_ir::workspace_model::WorkspaceOutput::Profile(profile) =
        &mut ir.workspace.outputs[1]
    else {
        unreachable!()
    };
    profile.hooks.clear();
    let captured = capture_profile(&ir, &repository, &pins, "chosen", None).unwrap();
    assert!(captured.document().completeness.complete);
    assert!(!captured.document().completeness.redacted);
    assert!(captured.document().completeness.unknown_reasons.is_empty());
}

#[test]
fn source_and_span_changes_are_provenance_not_attempt_or_config_identity() {
    let fixture = Fixture::new();
    let repository = fixture.repository('a');
    let ir = ir();
    let before = capture_evaluated(&ir, &repository, None, SnapshotOrigin::Recorded).unwrap();
    let same_source_new_attempt = fixture.repository('b');
    let after = capture_evaluated(
        &ir,
        &same_source_new_attempt,
        None,
        SnapshotOrigin::Recorded,
    )
    .unwrap();
    assert_eq!(before.document().nodes, after.document().nodes);
    fs::write(fixture.repo.join("unrelated"), "unrelated change").unwrap();
    let changed_repository = fixture.repository('c');
    let pins = crate::workspace::pins::WorkspacePins::read(&repository).unwrap();
    let old = capture_profile(&ir, &repository, &pins, "chosen", None).unwrap();
    let mut changed = ir.clone();
    changed.workspace.span.line += 1;
    let new = capture_profile(&changed, &changed_repository, &pins, "chosen", None).unwrap();
    let changes = compare(&old, &new);
    assert!(
        changes
            .changes
            .iter()
            .filter(|change| change.difference != Difference::Unknown)
            .all(|change| change.kind == ChangeKind::Provenance)
    );
    assert_eq!(node(&old, "output/chosen").provenance[0].line, 7);
    assert_eq!(
        node(&old, "output/chosen").provenance[0]
            .source_label
            .as_deref(),
        Some("gripsack.ts")
    );
    let encoded = serde_json::to_string(&before).unwrap();
    for forbidden in [
        "\"1234\"".to_owned(),
        digest(b"\"1234\"").to_string(),
        gripsack_process::Sha256Digest::of(b"1234").to_string(),
        fixture.repo.to_string_lossy().into_owned(),
        "a".repeat(64),
    ] {
        assert!(
            !encoded.contains(&forbidden),
            "private/volatile evidence leaked: {forbidden}"
        );
    }
    assert!(
        node(&before, "output/other")
            .facts
            .iter()
            .any(|fact| matches!(
                fact.knowledge,
                KnowledgeState::Unknown {
                    reason: UnknownReason::Redacted
                }
            ))
    );
}

#[test]
fn admitted_lock_and_confirmed_local_pin_absence_stay_distinct_from_missing_external_pin() {
    let fixture = Fixture::new();
    let repository = fixture.repository('a');
    let mut ir = ir();
    let package = |name: &str, path: &str| {
        serde_json::from_value(json!({
        "kind":"package","name":name,"span":{"file":"gripsack.ts","line":9},
        "producer":{"kind":"provider","provider":{"kind":"fetch","fetch":{"kind":"file","path":path},"span":{"file":"gripsack.ts","line":9}}},
        "commands":{},"target":{"os":"linux","arch":"x86_64"},"layout":{"kind":"relocatable"}
    })).unwrap()
    };
    ir.workspace.outputs.push(package("local", "selected"));
    ir.workspace
        .outputs
        .push(package("external", "/outside/selected"));
    let captured = capture_evaluated(&ir, &repository, None, SnapshotOrigin::Recorded).unwrap();
    assert!(
        fact(&captured, "output/local", "/lock")
            .knowledge
            .is_known()
    );
    assert!(
        !fact(&captured, "output/external", "/lock")
            .knowledge
            .is_known()
    );
    assert!(
        !fact(&captured, "output/local", "/realized_artifact")
            .knowledge
            .is_known()
    );
    let mut lock = WorkspaceLock {
        lock_version: WORKSPACE_LOCK_VERSION,
        definitions: crate::workspace::captured_definitions(&repository).unwrap(),
        resolutions: BTreeMap::new(),
    };
    let locked = capture_ir(&ir, Some(&lock), SnapshotOrigin::Recorded).unwrap();
    assert!(
        fact(&locked, "workspace", "/definitions/frontend")
            .knowledge
            .is_known()
    );
    assert!(
        !fact(&locked, "output/local", "/lock").knowledge.is_known(),
        "direct IR cannot infer captured-file authority"
    );
    use gripsack_ir::workspace_model::{
        WorkspaceOutput, WorkspaceProducer,
        lock::{LockedPin, PlatformResolution, ResolvedPinFields, platform_key},
    };
    let WorkspaceOutput::Package(package) = ir.workspace.outputs.last().unwrap() else {
        unreachable!()
    };
    let WorkspaceProducer::Provider { provider } = &package.producer else {
        unreachable!()
    };
    lock.resolutions.insert(
        platform_key(&package.target),
        PlatformResolution {
            pins: vec![LockedPin {
                output: package.name.clone(),
                source: provider.locked(),
                resolved: ResolvedPinFields {
                    tree256: Some("d".repeat(64)),
                    ..Default::default()
                },
                conda: None,
            }],
            transitive: Vec::new(),
        },
    );
    let pinned = capture_ir(&ir, Some(&lock), SnapshotOrigin::Recorded).unwrap();
    assert!(
        fact(&pinned, "output/external", "/lock/resolved/tree256")
            .knowledge
            .is_known()
    );
    assert!(
        fact(&pinned, "output/external", "/package_identity")
            .knowledge
            .is_known()
    );
}

#[test]
fn directory_input_selection_excludes_unrelated_bytes_and_resolves_alias_children() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.repo.join("tree")).unwrap();
    fs::write(fixture.repo.join("tree/keep"), "selected directory member").unwrap();
    fs::write(fixture.repo.join("tree/skip"), "excluded directory member").unwrap();
    std::os::unix::fs::symlink("tree", fixture.repo.join("alias")).unwrap();
    let repository = fixture.repository('a');
    let mut ir = ir();
    ir.workspace.inputs[0].origin = InputOrigin::RepoDirectory {
        path: "alias".into(),
        include: vec!["keep".into(), "skip".into()],
        exclude: vec!["skip".into()],
    };
    ir.workspace.inputs[1].origin = InputOrigin::RepoFile {
        path: "alias/keep".into(),
    };
    let before = capture_evaluated(&ir, &repository, None, SnapshotOrigin::Recorded).unwrap();
    assert!(
        fact(&before, "input/selected", "/captured_bytes/keep")
            .knowledge
            .is_known()
    );
    assert!(
        fact(&before, "input/unrelated", "/captured_bytes")
            .knowledge
            .is_known()
    );
    assert!(
        !node(&before, "input/selected")
            .facts
            .iter()
            .any(|fact| fact.path.0.contains("skip"))
    );
    fs::write(fixture.repo.join("tree/skip"), "changed excluded bytes").unwrap();
    let after = capture_evaluated(
        &ir,
        &fixture.repository('b'),
        None,
        SnapshotOrigin::Recorded,
    )
    .unwrap();
    assert_eq!(
        node(&before, "input/selected"),
        node(&after, "input/selected")
    );
    fs::write(fixture.repo.join("tree/keep"), "changed selected bytes").unwrap();
    let selected_change = capture_evaluated(
        &ir,
        &fixture.repository('c'),
        None,
        SnapshotOrigin::Recorded,
    )
    .unwrap();
    assert_ne!(
        fact(&before, "input/selected", "/captured_bytes/keep"),
        fact(&selected_change, "input/selected", "/captured_bytes/keep")
    );
}

#[test]
fn typed_captured_paths_match_saved_logical_paths_without_rewriting_literal_arguments() {
    let fixture = Fixture::new();
    let repository = fixture.repository('a');
    let Repository::Evaluated { sources, .. } = &repository else {
        unreachable!()
    };
    let captured_path = sources
        .repository()
        .join("selected")
        .to_str()
        .unwrap()
        .to_owned();
    let mut normal = serde_json::to_value(ir()).unwrap();
    normal["workspace"]["inputs"][0]["origin"]["path"] = json!(captured_path);
    normal["workspace"]["outputs"][0]["run"]["argv"][0]["value"] = json!(captured_path);
    normal["workspace"]["outputs"].as_array_mut().unwrap().push(json!({
        "kind":"package","name":"native","span":{"file":"gripsack.ts","line":12},
        "producer":{"kind":"provider","provider":{"kind":"fetch","fetch":{"kind":"file","path":captured_path},"span":{"file":"gripsack.ts","line":12}}},
        "commands":{},"target":{"os":"linux","arch":"x86_64"},"layout":{"kind":"relocatable"}
    }));
    let normal: Ir = serde_json::from_value(normal).unwrap();
    let mut saved = normal.clone();
    crate::rebind_captured_ir(&mut saved, sources).unwrap();
    let before = capture_evaluated(&normal, &repository, None, SnapshotOrigin::Recorded).unwrap();
    let restored = capture_evaluated(&saved, &repository, None, SnapshotOrigin::Recorded).unwrap();
    assert_eq!(before.document().nodes, restored.document().nodes);
    assert!(
        fact(&before, "input/selected", "/captured_bytes")
            .knowledge
            .is_known()
    );
    let declaration_path = "/declaration/producer/provider/fetch/path";
    assert_eq!(
        fact(&before, "output/native", declaration_path).knowledge,
        KnowledgeState::Known {
            digest: digest(
                serde_json::to_string(fixture.repo.join("selected").to_str().unwrap())
                    .unwrap()
                    .as_bytes()
            )
        }
    );
    let literal_path = "/declaration/run/argv/0/value";
    assert_eq!(
        fact(&before, "output/use-selected", literal_path).knowledge,
        KnowledgeState::Known {
            digest: digest(serde_json::to_string(&captured_path).unwrap().as_bytes())
        }
    );
    let second_repository = fixture.repository('b');
    let Repository::Evaluated {
        sources: second_sources,
        ..
    } = &second_repository
    else {
        unreachable!()
    };
    let mut second = serde_json::to_value(&normal).unwrap();
    let second_path = second_sources.repository().join("selected");
    second["workspace"]["inputs"][0]["origin"]["path"] = json!(second_path);
    second["workspace"]["outputs"][3]["producer"]["provider"]["fetch"]["path"] = json!(second_path);
    let after = capture_evaluated(
        &serde_json::from_value(second).unwrap(),
        &second_repository,
        None,
        SnapshotOrigin::Recorded,
    )
    .unwrap();
    assert_eq!(before.document().nodes, after.document().nodes);
}
