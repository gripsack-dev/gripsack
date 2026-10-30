use gripsack_ir::{
    check, codes, parse,
    workspace_model::{identity, lock::DefinitionPins},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn schema() -> jsonschema::Validator {
    jsonschema::validator_for(
        &serde_json::from_str::<Value>(include_str!("../schema-v10.json")).unwrap(),
    )
    .unwrap()
}

fn document() -> Value {
    let span = json!({"file":"services.ts","line":2});
    json!({"ir_version":10,"host":{"os":"linux","arch":"x86_64"},"workspace":{
    "span":span,"outputs":[
        {"kind":"service","name":"db","span":span,
         "image":format!("docker.io/library/postgres@sha256:{}", "a".repeat(64)),
         "ports":[{"host":5432,"container":5432,"address":"127.0.0.1"}],
         "volumes":[{"name":"postgres-data","target":"/var/lib/postgresql/data"}],
         "env":{"POSTGRES_PASSWORD":{"kind":"inherited","name":"DATABASE_PASSWORD"},
                "POSTGRES_DB":{"kind":"literal","value":"development"}},
         "command":["postgres","-c","log_statement=all"],
         "health":{"kind":"command","argv":["pg_isready"],"interval_seconds":5,"timeout_seconds":2,"retries":3}},
        {"kind":"task","name":"test","span":span,"services":["db"],
         "context":{"kind":"host","mutable_paths":[]},
         "steps":[{"command":{"kind":"exec","span":span,"argv":[{"kind":"literal","value":"true"}]}}]}
    ]}})
}

#[test]
fn service_and_task_readiness_are_typed_and_do_not_become_recipe_outputs() {
    let value = document();
    assert!(schema().is_valid(&value));
    let ir = check(&value.to_string()).unwrap();
    let edges = gripsack_ir::workspace_model::graph::references(&ir.workspace);
    assert!(
        edges
            .iter()
            .any(|edge| edge.from.name() == "test" && edge.to == "db" && edge.kinds == ["service"])
    );
    let encoded = serde_json::to_value(&ir).unwrap();
    check(&encoded.to_string()).unwrap();
    assert_eq!(
        encoded["workspace"]["outputs"][0]["env"]["POSTGRES_PASSWORD"],
        json!({"kind":"inherited","name":"DATABASE_PASSWORD"})
    );
}

#[test]
fn service_declarations_are_bounded_and_fail_closed() {
    let validator = schema();
    for (pointer, invalid) in [
        ("/image", json!("postgres:latest")),
        (
            "/image",
            json!(format!(
                "https://registry/postgres@sha256:{}",
                "a".repeat(64)
            )),
        ),
        (
            "/image",
            json!(format!("postgres@sha256:{}", "A".repeat(64))),
        ),
        ("/ports/0/host", json!(0)),
        ("/ports/0/container", json!(65536)),
        ("/ports/0/address", json!("0.0.0.0")),
        ("/ports/0/address", json!("127.000.0.1")),
        ("/ports/0/address", json!("localhost")),
        ("/volumes/0/name", json!("/tmp/database")),
        ("/volumes/0/target", json!("/")),
        ("/volumes/0/target", json!("/../outside")),
        ("/volumes/0/target", json!("/data/../outside")),
        ("/env/POSTGRES_DB/value", json!("bad\0value")),
        ("/env/POSTGRES_PASSWORD/name", json!("BAD-NAME")),
        ("/command", json!([""])),
        ("/command", json!(["postgres", "bad\0argument"])),
        ("/health/argv", json!([])),
        ("/health/interval_seconds", json!(0)),
        ("/health/timeout_seconds", json!(3601)),
        ("/health/retries", json!(101)),
        ("/depends_on", json!(["db", "db"])),
    ] {
        let mut value = document();
        let service = &mut value["workspace"]["outputs"][0];
        if pointer == "/depends_on" {
            service["depends_on"] = invalid;
        } else {
            *service.pointer_mut(pointer).unwrap() = invalid;
        }
        assert!(!validator.is_valid(&value), "schema admitted {pointer}");
        assert!(
            check(&value.to_string()).is_err(),
            "core admitted {pointer}"
        );
    }
    for field in [
        "binds",
        "privileged",
        "network_mode",
        "restart",
        "remove_volumes",
    ] {
        let mut value = document();
        value["workspace"]["outputs"][0][field] = json!(true);
        assert!(!validator.is_valid(&value));
        assert!(parse(&value.to_string()).is_err());
    }
    for field in ["span", "image", "health"] {
        let mut value = document();
        value["workspace"]["outputs"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(!validator.is_valid(&value));
        assert!(parse(&value.to_string()).is_err());
    }
    for field in ["ports", "volumes"] {
        let mut value = document();
        let first = value["workspace"]["outputs"][0][field][0].clone();
        value["workspace"]["outputs"][0][field]
            .as_array_mut()
            .unwrap()
            .push(first);
        assert!(check(&value.to_string()).is_err());
    }
    let mut value = document();
    value["workspace"]["outputs"][0]["ports"] = json!(vec![
        json!({"host":5432,"container":5432,"address":"::1"});
        129
    ]);
    assert!(!validator.is_valid(&value));
    assert!(check(&value.to_string()).is_err());
}

#[test]
fn service_dependencies_reject_wrong_kinds_missing_outputs_and_cycles() {
    for (dependency, code) in [
        ("missing", codes::UNKNOWN_WORKSPACE_REF),
        ("test", codes::UNKNOWN_WORKSPACE_REF),
        ("db", codes::WORKSPACE_CYCLE),
    ] {
        let mut value = document();
        value["workspace"]["outputs"][0]["depends_on"] = json!([dependency]);
        assert!(
            check(&value.to_string())
                .unwrap_err()
                .iter()
                .any(|error| error.code == code)
        );
    }
    let mut value = document();
    value["workspace"]["outputs"][1]["services"] = json!(["test"]);
    assert!(check(&value.to_string()).is_err());
}

fn digest(value: &Value) -> identity::PlanDigest {
    let ir = check(&value.to_string()).unwrap();
    identity::plan_digest(
        &ir.workspace,
        &ir.host,
        &DefinitionPins {
            frontend: identity::DefinitionDigest::parse(&"f".repeat(64)).unwrap(),
            imports: BTreeMap::new(),
        },
        &BTreeMap::new(),
        None,
    )
}

#[test]
fn service_configuration_and_readiness_bind_plan_identity_but_spans_do_not() {
    let value = document();
    let original = digest(&value);
    for (pointer, replacement) in [
        (
            "/workspace/outputs/0/image",
            json!(format!("postgres@sha256:{}", "b".repeat(64))),
        ),
        ("/workspace/outputs/0/ports/0/host", json!(15432)),
        ("/workspace/outputs/0/volumes/0/name", json!("other-data")),
        (
            "/workspace/outputs/0/env/POSTGRES_PASSWORD/name",
            json!("OTHER_PASSWORD"),
        ),
        ("/workspace/outputs/0/health/timeout_seconds", json!(3)),
        ("/workspace/outputs/1/services", json!([])),
    ] {
        let mut changed = value.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        assert_ne!(digest(&changed), original, "{pointer}");
    }
    let mut relocated = value;
    relocated["workspace"]["outputs"][0]["span"]["line"] = json!(200);
    assert_eq!(digest(&relocated), original);
}
