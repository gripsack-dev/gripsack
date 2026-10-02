//! Historical v4 workspace wire stays readable, strict and read-only
//! after the v5 writer cutover. The v4 schema is never rewritten to
//! interpret newer target, execution or prefix fields.

use gripsack_ir::{WORKSPACE_V4_VERSION, check, codes, parse};
use jsonschema::validator_for;
use serde_json::{Value, json};

const SCHEMA_V4: &str = include_str!("../schema-v4.json");

fn old_package() -> Value {
    json!({
        "ir_version": 4, "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {"span": {"file": "old.ts", "line": 1}, "outputs": [
            {"kind": "package", "name": "tool", "span": {"file": "old.ts", "line": 3},
             "producer": {"kind": "provider", "provider": {
                 "fetch": {"kind": "file", "path": "tool.bin"},
                 "span": {"file": "old.ts", "line": 4}}},
             "commands": {"tool": "bin/tool"},
             "target": {"os": "linux", "arch": "x86_64",
                        "abi": "legacy-special", "minimum_os": "legacy-floor"},
             "layout": "relocatable"}
        ]}
    })
}

#[test]
fn historical_schema_and_reader_preserve_opaque_platform_strings() {
    let validator = validator_for(&serde_json::from_str(SCHEMA_V4).unwrap()).unwrap();
    let old = old_package();
    assert!(validator.is_valid(&old));
    let ir = check(&old.to_string()).expect("historical v4 package remains admitted");
    assert_eq!(ir.ir_version, WORKSPACE_V4_VERSION);
    assert!(ir.workspace.is_none(), "never reclassify as executable v5");
    assert_eq!(ir.workspace_v4.as_ref().unwrap().outputs[0].name(), "tool");
    let preserved = serde_json::to_value(&ir).unwrap();
    assert_eq!(
        preserved["workspace"]["outputs"][0]["layout"],
        "relocatable"
    );
    assert_eq!(
        preserved["workspace"]["outputs"][0]["target"]["abi"],
        "legacy-special"
    );
    assert_eq!(
        preserved["workspace"]["outputs"][0]["target"]["minimum_os"],
        "legacy-floor"
    );
    assert!(preserved.get("modules").is_none());
    check(&preserved.to_string()).expect("v4 round trip retains its own reader");
    let mut mislabeled = ir;
    mislabeled.ir_version = 5;
    assert!(
        serde_json::to_value(&mislabeled).is_err(),
        "a v4 workspace must never be emitted under the v5 tag"
    );
}

#[test]
fn historical_recipe_execution_and_fixed_layout_never_gain_new_policy() {
    let mut old = old_package();
    let outputs = old["workspace"]["outputs"].as_array_mut().unwrap();
    let mut package = outputs.remove(0);
    package["producer"] = json!({"kind": "recipe", "recipe": "build"});
    package["layout"] = json!("fixed_prefix");
    outputs.push(json!({
        "kind": "recipe", "name": "build", "span": {"file": "old.ts", "line": 2},
        "source": {"fetch": {"kind": "file", "path": "src.tar.gz"},
                   "span": {"file": "old.ts", "line": 2}},
        "execution": "native", "output_kind": "tree",
        "target": {"os": "linux", "arch": "x86_64",
                   "abi": "legacy-special", "minimum_os": "legacy-floor"}
    }));
    outputs.push(package);
    let validator = validator_for(&serde_json::from_str(SCHEMA_V4).unwrap()).unwrap();
    assert!(validator.is_valid(&old));
    let ir = check(&old.to_string()).expect("v4 native is historical read-only grammar");
    let roundtrip = serde_json::to_value(&ir).unwrap();
    assert_eq!(roundtrip["workspace"]["outputs"][0]["execution"], "native");
    assert_eq!(
        roundtrip["workspace"]["outputs"][1]["layout"],
        "fixed_prefix"
    );
    assert!(ir.workspace.is_none());

    // v4 never had an install-prefix field: selection is rejected, not
    // silently interpreted using v5's new destination policy.
    old["workspace"]["outputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "kind": "environment", "name": "dev", "span": {"file": "old.ts", "line": 7},
            "packages": ["tool"], "target": {"os": "linux", "arch": "x86_64",
                "abi": "legacy-special", "minimum_os": "legacy-floor"}
        }));
    let diagnostics = check(&old.to_string()).unwrap_err();
    let layout = diagnostics
        .iter()
        .find(|d| d.code == codes::UNKNOWN_WORKSPACE_REF && d.message.contains("fixed_prefix"))
        .expect("v4 fixed-prefix selection rejected");
    assert_eq!(layout.labels.iter().filter(|l| l.span.is_some()).count(), 2);
}

#[test]
fn versions_reject_each_others_fields_and_bad_refs() {
    let mut old = old_package();
    old["workspace"]["outputs"][0]["layout"] = json!({"kind": "relocatable"});
    assert_eq!(parse(&old.to_string()).unwrap_err().code, codes::MALFORMED);
    let mut old = old_package();
    old["workspace"]["outputs"][0]["target"]["minimum_os"] = json!({"major": 6, "minor": 1});
    assert_eq!(parse(&old.to_string()).unwrap_err().code, codes::MALFORMED);
    let mut old = old_package();
    old["workspace"]["outputs"][0]["runtime"] = json!(["missing"]);
    assert!(
        check(&old.to_string())
            .unwrap_err()
            .iter()
            .any(|d| d.code == codes::UNKNOWN_WORKSPACE_REF)
    );
    let mut old = old_package();
    old["workspace"]["outputs"][0]["unexpected"] = json!(true);
    assert_eq!(parse(&old.to_string()).unwrap_err().code, codes::MALFORMED);
}
