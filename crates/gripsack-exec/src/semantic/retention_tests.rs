use super::*;
use crate::semantic::{
    capture_generation,
    normalize::{known, provenance},
};
use gripsack_store::{Generation, GenerationId};
use std::os::unix::fs::{PermissionsExt, symlink};

fn snapshot(profile: &str, value: &[u8]) -> SemanticSnapshot {
    let mut document = SnapshotDocument::new(SnapshotOrigin::Recorded);
    document.nodes = vec![
        SnapshotNode {
            id: SubjectRef("workspace".into()),
            kind: SubjectKind::Workspace,
            facts: vec![known(
                "/evaluated",
                ChangeKind::Input,
                value,
                EvidenceOrigin::Observation,
            )],
            provenance: vec![],
        },
        SnapshotNode {
            id: subject("output", profile),
            kind: SubjectKind::Profile,
            facts: vec![known(
                "/declaration",
                ChangeKind::Deployment,
                b"profile",
                EvidenceOrigin::Declaration,
            )],
            provenance: vec![],
        },
    ];
    document.edges.push(SnapshotEdge {
        dependency: SubjectRef("workspace".into()),
        consumer: subject("output", profile),
        kind: EdgeKind::Runtime,
    });
    SemanticSnapshot::admit(document).unwrap()
}
fn deployment(home: &Path, roots: Vec<PathBuf>) -> DeploymentState {
    DeploymentState {
        store_path: home.join("store/profile-payload"),
        build_only: false,
        entries: vec![],
        intents: vec![],
        verified: None,
        env: vec![],
        tree256: None,
        build_closure: roots,
    }
}
fn generation(
    home: &Path,
    number: u64,
    deployments: BTreeMap<String, DeploymentState>,
) -> GenerationId {
    let id = GenerationId::new(number);
    gripsack_store::generations::write_manifest(
        &gripsack_fs::open(home).unwrap(),
        &Generation {
            number: id,
            deployments,
            directory_transitions: vec![],
            migration: None,
        },
    )
    .unwrap();
    id
}
fn corrupt(path: &Path, bytes: &[u8]) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o400)).unwrap();
}

#[test]
fn publication_deduplicates_exact_private_tree_and_profile_binding() {
    let home = tempfile::tempdir().unwrap();
    let session = LifecycleSession::acquire(home.path()).unwrap();
    let snapshot = snapshot("one", b"frozen");
    let root = publish_profile(&session, "one", snapshot.clone()).unwrap();
    assert_eq!(root, publish_profile(&session, "one", snapshot).unwrap());
    assert_eq!(
        root,
        root_path(
            home.path(),
            &gripsack_store::canonical_tree_hash(&root).unwrap()
        )
    );
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(root.join(RECORD))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o400
    );
    let state = deployment(home.path(), vec![root.clone()]);
    assert!(read_profile(home.path(), "one", &state).unwrap().is_some());
    assert!(read_profile(home.path(), "other", &state).is_err());
    let duplicate = deployment(home.path(), vec![root.clone(), root]);
    assert!(read_profile(home.path(), "one", &duplicate).is_err());
}

#[test]
fn absent_artifacts_require_fresh_publication_not_historical_reconstruction() {
    for remove_store in [false, true] {
        let home = tempfile::tempdir().unwrap();
        let session = LifecycleSession::acquire(home.path()).unwrap();
        let old = publish_profile(&session, "one", snapshot("one", b"old evidence")).unwrap();
        let state = deployment(home.path(), vec![old.clone()]);
        let id = generation(
            home.path(),
            1,
            BTreeMap::from([("one".into(), state.clone())]),
        );
        let before = serde_json::to_value(capture_generation(home.path(), id).unwrap()).unwrap();
        std::fs::remove_dir_all(if remove_store {
            home.path().join("store")
        } else {
            old.clone()
        })
        .unwrap();

        assert!(matches!(
            read_profile(home.path(), "one", &state),
            Err(SnapshotError::MissingArtifact)
        ));
        assert!(matches!(
            capture_generation(home.path(), id),
            Err(SnapshotError::MissingArtifact)
        ));
        assert!(audit_root(home.path(), &old).is_err());
        // A generation which never retained declarations still has unknown
        // evidence, unlike one whose bound artifact has disappeared.
        assert!(
            read_profile(home.path(), "legacy", &deployment(home.path(), vec![]))
                .unwrap()
                .is_none()
        );

        let fresh = snapshot("one", b"fresh evidence");
        let new = publish_profile(&session, "one", fresh.clone()).unwrap();
        assert_ne!(new, old);
        let captured = read_profile(home.path(), "one", &deployment(home.path(), vec![new]))
            .unwrap()
            .unwrap();
        assert!(same_desired_state(&captured, &fresh));
        // New evidence cannot silently repair the old generation's binding.
        assert!(matches!(
            capture_generation(home.path(), id),
            Err(SnapshotError::MissingArtifact)
        ));
        // Independently captured identical bytes do recover the exact old
        // content address, so the generation's frozen meaning is unchanged.
        let identical = publish_profile(&session, "one", snapshot("one", b"old evidence")).unwrap();
        assert_eq!(identical, old);
        assert_eq!(
            serde_json::to_value(capture_generation(home.path(), id).unwrap()).unwrap(),
            before
        );
    }
}

