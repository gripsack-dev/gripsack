//! The public IR envelope has exactly one required current workspace.
use gripsack_ir::{check, parse, workspace::WorkspaceOperation};
use serde_json::{Value, json};

fn document() -> Value {
    json!({"ir_version":9,"host":{"os":"linux","arch":"x86_64"},"workspace":{
        "span":{"file":"gripsack.ts","line":1},"outputs":[{
            "kind":"package","name":"tool","span":{"file":"gripsack.ts","line":2},
            "producer":{"kind":"provider","provider":{"kind":"fetch",
                "fetch":{"kind":"file","path":"payload"},"span":{"file":"gripsack.ts","line":3}}},
            "commands":{"tool":"bin/tool"},"target":{"os":"linux","arch":"x86_64"},
            "layout":{"kind":"relocatable"}
        }]
    }})
}

#[test]
fn required_workspace_lists_provider_outputs_and_round_trips() {
    let ir = check(&document().to_string()).unwrap();
    assert_eq!(ir.workspace.outputs.len(), 1);
    assert_eq!(ir.workspace.outputs[0].name(), "tool");
    assert_eq!(ir.workspace.outputs[0].kind(), "package");
    let saved = serde_json::to_value(&ir).unwrap();
    assert!(saved.get("modules").is_none());
    assert!(saved.get("workspace").is_some());
    check(&saved.to_string()).unwrap();
    for operation in [
        WorkspaceOperation::Update,
        WorkspaceOperation::Build,
        WorkspaceOperation::Run,
    ] {
        assert!(ir.workspace_execution_error(operation).is_none());
    }
}

#[test]
fn noncurrent_versions_never_gain_admission_or_execution_authority() {
    let schema = jsonschema::validator_for(
        &serde_json::from_str::<Value>(include_str!("../schema-v9.json")).unwrap(),
    )
    .unwrap();
    let ir = check(&document().to_string()).unwrap();
    for version in [0, 1, 2, 3, 4, 5, 6, 7, 8, 10, u32::MAX] {
        let mut doc = document();
        doc["ir_version"] = json!(version);
        assert!(!schema.is_valid(&doc));
        assert!(parse(&doc.to_string()).is_err());
        let mut forged = ir.clone();
        forged.ir_version = version;
        assert!(!gripsack_ir::sema::run(&forged).is_empty());
        for operation in [
            WorkspaceOperation::Update,
            WorkspaceOperation::Build,
            WorkspaceOperation::Run,
        ] {
            assert!(forged.workspace_execution_error(operation).is_some());
        }
        // Serialization is not admission, even for a manually forged typed value.
        let saved = serde_json::to_value(&forged).unwrap();
        assert!(!schema.is_valid(&saved));
        assert!(parse(&saved.to_string()).is_err());
    }
}
