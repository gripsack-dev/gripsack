use super::*;
use crate::semantic::{
    EvidenceOrigin, KnowledgeState, SemanticSnapshot, SnapshotOrigin, capture_ir,
};
use crate::workspace::artifact::Artifact;
use gripsack_ir::workspace_model::{
    CatalogPackageLayout,
    identity::{DefinitionDigest, ExecutableDigest, PackageDigest},
};
use serde_json::json;

fn ir() -> Ir {
    serde_json::from_value(json!({"ir_version":gripsack_ir::IR_VERSION,"host":{"os":"linux","arch":"x86_64"},
        "workspace":{"span":{"file":"gripsack.ts","line":1},"outputs":[{
            "kind":"package","name":"native","span":{"file":"gripsack.ts","line":2},
            "producer":{"kind":"provider","provider":{"kind":"fetch","fetch":{"kind":"file","path":"tool"},"span":{"file":"gripsack.ts","line":3}}},
            "commands":{"tool":"bin/tool"},"target":{"os":"linux","arch":"x86_64"},"layout":{"kind":"relocatable"}
        }]}})).unwrap()
}

#[test]
fn actual_native_receipt_projection_replaces_unknown_without_exposing_storage_paths() {
    let ir = ir();
    let mut document = capture_ir(&ir, None, SnapshotOrigin::Recorded)
        .unwrap()
        .into_document();
    let identity = PackageDigest::from_bytes([0x42; 32]);
    let package = Arc::new(Package {
        identity,
        root: PathBuf::from("/private/home/store/package"),
        producer: Arc::new(Artifact {
            root: PathBuf::from("/private/home/store/source"),
            payload: PathBuf::from("/private/home/store/source/payload"),
            tree: serde_json::from_value(json!("c".repeat(64))).unwrap(),
            retention: BTreeSet::new(),
        }),
        archive_root: None,
        selected_root: None,
        selected_payload: None,
        commands: BTreeMap::from([(
            "tool".into(),
            super::super::artifact::ProvidedCommand {
                selector: "bin/tool".into(),
                executable: ExecutableDigest::from_bytes([0x53; 32]),
            },
        )]),
        runtime: Vec::new(),
        target: serde_json::from_value(json!({"os":"linux","arch":"x86_64"})).unwrap(),
        layout: CatalogPackageLayout::Relocatable,
        host_runtime: None,
        conda: None,
    });
    let realized = Realization {
        packages: BTreeMap::from([("native", Arc::clone(&package)), ("not-selected", package)]),
        inputs: BTreeMap::new(),
    };
    assert!(
        document
            .nodes
            .iter()
            .find(|node| node.id.0 == "output/native")
            .unwrap()
            .facts
            .iter()
            .any(|fact| fact.path.0 == "/realized_artifact" && !fact.knowledge.is_known())
    );
    realized.capture_semantic_evidence(&mut document).unwrap();
    let snapshot = SemanticSnapshot::admit(document).unwrap();
    let native = snapshot
        .document()
        .nodes
        .iter()
        .find(|node| node.id.0 == "output/native")
        .unwrap();
    let identity_fact = native
        .facts
        .iter()
        .find(|fact| fact.path.0 == "/package_identity")
        .unwrap();
    assert_eq!(
        identity_fact.knowledge,
        KnowledgeState::Known {
            digest: DefinitionDigest::from_bytes(*identity.bytes())
        }
    );
    assert_eq!(identity_fact.origin, EvidenceOrigin::Receipt);
    for path in [
        "/realized_artifact/tree",
        "/realized_artifact/commands/tool/executable",
        "/realized_artifact/layout/kind",
    ] {
        assert!(
            native
                .facts
                .iter()
                .find(|fact| fact.path.0 == path)
                .unwrap()
                .knowledge
                .is_known()
        );
    }
    let encoded = serde_json::to_string(&snapshot).unwrap();
    assert!(!encoded.contains("/private/home"));
    assert!(!encoded.contains("not-selected"));
}
