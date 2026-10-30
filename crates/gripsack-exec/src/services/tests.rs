use super::{
    plan,
    state::{ServiceRecord, ServiceState},
};
use gripsack_ir::{
    Span,
    workspace_model::{ServiceHealth, ServiceOutput},
};
use gripsack_process::Sha256Digest;
use serde_json::json;

fn service() -> ServiceOutput {
    ServiceOutput {
        name: "postgres".into(),
        span: Span {
            file: "gripsack.ts".into(),
            line: 1,
            col: None,
        },
        image: format!("postgres@sha256:{}", "a".repeat(64)),
        ports: Vec::new(),
        volumes: Vec::new(),
        env: Default::default(),
        command: Vec::new(),
        health: ServiceHealth::Command {
            argv: vec!["pg_isready".into()],
            interval_seconds: 1,
            timeout_seconds: 1,
            retries: 10,
        },
        depends_on: vec!["first".into(), "second".into()],
    }
}
fn state() -> ServiceState {
    serde_json::from_value(json!({
        "version":1,"workspace":"a".repeat(64),"owner":"b".repeat(64),"engine":"fixture-engine","socket":"/run/docker.sock","services":[]
    })).unwrap()
}

#[test]
fn provenance_and_unordered_dependencies_do_not_restart_a_service() {
    let original = service();
    let mut moved = original.clone();
    moved.span.file = "other.ts".into();
    moved.span.line = 40;
    moved.depends_on.reverse();
    assert_eq!(
        plan::revision(&original).unwrap(),
        plan::revision(&moved).unwrap()
    );
    moved.command = vec!["postgres".into(), "-c".into(), "max_connections=20".into()];
    assert_ne!(
        plan::revision(&original).unwrap(),
        plan::revision(&moved).unwrap()
    );
}

#[test]
fn foreign_container_identity_cannot_inherit_owned_labels() {
    let state = state();
    let revision = Sha256Digest::of(b"revision");
    let record = ServiceRecord {
        name: "postgres".into(),
        revision,
        container: Some("c".repeat(64)),
    };
    let mut labels = plan::labels(&state, "postgres");
    labels.insert(plan::REVISION_LABEL.into(), revision.to_string());
    let mut value = json!({
        "Id":"c".repeat(64),"Name":format!("/{}",state.workspace.resource("service","postgres")),
        "Config":{"Labels":labels},
        "HostConfig":{"Privileged":false,"NetworkMode":state.workspace.resource("network","default"),"RestartPolicy":{"Name":"no"}}
    });
    assert_eq!(
        plan::admit_container(&value, &state, &record).unwrap(),
        "c".repeat(64)
    );
    value["Id"] = json!("d".repeat(64));
    assert!(plan::admit_container(&value, &state, &record).is_err());
    value["Id"] = json!("c".repeat(64));
    value["Config"]["Labels"][plan::OWNER_LABEL] = json!("e".repeat(64));
    assert!(plan::admit_container(&value, &state, &record).is_err());
}

#[test]
fn readiness_requires_running_and_positive_health_not_old_success() {
    assert!(plan::is_healthy(
        &json!({"State":{"Running":true,"Health":{"Status":"healthy"}}})
    ));
    assert!(!plan::is_healthy(
        &json!({"State":{"Running":false,"Health":{"Status":"healthy"}}})
    ));
    assert!(!plan::is_healthy(
        &json!({"State":{"Running":true,"Health":{"Status":"starting"}}})
    ));
    assert!(!plan::is_healthy(&json!({"State":{"Running":true}})));
}
