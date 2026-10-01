use gripsack_ir::{check, codes};

fn workspace() -> serde_json::Value {
    serde_json::json!({"ir_version":6,"host":{"os":"linux","arch":"x86_64"},"workspace":{
    "span":{"file":"gripsack.ts","line":1},"outputs":[
        {"kind":"recipe","name":"build","span":{"file":"gripsack.ts","line":2},
         "source":{"kind":"fetch","fetch":{"kind":"file","path":"src"},"span":{"file":"gripsack.ts","line":2}},
         "execution":{"kind":"isolated_linux","worker":"buildkit","platform":{"os":"linux","arch":"x86_64"},
            "toolchain":{"reference":format!("docker.io/library/toolchain@sha256:{}", "a".repeat(64))}},
         "output_kind":"tree","target":{"os":"linux","arch":"x86_64"},"checks":["verify"]},
        {"kind":"check","name":"verify","span":{"file":"checks.ts","line":4},"subject":"build",
         "run":{"kind":"exec","span":{"file":"checks.ts","line":5},"argv":[{"kind":"literal","value":"/bin/true"}]}}
    ]}})
}

#[test]
fn required_check_cannot_smuggle_a_live_host_directory_into_isolated_production() {
    let mut declaration = workspace();
    check(&declaration.to_string()).unwrap();
    declaration["workspace"]["outputs"][1]["run"]["cwd"] =
        serde_json::json!({"kind":"host","path":"/tmp/live-checkout"});
    let rejected = check(&declaration.to_string()).unwrap_err();
    assert!(
        rejected
            .iter()
            .any(|error| error.code == codes::BAD_WORKSPACE_CONTEXT
                && error.labels.iter().any(|label| label
                    .span
                    .as_ref()
                    .is_some_and(|span| span.file == "checks.ts" && span.line == 5)))
    );
    // A standalone host check remains a valid declaration. Context comes from
    // the required-check use site, not a blanket ban on shared commands.
    declaration["workspace"]["outputs"][0]["checks"] = serde_json::json!([]);
    assert!(check(&declaration.to_string()).is_ok());
}

#[test]
fn task_postconditions_keep_task_subjects_without_becoming_build_dependencies() {
    let mut declaration = workspace();
    declaration["workspace"]["outputs"][0] = serde_json::json!({
        "kind":"task","name":"build","span":{"file":"task.ts","line":2},
        "context":{"kind":"host","mutable_paths":[]},"checks":["verify"],
        "steps":[{"command":{"kind":"exec","span":{"file":"task.ts","line":3},"argv":[{"kind":"literal","value":"true"}]}}]
    });
    check(&declaration.to_string()).unwrap();
    declaration["workspace"]["outputs"][0]["deps"] = serde_json::json!(["build"]);
    assert!(
        check(&declaration.to_string())
            .unwrap_err()
            .iter()
            .any(|error| error.code == codes::WORKSPACE_CYCLE)
    );
}

#[test]
fn production_bindings_cannot_be_borrowed_by_tasks_or_their_postconditions() {
    for binding in ["source", "output"] {
        let mut declaration = workspace();
        declaration["workspace"]["outputs"][1]["run"]["argv"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"kind":binding,"selector":"result"}));
        check(&declaration.to_string()).unwrap();
        declaration["workspace"]["outputs"][0] = serde_json::json!({
            "kind":"task","name":"build","span":{"file":"task.ts","line":2},
            "context":{"kind":"host","mutable_paths":[]},"checks":["verify"],
            "steps":[{"command":{"kind":"exec","span":{"file":"task.ts","line":3},"argv":[{"kind":"literal","value":"true"}]}}]
        });
        let rejected = check(&declaration.to_string()).unwrap_err();
        assert!(
            rejected
                .iter()
                .any(|error| error.code == codes::BAD_WORKSPACE_CONTEXT
                    && error.labels.iter().any(|label| label
                        .span
                        .as_ref()
                        .is_some_and(|span| span.file == "checks.ts" && span.line == 5))),
            "{rejected:?}"
        );
        declaration["workspace"]["outputs"][1]["run"]["argv"] =
            serde_json::json!([{"kind":"literal","value":"true"}]);
        declaration["workspace"]["outputs"][0]["steps"] = serde_json::json!([{"command":{
            "kind":"exec","span":{"file":"task.ts","line":3},"argv":[{"kind":"literal","value":"true"}],
            "cwd":{"kind":binding,"selector":"."}
        }}]);
        assert!(
            check(&declaration.to_string())
                .unwrap_err()
                .iter()
                .any(|error| error.code == codes::BAD_WORKSPACE_CONTEXT)
        );
    }
}
