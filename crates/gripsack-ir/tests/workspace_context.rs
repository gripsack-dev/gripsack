use gripsack_ir::{check, codes};

fn workspace() -> serde_json::Value {
    serde_json::json!({"ir_version":10,"host":{"os":"linux","arch":"x86_64"},"workspace":{
    "span":{"file":"gripsack.ts","line":1},"outputs":[
        {"kind":"package","name":"build","span":{"file":"gripsack.ts","line":2},
         "producer":{"kind":"provider","provider":{"kind":"fetch","fetch":{"kind":"file","path":"src"},"span":{"file":"gripsack.ts","line":2}}},
         "commands":{},"target":{"os":"linux","arch":"x86_64"},"layout":{"kind":"relocatable"}},
        {"kind":"check","name":"verify","span":{"file":"checks.ts","line":4},"subject":"build",
         "run":{"kind":"exec","span":{"file":"checks.ts","line":5},"argv":[{"kind":"literal","value":"/bin/true"}]}}
    ]}})
}

#[test]
fn standalone_host_checks_remain_valid_without_recipe_execution() {
    let mut declaration = workspace();
    declaration["workspace"]["outputs"][1]["run"]["cwd"] =
        serde_json::json!({"kind":"host","path":"/tmp/live-checkout"});
    check(&declaration.to_string()).unwrap();
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

fn binding_command(binding: &str, directory: bool) -> serde_json::Value {
    let mut command = serde_json::json!({
        "kind":"exec","span":{"file":"checks.ts","line":5},
        "argv":[{"kind":"literal","value":"/bin/true"}]
    });
    let value = serde_json::json!({"kind":binding,"selector":"."});
    if directory {
        command["cwd"] = value;
    } else {
        command["argv"].as_array_mut().unwrap().push(value);
    }
    command
}

fn assert_binding_refused(declaration: &serde_json::Value) {
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
}

#[test]
fn provider_subject_checks_resolve_source_but_never_grant_staging() {
    for directory in [false, true] {
        let mut declaration = workspace();
        declaration["workspace"]["outputs"][1]["run"] = binding_command("source", directory);
        check(&declaration.to_string()).unwrap();
        declaration["workspace"]["outputs"][1]["run"] = binding_command("output", directory);
        assert_binding_refused(&declaration);
    }
}

#[test]
fn task_checks_resolve_provider_package_subject_without_a_recipe_owner() {
    for directory in [false, true] {
        let mut declaration = workspace();
        let source = declaration["workspace"]["outputs"][0]["producer"]["provider"].clone();
        declaration["workspace"]["outputs"][0] = serde_json::json!({
            "kind":"package","name":"data","span":{"file":"gripsack.ts","line":2},
            "producer":{"kind":"provider","provider":source},
            "commands":{},"target":{"os":"linux","arch":"x86_64"},
            "layout":{"kind":"relocatable"}
        });
        declaration["workspace"]["outputs"][1]["subject"] = serde_json::json!("data");
        declaration["workspace"]["outputs"][1]["run"] = binding_command("source", directory);
        declaration["workspace"]["outputs"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "kind":"task","name":"invoke","span":{"file":"task.ts","line":2},
                "context":{"kind":"host","mutable_paths":[]},"checks":["verify"],
                "steps":[{"command":{"kind":"exec","span":{"file":"task.ts","line":3},
                    "argv":[{"kind":"literal","value":"/bin/true"}]}}]
            }));
        check(&declaration.to_string()).unwrap();
        declaration["workspace"]["outputs"][1]["run"] = binding_command("output", directory);
        assert_binding_refused(&declaration);
    }
}

#[test]
fn task_and_hook_commands_cannot_borrow_source_or_staging_bindings() {
    for binding in ["source", "output"] {
        for directory in [false, true] {
            for hook in [false, true] {
                let mut declaration = workspace();
                let command = binding_command(binding, directory);
                let output = if hook {
                    serde_json::json!({
                        "kind":"hook","name":"invoke","span":{"file":"hook.ts","line":2},
                        "trigger":"post_link","run":command
                    })
                } else {
                    serde_json::json!({
                        "kind":"task","name":"invoke","span":{"file":"task.ts","line":2},
                        "context":{"kind":"host","mutable_paths":[]},"steps":[{"command":command}]
                    })
                };
                declaration["workspace"]["outputs"]
                    .as_array_mut()
                    .unwrap()
                    .push(output);
                assert_binding_refused(&declaration);
            }
        }
    }
}

#[test]
fn task_subject_postconditions_do_not_acquire_artifact_bindings() {
    for binding in ["source", "output"] {
        for directory in [false, true] {
            let mut declaration = workspace();
            declaration["workspace"]["outputs"][0] = serde_json::json!({
                "kind":"task","name":"build","span":{"file":"task.ts","line":2},
                "context":{"kind":"host","mutable_paths":[]},"checks":["verify"],
                "steps":[{"command":{"kind":"exec","span":{"file":"task.ts","line":3},
                    "argv":[{"kind":"literal","value":"/bin/true"}]}}]
            });
            declaration["workspace"]["outputs"][1]["run"] = binding_command(binding, directory);
            assert_binding_refused(&declaration);
        }
    }
}

#[test]
fn literal_bash_parameter_expansion_is_not_javascript_interpolation() {
    let mut declaration = workspace();
    declaration["workspace"]["outputs"][0]["commands"] = serde_json::json!({"bash":"bin/bash"});
    declaration["workspace"]["outputs"][1]["run"] = serde_json::json!({
        "kind":"run_bash","span":{"file":"checks.ts","line":5},
        "interpreter":{"kind":"package_command","package":"build","command":"bash"},
        "options":["-e","-u","-o","pipefail"],
        "body":"printf '%s\\n' \"${HOME}\" \"${VALUE:-default}\"",
        "line_map":[5]
    });
    check(&declaration.to_string()).unwrap();
    declaration["workspace"]["outputs"][1]["run"]["line_map"] = serde_json::json!([0]);
    assert!(check(&declaration.to_string()).is_err());
}