#[test]
fn incomplete_or_nonprivate_artifacts_are_not_absent() {
    let home = tempfile::tempdir().unwrap();
    let session = LifecycleSession::acquire(home.path()).unwrap();
    let evidence = snapshot("one", b"frozen");
    let root = publish_profile(&session, "one", evidence.clone()).unwrap();
    let state = deployment(home.path(), vec![root.clone()]);
    let id = generation(
        home.path(),
        1,
        BTreeMap::from([("one".into(), state.clone())]),
    );
    for path in [&root, &root.join(RECORD)] {
        let original = std::fs::metadata(path).unwrap().permissions();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            read_profile(home.path(), "one", &state),
            Err(SnapshotError::Invalid(_))
        ));
        assert!(capture_generation(home.path(), id).is_err());
        assert!(publish_profile(&session, "one", evidence.clone()).is_err());
        std::fs::set_permissions(path, original).unwrap();
    }
    std::fs::remove_file(root.join(RECORD)).unwrap();
    assert!(matches!(
        read_profile(home.path(), "one", &state),
        Err(SnapshotError::Invalid(_))
    ));
    assert!(capture_generation(home.path(), id).is_err());
    assert!(publish_profile(&session, "one", evidence).is_err());
}

#[test]
fn corruption_and_extra_members_never_fall_back_or_deduplicate() {
    let home = tempfile::tempdir().unwrap();
    let session = LifecycleSession::acquire(home.path()).unwrap();
    let evidence = snapshot("one", b"original");
    let root = publish_profile(&session, "one", evidence.clone()).unwrap();
    let state = deployment(home.path(), vec![root.clone()]);
    std::fs::write(root.join("extra"), b"not admitted").unwrap();
    assert!(read_profile(home.path(), "one", &state).is_err());
    assert!(publish_profile(&session, "one", evidence.clone()).is_err());
    std::fs::remove_file(root.join("extra")).unwrap();
    corrupt(&root.join(RECORD), b"{}");
    assert!(read_profile(home.path(), "one", &state).is_err());
    assert!(publish_profile(&session, "one", evidence).is_err());
}

#[test]
fn root_and_record_symlinks_are_not_artifact_authority() {
    let home = tempfile::tempdir().unwrap();
    let session = LifecycleSession::acquire(home.path()).unwrap();
    let root = publish_profile(&session, "one", snapshot("one", b"frozen")).unwrap();
    let state = deployment(home.path(), vec![root.clone()]);
    let moved = home.path().join("outside");
    std::fs::rename(&root, &moved).unwrap();
    symlink(home.path().join("absent"), &root).unwrap();
    assert!(matches!(
        read_profile(home.path(), "one", &state),
        Err(SnapshotError::Io(_))
    ));
    std::fs::remove_file(&root).unwrap();
    symlink(&moved, &root).unwrap();
    assert!(read_profile(home.path(), "one", &state).is_err());
    std::fs::remove_file(&root).unwrap();
    std::fs::rename(&moved, &root).unwrap();
    std::fs::rename(root.join(RECORD), home.path().join(RECORD)).unwrap();
    symlink(home.path().join(RECORD), root.join(RECORD)).unwrap();
    assert!(read_profile(home.path(), "one", &state).is_err());
}

#[test]
fn bounded_regular_record_rejects_oversize_and_fifo() {
    let home = tempfile::tempdir().unwrap();
    let session = LifecycleSession::acquire(home.path()).unwrap();
    let root = publish_profile(&session, "one", snapshot("one", b"frozen")).unwrap();
    let path = root.join(RECORD);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len((MAX_SNAPSHOT_BYTES + 1) as u64)
        .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(audit_root(home.path(), &root).is_err());
    std::fs::remove_file(&path).unwrap();
    let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(audit_root(home.path(), &root).is_err());
}

#[test]
fn legacy_and_new_profiles_keep_distinct_frozen_scopes() {
    let home = tempfile::tempdir().unwrap();
    let session = LifecycleSession::acquire(home.path()).unwrap();
    let root = publish_profile(&session, "new", snapshot("new", b"frozen")).unwrap();
    let id = generation(
        home.path(),
        1,
        BTreeMap::from([
            ("new".into(), deployment(home.path(), vec![root.clone()])),
            ("legacy".into(), deployment(home.path(), vec![])),
        ]),
    );
    let captured = capture_generation(home.path(), id).unwrap();
    let new = captured
        .document()
        .nodes
        .iter()
        .find(|node| node.id == subject("output", "new"))
        .unwrap();
    assert!(
        new.facts
            .iter()
            .any(|fact| fact.path.0 == "/declaration" && fact.knowledge.is_known())
    );
    let old = captured
        .document()
        .nodes
        .iter()
        .find(|node| node.id == subject("output", "legacy"))
        .unwrap();
    assert!(!new.facts.iter().any(|fact| fact.path.0 == "/checks"));
    assert!(old.facts.iter().any(|fact| matches!(
        fact.knowledge,
        KnowledgeState::Unknown {
            reason: UnknownReason::HistoricalDeclaration
        }
    )));
    assert!(
        !serde_json::to_string(&captured)
            .unwrap()
            .contains("build_closure")
    );
    corrupt(&root.join(RECORD), b"{}");
    assert!(capture_generation(home.path(), id).is_err());
}

