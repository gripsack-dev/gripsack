use super::normalize::{facts, known, unknown};
use super::*;
use gripsack_ir::{Ir, Span};
use serde_json::json;

fn node(name: &str, value: &[u8]) -> SnapshotNode {
    SnapshotNode {
        id: SubjectRef(name.into()),
        kind: SubjectKind::Package,
        facts: vec![known(
            "/input",
            ChangeKind::Input,
            value,
            EvidenceOrigin::Lock,
        )],
        provenance: Vec::new(),
    }
}
fn edge(dependency: &str, consumer: &str, kind: EdgeKind) -> SnapshotEdge {
    SnapshotEdge {
        dependency: SubjectRef(dependency.into()),
        consumer: SubjectRef(consumer.into()),
        kind,
    }
}
fn snapshot(nodes: Vec<SnapshotNode>, edges: Vec<SnapshotEdge>) -> SemanticSnapshot {
    let mut document = SnapshotDocument::new(SnapshotOrigin::Recorded);
    document.nodes = nodes;
    document.edges = edges;
    SemanticSnapshot::admit(document).unwrap()
}
fn task_ir() -> Ir {
    serde_json::from_value(json!({
        "ir_version":gripsack_ir::IR_VERSION,
        "host":{"os":"linux","arch":"x86_64"},
        "workspace":{"span":{"file":"gripsack.ts","line":1},"outputs":[
            {"kind":"task","name":"a","span":{"file":"tasks.ts","line":2},
             "context":{"kind":"host","mutable_paths":[]},
             "steps":[{"command":{"kind":"exec","span":{"file":"tasks.ts","line":3},
                        "argv":[{"kind":"literal","value":"/bin/echo"},{"kind":"literal","value":"first"},{"kind":"literal","value":"second"}]}}]},
            {"kind":"task","name":"b","span":{"file":"tasks.ts","line":5},
             "context":{"kind":"host","mutable_paths":[]},"deps":["a"],
             "steps":[{"command":{"kind":"exec","span":{"file":"tasks.ts","line":6},"argv":[{"kind":"literal","value":"/bin/true"}]}}]}
        ]}
    })).unwrap()
}

