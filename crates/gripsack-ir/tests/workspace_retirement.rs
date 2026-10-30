//! Retained historical data never grants fresh recipe or platform authority.
use gripsack_ir::{
    check, parse,
    workspace::WorkspaceOperation,
    workspace_model::{WorkspaceOutput, lock::WorkspaceLock},
};
use serde_json::{Value, json};

fn historical() -> Value {
    let span = json!({"file":"historical.ts","line":2});
    json!({"ir_version":9,"host":{"os":"macos","arch":"aarch64"},"workspace":{
    "span":span,"outputs":[
        {"kind":"recipe","name":"build","span":span,"source":{"kind":"fetch","span":span,"fetch":{"kind":"file","path":"source"}},
         "execution":{"kind":"isolated_linux","worker":"buildkit","platform":{"os":"linux","arch":"x86_64"},"toolchain":{"reference":format!("toolchain@sha256:{}","a".repeat(64))}},
         "output_kind":"tree","target":{"os":"macos","arch":"aarch64","abi":"darwin"}},
        {"kind":"package","name":"tool","span":span,"producer":{"kind":"recipe","recipe":"build"},"commands":{"tool":"bin/tool"},"target":{"os":"macos","arch":"aarch64","abi":"darwin"},"layout":{"kind":"relocatable"}},
        {"kind":"image","name":"image","span":span,"packages":["tool"],"target":{"os":"linux","arch":"x86_64"}}
    ]}})
}

#[test]
fn historical_recipe_image_and_mac_models_remain_data_only() {
    let original = historical().to_string();
    let retained: gripsack_ir::Ir = serde_json::from_value(historical()).unwrap();
    assert!(matches!(
        &retained.workspace.outputs[0],
        WorkspaceOutput::Recipe(_)
    ));
    assert!(matches!(
        &retained.workspace.outputs[2],
        WorkspaceOutput::Image(_)
    ));
    assert_eq!(retained.ir_version, 9);
    assert!(parse(&original).is_err());
    assert!(!gripsack_ir::run(&retained).is_empty());
    for operation in [
        WorkspaceOperation::Apply,
        WorkspaceOperation::Plan,
        WorkspaceOperation::Update,
        WorkspaceOperation::Build,
        WorkspaceOperation::Run,
        WorkspaceOperation::Shell,
        WorkspaceOperation::Task,
    ] {
        assert!(retained.workspace_execution_error(operation).is_some());
        let mut forged = retained.clone();
        forged.ir_version = 10;
        assert!(forged.workspace_execution_error(operation).is_some());
        assert!(!gripsack_ir::run(&forged).is_empty());
    }
    let mut current = historical();
    current["ir_version"] = json!(10);
    assert!(parse(&current.to_string()).is_err());
}

fn imported() -> Value {
    let span = json!({"file":"packages.ts","line":1});
    json!({"ir_version":10,"host":{"os":"linux","arch":"x86_64"},"workspace":{
        "span":span,"outputs":[{"kind":"package","name":"tool","span":span,
        "producer":{"kind":"provider","provider":{"kind":"package","manifest":"a".repeat(64),"span":span}},
        "commands":{"tool":"bin/tool"},"target":{"os":"linux","arch":"x86_64"},"layout":{"kind":"relocatable"}}]}})
}

#[test]
fn imported_manifest_sources_have_closed_digest_only_authority() {
    let value = imported();
    let schema = jsonschema::validator_for(
        &serde_json::from_str::<Value>(include_str!("../schema-v10.json")).unwrap(),
    )
    .unwrap();
    assert!(schema.is_valid(&value));
    let ir = check(&value.to_string()).unwrap();
    check(&serde_json::to_string(&ir).unwrap()).unwrap();
    for digest in [
        "../manifest.json".to_owned(),
        "A".repeat(64),
        "a".repeat(63),
        "a".repeat(65),
    ] {
        let mut invalid = value.clone();
        invalid["workspace"]["outputs"][0]["producer"]["provider"]["manifest"] = json!(digest);
        assert!(!schema.is_valid(&invalid));
        assert!(check(&invalid.to_string()).is_err());
    }
    let mut invalid = value.clone();
    invalid["workspace"]["outputs"][0]["producer"]["provider"]["url"] =
        json!("https://unreviewed.test");
    assert!(parse(&invalid.to_string()).is_err());
    let mut old = value;
    old["ir_version"] = json!(9);
    assert!(parse(&old.to_string()).is_err());
}

fn lock(source: Value) -> Value {
    json!({"lock_version":3,"definitions":{"frontend":"f".repeat(64)},"resolutions":{
        "linux-x86_64":{"pins":[{"output":"db","source":source,"resolved":{"sha256":"a".repeat(64)}}]}
    }})
}

#[test]
fn lock_v3_binds_new_sources_without_reinterpreting_v2() {
    for source in [
        json!({"kind":"package","manifest":"a".repeat(64)}),
        json!({"kind":"oci_image","reference":format!("postgres@sha256:{}", "a".repeat(64))}),
    ] {
        let value = lock(source);
        let decoded = WorkspaceLock::from_json(&value.to_string()).unwrap();
        WorkspaceLock::from_json(&serde_json::to_string(&decoded).unwrap()).unwrap();
        let mut old = value.clone();
        old["lock_version"] = json!(2);
        assert!(WorkspaceLock::from_json(&old.to_string()).is_err());
        let mut unknown = value;
        unknown["resolutions"]["linux-x86_64"]["pins"][0]["source"]["unreviewed"] = json!(true);
        assert!(WorkspaceLock::from_json(&unknown.to_string()).is_err());
    }
    let mut image =
        lock(json!({"kind":"oci_image","reference":format!("postgres@sha256:{}", "a".repeat(64))}));
    image["resolutions"]["linux-x86_64"]["pins"][0]["resolved"]["sha256"] = json!("b".repeat(64));
    assert!(WorkspaceLock::from_json(&image.to_string()).is_err());
    let mut old = lock(json!({"kind":"fetch","fetch":{"kind":"file","path":"payload"}}));
    old["lock_version"] = json!(2);
    let decoded = WorkspaceLock::from_json(&old.to_string()).unwrap();
    assert_eq!(decoded.lock_version, 2);
    assert_eq!(serde_json::to_value(decoded).unwrap(), old);
}