#[test]
fn differing_shared_facts_remain_unknown_not_last_writer_wins() {
    let mut document = SnapshotDocument::new(SnapshotOrigin::Generation);
    merge(&mut document, snapshot("a", b"old"), "a");
    merge(&mut document, snapshot("b", b"new"), "b");
    let snapshot = SemanticSnapshot::admit(document).unwrap();
    let workspace = snapshot
        .document()
        .nodes
        .iter()
        .find(|node| node.id.0 == "workspace")
        .unwrap();
    assert!(matches!(
        workspace.facts[0].knowledge,
        KnowledgeState::Unknown {
            reason: UnknownReason::Unobserved
        }
    ));
    assert!(!snapshot.document().completeness.complete);
}

#[test]
fn changing_dependency_direction_preserves_individual_frozen_graphs() {
    fn graph(profile: &str, reverse: bool) -> SemanticSnapshot {
        let mut document = snapshot(profile, b"same").into_document();
        for name in ["x", "y"] {
            document.nodes.push(SnapshotNode {
                id: subject("output", name),
                kind: SubjectKind::Package,
                facts: vec![],
                provenance: vec![],
            });
        }
        let (dependency, consumer) = if reverse { ("y", "x") } else { ("x", "y") };
        document.edges.push(SnapshotEdge {
            dependency: subject("output", dependency),
            consumer: subject("output", consumer),
            kind: EdgeKind::Production,
        });
        document.edges.push(SnapshotEdge {
            dependency: subject("output", consumer),
            consumer: subject("output", profile),
            kind: EdgeKind::Runtime,
        });
        SemanticSnapshot::admit(document).unwrap()
    }
    let mut document = SnapshotDocument::new(SnapshotOrigin::Generation);
    merge(&mut document, graph("a", false), "a");
    merge(&mut document, graph("b", true), "b");
    let snapshot = SemanticSnapshot::admit(document).unwrap();
    assert!(
        snapshot
            .document()
            .nodes
            .iter()
            .any(|node| node.id.0 == "retained/b/output/x")
    );
    assert!(!snapshot.document().completeness.complete);
    assert_eq!(
        snapshot
            .document()
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Production)
            .count(),
        2
    );
}

#[test]
fn provenance_only_evidence_does_not_change_desired_state() {
    let before = snapshot("one", b"input");
    let mut document = before.clone().into_document();
    document.nodes[0]
        .provenance
        .push(provenance(&gripsack_ir::Span {
            file: "profile.ts".into(),
            line: 22,
            col: None,
        }));
    document.nodes[0].facts.push(known(
        "/source",
        ChangeKind::Provenance,
        b"new source bytes",
        EvidenceOrigin::Observation,
    ));
    document.nodes[0].facts.push(known(
        "/age",
        ChangeKind::Observation,
        b"new observation",
        EvidenceOrigin::Observation,
    ));
    assert!(same_desired_state(
        &before,
        &SemanticSnapshot::admit(document).unwrap()
    ));
    assert!(!same_desired_state(
        &before,
        &snapshot("one", b"changed input")
    ));
}

#[test]
fn generation_roots_retain_semantics_and_native_audit_checks_them() {
    let home = tempfile::tempdir().unwrap();
    let session = LifecycleSession::acquire(home.path()).unwrap();
    let retained = publish_profile(&session, "one", snapshot("one", b"retained")).unwrap();
    let orphan = publish_profile(&session, "orphan", snapshot("orphan", b"unreferenced")).unwrap();
    let state = deployment(home.path(), vec![retained.clone()]);
    std::fs::create_dir_all(&state.store_path).unwrap();
    generation(home.path(), 1, BTreeMap::from([("one".into(), state)]));
    let report = crate::gc(&session, None, false).unwrap();
    assert!(!report.store_removed.contains(&retained));
    assert!(retained.exists());
    assert!(report.store_removed.contains(&orphan));
    let roots = BTreeMap::from([(
        retained.clone(),
        crate::workspace::integrity::RootAuthority::Unclassified,
    )]);
    assert!(
        crate::workspace::integrity::inspect(home.path(), &roots)
            .unwrap()
            .is_empty()
    );
    corrupt(&retained.join(RECORD), b"{}");
    assert!(
        !crate::workspace::integrity::inspect(home.path(), &roots)
            .unwrap()
            .is_empty()
    );
}
