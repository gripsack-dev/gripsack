use gripsack_ir::{
    check,
    workspace_model::{
        WorkspaceOutput, WorkspaceProducer,
        identity::{self, RecipeDigest},
    },
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn document(version: u32) -> Value {
    json!({"ir_version":version,"host":{"os":"linux","arch":"x86_64"},"workspace":{
    "span":{"file":"gripsack.ts","line":1},"outputs":[{
        "kind":"package","name":"tool","span":{"file":"gripsack.ts","line":2},
        "producer":{"kind":"provider","provider":{"kind":"fetch","fetch":{"kind":"file","path":"payload"},"span":{"file":"gripsack.ts","line":2}}},
        "commands":{"tool":"bin/tool"},"target":{"os":"linux","arch":"x86_64","abi":"gnu"},"layout":{"kind":"relocatable"}
    }]}})
}
fn policy() -> Value {
    json!({"library_directories":["/opt/bb/lib64","/opt/vendor/lib"]})
}
fn schema() -> jsonschema::Validator {
    jsonschema::validator_for(
        &serde_json::from_str::<Value>(include_str!("../schema-v10.json")).unwrap(),
    )
    .unwrap()
}

#[test]
fn host_runtime_schema_and_core_reject_unsafe_or_unknown_policy() {
    let schema = schema();
    let mut cases = vec![
        json!(null),
        json!({}),
        json!({"library_directories":[]}),
        json!({"library_directories":["/opt/lib","/opt/lib"]}),
        json!({"library_directories":["/opt/lib"],"ambient":true}),
    ];
    for path in [
        "/",
        "relative",
        "/opt/../lib",
        "/opt/./lib",
        "/opt//lib",
        "/opt/lib/",
        "/opt/$LIB",
        "/opt/a:b",
        "/opt/a;b",
        "/opt/a\\b",
        "/opt/a\nb",
    ] {
        cases.push(json!({"library_directories":[path]}));
    }
    for invalid in cases {
        let mut doc = document(gripsack_ir::IR_VERSION);
        doc["workspace"]["outputs"][0]["host_runtime"] = invalid.clone();
        assert!(!schema.is_valid(&doc), "schema admitted {invalid}");
        assert!(check(&doc.to_string()).is_err(), "core admitted {invalid}");
    }
    let mut doc = document(gripsack_ir::IR_VERSION);
    doc["workspace"]["outputs"][0]["host_runtime"] = policy();
    doc["workspace"]["outputs"][0]["target"]["abi"] = json!("musl");
    assert!(!schema.is_valid(&doc));
    assert!(check(&doc.to_string()).is_err());
}

#[test]
fn conda_baselines_are_strict_declarations() {
    let current = schema();
    let mut doc = document(gripsack_ir::IR_VERSION);
    doc["workspace"]["outputs"][0]["layout"] = json!({"kind":"prefix_materialized"});
    doc["workspace"]["outputs"][0]["producer"]["provider"] = json!({
        "kind":"conda_environment","channels":["conda-forge"],"packages":{"python":"==3.12.1"},
        "system_requirements":{"libc":{"family":"glibc","version":"2.28"},"linux":"4.18"},
        "span":{"file":"gripsack.ts","line":3}
    });
    assert!(current.is_valid(&doc));
    assert!(check(&doc.to_string()).is_ok());
    for invalid in [
        json!(null),
        json!({"cuda":"12"}),
        json!({"libc":null}),
        json!({"linux":null}),
        json!({"linux":">=4.18"}),
        json!({"libc":{"family":"musl","version":"1.2"}}),
        json!({"libc":{"family":"glibc","version":"2.28","extra":true}}),
    ] {
        doc["workspace"]["outputs"][0]["producer"]["provider"]["system_requirements"] =
            invalid.clone();
        assert!(!current.is_valid(&doc), "schema admitted {invalid}");
        assert!(check(&doc.to_string()).is_err(), "core admitted {invalid}");
    }
    // Unsupported floors must also fail when the source did not list platforms.
    doc["workspace"]["outputs"][0]["producer"]["provider"]["system_requirements"] =
        json!({"linux":"4.18"});
    doc["workspace"]["outputs"][0]["target"] = json!({"os":"macos","arch":"aarch64"});
    assert!(!current.is_valid(&doc));
    assert!(check(&doc.to_string()).is_err());
}

#[test]
fn ordered_host_policy_changes_consumer_identity_not_producer() {
    let mut ir = check(&document(gripsack_ir::IR_VERSION).to_string()).unwrap();
    let WorkspaceOutput::Package(mut package) = ir.workspace.outputs.remove(0) else {
        panic!("package fixture")
    };
    package.producer = WorkspaceProducer::Recipe {
        recipe: "build".into(),
    };
    let recipe = RecipeDigest::parse(&"a".repeat(64)).unwrap();
    let original = identity::recipe_package_digest(&package, recipe, &BTreeMap::new()).unwrap();
    let producer = package.producer.clone();
    package.host_runtime = Some(serde_json::from_value(policy()).unwrap());
    let reviewed = identity::recipe_package_digest(&package, recipe, &BTreeMap::new()).unwrap();
    assert_ne!(reviewed, original);
    package
        .host_runtime
        .as_mut()
        .unwrap()
        .library_directories
        .reverse();
    assert_ne!(
        identity::recipe_package_digest(&package, recipe, &BTreeMap::new()).unwrap(),
        reviewed
    );
    assert_eq!(package.producer, producer);
    package.host_runtime = None;
    assert_eq!(
        identity::recipe_package_digest(&package, recipe, &BTreeMap::new()).unwrap(),
        original
    );
}