#[test]
fn strict_snapshot_rejects_duplicates_unknown_versions_and_fields() {
    let valid = serde_json::to_value(snapshot(vec![node("a", b"v")], vec![])).unwrap();
    let text = serde_json::to_string(&valid).unwrap();
    let duplicate = text.replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
    assert!(parse_snapshot(duplicate.as_bytes()).is_err());
    let duplicate_nested = text.replacen(
        "\"kind\":\"package\"",
        "\"kind\":\"package\",\"kind\":\"package\"",
        1,
    );
    assert!(parse_snapshot(duplicate_nested.as_bytes()).is_err());
    let mut extra = valid.clone();
    extra["unexpected"] = json!(true);
    assert!(parse_snapshot(&serde_json::to_vec(&extra).unwrap()).is_err());
    let mut version = valid;
    version["version"] = json!(999);
    assert!(matches!(
        parse_snapshot(&serde_json::to_vec(&version).unwrap()),
        Err(SnapshotError::Version(999))
    ));
}
#[test]
fn hostile_size_depth_ids_and_graphs_fail_closed() {
    assert!(matches!(
        parse_snapshot(&vec![b' '; MAX_SNAPSHOT_BYTES + 1]),
        Err(SnapshotError::Bound("bytes"))
    ));
    assert!(matches!(
        admission::parse_json(
            format!(
                "{}0{}",
                "[".repeat(MAX_DEPTH + 1),
                "]".repeat(MAX_DEPTH + 1)
            )
            .as_bytes()
        ),
        Err(SnapshotError::Bound("depth"))
    ));
    for name in [
        "escape\u{1b}[31m",
        "https://user:password@example.org",
        "bidi\u{202e}name",
    ] {
        let mut document = SnapshotDocument::new(SnapshotOrigin::Recorded);
        document.nodes.push(node(name, b"v"));
        assert!(SemanticSnapshot::admit(document).is_err());
    }
    let mut document = snapshot(vec![node("a", b"v")], vec![]).into_document();
    document.edges.push(edge("missing", "a", EdgeKind::Runtime));
    assert!(SemanticSnapshot::admit(document).is_err());
    let mut document = SnapshotDocument::new(SnapshotOrigin::Recorded);
    document.nodes = vec![node("a", b"v"), node("a", b"v")];
    assert!(SemanticSnapshot::admit(document).is_err());
}
#[test]
fn unknown_to_unknown_is_explicit_and_not_complete_no_change() {
    let mut unknown_node = node("a", b"v");
    unknown_node.facts = vec![unknown(
        "/input",
        ChangeKind::Input,
        UnknownReason::MissingInput,
    )];
    let snapshot = snapshot(vec![unknown_node], vec![]);
    let report = compare(&snapshot, &snapshot);
    assert!(!report.is_complete());
    assert!(!report.has_changes());
    assert_eq!(report.changes[0].difference, Difference::Unknown);
    assert_eq!(report.impacts[0].kind, ImpactKind::Uncertain);
}
#[test]
fn old_unknown_subtree_does_not_turn_new_evidence_into_added_fact() {
    let mut before = node("a", b"v");
    before.facts = vec![unknown(
        "/input",
        ChangeKind::Input,
        UnknownReason::MissingInput,
    )];
    let mut after = node("a", b"v");
    after.facts = vec![known(
        "/input/digest",
        ChangeKind::Input,
        b"new",
        EvidenceOrigin::Lock,
    )];
    let report = compare(
        &snapshot(vec![before], vec![]),
        &snapshot(vec![after], vec![]),
    );
    assert!(
        report
            .changes
            .iter()
            .all(|change| change.difference == Difference::Unknown)
    );
}
#[test]
fn field_normalization_preserves_argv_path_and_script_order_but_sorts_sets() {
    let left = facts(
        json!({"deps":["z","a"],"runtime":["b","a"],"argv":["one","two"],"packages":["a","b"],"body":"echo hi\n"}),
        "/declaration",
        "task",
        ChangeKind::Invocation,
        EvidenceOrigin::Declaration,
    );
    let right = facts(
        json!({"deps":["a","z"],"runtime":["a","b"],"argv":["one","two"],"packages":["a","b"],"body":"echo hi\n"}),
        "/declaration",
        "task",
        ChangeKind::Invocation,
        EvidenceOrigin::Declaration,
    );
    assert_eq!(left, right);
    for changed in [
        json!({"argv":["two","one"]}),
        json!({"packages":["b","a"]}),
        json!({"body":"echo hi # comment\n"}),
    ] {
        let candidate = facts(
            changed,
            "/declaration",
            "task",
            ChangeKind::Invocation,
            EvidenceOrigin::Declaration,
        );
        assert_ne!(left, candidate);
    }
}
#[test]
fn actual_ir_provenance_move_is_not_a_semantic_change() {
    let before = task_ir();
    let mut value = serde_json::to_value(&before).unwrap();
    value["workspace"]["outputs"][0]["span"]["file"] = json!("moved.ts");
    value["workspace"]["outputs"][0]["steps"][0]["command"]["span"]["line"] = json!(99);
    let after: Ir = serde_json::from_value(value).unwrap();
    let report = compare(
        &capture_ir(&before, None, SnapshotOrigin::Proposed).unwrap(),
        &capture_ir(&after, None, SnapshotOrigin::Proposed).unwrap(),
    );
    assert!(
        report
            .changes
            .iter()
            .filter(|change| change.difference != Difference::Unknown)
            .all(|change| change.kind == ChangeKind::Provenance)
    );
    assert!(
        !report
            .impacts
            .iter()
            .any(|impact| impact.kind == ImpactKind::NeedsRealization)
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.before_provenance != change.after_provenance)
    );
}
#[test]
fn actual_ir_argument_reordering_changes_invocation_without_package_realization() {
    let before = task_ir();
    let mut value = serde_json::to_value(&before).unwrap();
    value["workspace"]["outputs"][0]["steps"][0]["command"]["argv"]
        .as_array_mut()
        .unwrap()
        .swap(1, 2);
    let after: Ir = serde_json::from_value(value).unwrap();
    let report = compare(
        &capture_ir(&before, None, SnapshotOrigin::Proposed).unwrap(),
        &capture_ir(&after, None, SnapshotOrigin::Proposed).unwrap(),
    );
    assert!(
        report
            .impacts
            .iter()
            .any(|impact| impact.subject.0 == "output/b" && impact.kind == ImpactKind::Invocation)
    );
    assert!(
        !report
            .impacts
            .iter()
            .any(|impact| impact.kind == ImpactKind::NeedsRealization)
    );
}
#[test]
fn removed_and_retargeted_edges_keep_both_graph_witnesses() {
    let before = snapshot(
        vec![
            node("source", b"old"),
            node("other", b"v"),
            node("consumer", b"v"),
            node("leaf", b"v"),
        ],
        vec![
            edge("source", "consumer", EdgeKind::Production),
            edge("consumer", "leaf", EdgeKind::Runtime),
        ],
    );
    let after = snapshot(
        vec![
            node("other", b"v"),
            node("consumer", b"v"),
            node("leaf", b"v"),
        ],
        vec![
            edge("other", "consumer", EdgeKind::Production),
            edge("consumer", "leaf", EdgeKind::Runtime),
        ],
    );
    let report = compare(&before, &after);
    assert!(
        report
            .impacts
            .iter()
            .any(|impact| impact.subject.0 == "leaf")
    );
    assert!(
        report
            .reasons
            .iter()
            .any(|reason| reason.dependency.0 == "source" && reason.graph == GraphSide::Before)
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.subject.0 == "source" && change.difference == Difference::Removed)
    );
}
#[test]
fn runtime_cycle_terminates_but_production_cycle_is_rejected() {
    let nodes = vec![node("a", b"old"), node("b", b"v")];
    let before = snapshot(
        nodes.clone(),
        vec![
            edge("a", "b", EdgeKind::Runtime),
            edge("b", "a", EdgeKind::Runtime),
        ],
    );
    let mut after = before.clone().into_document();
    after.nodes[0].facts[0] = known("/input", ChangeKind::Input, b"new", EvidenceOrigin::Lock);
    let report = compare(&before, &SemanticSnapshot::admit(after).unwrap());
    assert!(report.impacts.iter().any(|impact| impact.subject.0 == "b"));
    let mut invalid = SnapshotDocument::new(SnapshotOrigin::Recorded);
    invalid.nodes = nodes;
    invalid.edges = vec![
        edge("a", "b", EdgeKind::Production),
        edge("b", "a", EdgeKind::Production),
    ];
    assert!(SemanticSnapshot::admit(invalid).is_err());
}
#[test]
fn validation_and_order_edges_do_not_invent_content_dependencies() {
    let before = snapshot(
        vec![node("a", b"old"), node("b", b"v"), node("c", b"v")],
        vec![
            edge("a", "b", EdgeKind::Validation),
            edge("a", "c", EdgeKind::Ordering),
        ],
    );
    let mut after = before.clone().into_document();
    after.nodes[0].facts[0] = known("/input", ChangeKind::Input, b"new", EvidenceOrigin::Lock);
    let report = compare(&before, &SemanticSnapshot::admit(after).unwrap());
    assert!(report.impacts.contains(&Impact {
        subject: SubjectRef("b".into()),
        kind: ImpactKind::RequiredValidation
    }));
    assert!(report.impacts.contains(&Impact {
        subject: SubjectRef("c".into()),
        kind: ImpactKind::ExecutionOrder
    }));
    assert!(!report.impacts.contains(&Impact {
        subject: SubjectRef("c".into()),
        kind: ImpactKind::NeedsRealization
    }));
}
#[test]
fn redaction_never_persists_raw_configuration_or_low_entropy_secret_hashes() {
    let secret = "1234";
    let node = SnapshotNode {
        id: SubjectRef("config".into()),
        kind: SubjectKind::Profile,
        facts: facts(
            json!({"vars":{"password":secret},"content":"private configuration", "url":"https://u:password@example.org", "env":{"TOKEN":{"kind":"literal","value":secret}}}),
            "/declaration",
            "profile",
            ChangeKind::Deployment,
            EvidenceOrigin::Declaration,
        ),
        provenance: vec![],
    };
    let snapshot = snapshot(vec![node], vec![]);
    let encoded = serde_json::to_string(&snapshot).unwrap();
    for forbidden in [
        secret,
        "private configuration",
        "https://u:password@example.org",
        &super::normalize::digest(json!(secret).to_string().as_bytes()).to_string(),
    ] {
        assert!(!encoded.contains(forbidden));
    }
    assert!(snapshot.document().completeness.redacted);
    assert!(!snapshot.document().completeness.complete);
}
#[test]
fn private_write_refuses_replacement_and_read_refuses_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot.json");
    let snapshot = snapshot(vec![node("a", b"v")], vec![]);
    write_snapshot_private(&path, &snapshot).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(write_snapshot_private(&path, &snapshot).is_err());
    let link = directory.path().join("link");
    symlink(&path, &link).unwrap();
    assert!(read_snapshot(&link).is_err());
    assert!(read_snapshot(directory.path()).is_err());
    assert!(!compare(&snapshot, &read_snapshot(&path).unwrap()).has_changes());
}
#[test]
fn safe_relative_source_locations_survive_without_terminal_controls() {
    let span = Span {
        file: "tasks/job.ts".into(),
        line: 4,
        col: Some(2),
    };
    assert_eq!(
        super::normalize::provenance(&span).source_label.as_deref(),
        Some("tasks/job.ts")
    );
    for file in [
        "/secret/checkout.ts",
        "https://user:secret@example.org/a.ts",
        "escape\u{1b}.ts",
    ] {
        assert!(
            super::normalize::provenance(&Span {
                file: file.into(),
                ..span.clone()
            })
            .source_label
            .is_none()
        );
    }
}
#[test]
fn diamond_closure_matches_independent_reachability_oracle() {
    // Independent set fixed-point oracle over all forward graphs on four nodes.
    for mask in 0u8..64 {
        let mut edges = Vec::new();
        let mut bit = 0;
        for from in 0..4 {
            for to in from + 1..4 {
                if mask & (1 << bit) != 0 {
                    edges.push(edge(
                        &from.to_string(),
                        &to.to_string(),
                        EdgeKind::Production,
                    ));
                }
                bit += 1;
            }
        }
        let before = snapshot(
            (0..4).map(|id| node(&id.to_string(), b"v")).collect(),
            edges.clone(),
        );
        let mut after = before.clone().into_document();
        after.nodes[0].facts[0] = known(
            "/input",
            ChangeKind::Input,
            b"changed",
            EvidenceOrigin::Lock,
        );
        let report = compare(&before, &SemanticSnapshot::admit(after).unwrap());
        let mut expected = std::collections::BTreeSet::from([SubjectRef("0".into())]);
        loop {
            let previous = expected.len();
            for edge in &edges {
                if expected.contains(&edge.dependency) {
                    expected.insert(edge.consumer.clone());
                }
            }
            if previous == expected.len() {
                break;
            }
        }
        let actual = report
            .impacts
            .iter()
            .map(|impact| impact.subject.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected, "graph mask {mask}");
    }
}
#[test]
fn explanation_truncation_keeps_complete_affected_set() {
    let count = MAX_REASONS / 2 + 2;
    let mut nodes = vec![node("root", b"old")];
    let mut edges = Vec::new();
    for index in 0..count {
        let name = format!("child{index}");
        nodes.push(node(&name, b"v"));
        edges.push(edge("root", &name, EdgeKind::Runtime));
    }
    let before = snapshot(nodes, edges);
    let mut after = before.clone().into_document();
    let root = after
        .nodes
        .iter_mut()
        .find(|node| node.id.0 == "root")
        .unwrap();
    root.facts[0] = known("/input", ChangeKind::Input, b"new", EvidenceOrigin::Lock);
    let report = compare(&before, &SemanticSnapshot::admit(after).unwrap());
    assert!(report.explanations_truncated);
    assert!(!report.is_complete());
    assert_eq!(
        report
            .impacts
            .iter()
            .map(|impact| &impact.subject)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        count + 1
    );
}
#[test]
fn empty_current_preserves_missing_historical_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let current = capture_current(directory.path()).unwrap();
    assert!(
        current
            .document()
            .completeness
            .unknown_reasons
            .contains(&UnknownReason::HistoricalDeclaration)
    );
    assert!(!compare(&current, &current).is_complete());
}
