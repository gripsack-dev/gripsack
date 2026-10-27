use gripsack_ir::{check, codes};
use serde_json::{Value, json};

fn fixture() -> Value {
    let span = json!({"file":"files.ts","line":1});
    json!({"ir_version":9,"host":{"os":"linux","arch":"x86_64"},"workspace":{
        "span":span,"outputs":[
            {"kind":"profile","name":"personal","span":span,"files":[{
                "span":span,"source":{"kind":"repo_file","path":"config"},
                "content":{"kind":"identity"},
                "destination":{"kind":"tracked_copy","path":"~/.config/tool"},
                "checks":[{"check":"syntax","subject":"rendered","stage":"pre_flip","span":span}]
            }]},
            {"kind":"check","name":"syntax","subject":"personal","span":span,
             "run":{"kind":"exec","span":span,"argv":[
                {"kind":"literal","value":"/bin/true"},{"kind":"source","selector":"."}
             ]}}
        ]
    }})
}

fn rejected(document: &Value) {
    let errors = check(&document.to_string()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == codes::BAD_WORKSPACE_CONTEXT),
        "{errors:?}"
    );
}

#[test]
fn attached_file_subject_does_not_grant_neighbor_output_or_directory_authority() {
    let mut document = fixture();
    assert!(check(&document.to_string()).is_ok());
    document["workspace"]["outputs"][1]["run"]["argv"][1]["selector"] = json!("sibling");
    rejected(&document);
    document["workspace"]["outputs"][1]["run"]["argv"][1] = json!({"kind":"output","selector":"."});
    rejected(&document);
    document["workspace"]["outputs"][1]["run"]["argv"][1] = json!({"kind":"source","selector":"."});
    document["workspace"]["outputs"][1]["run"]["cwd"] = json!({"kind":"source","selector":"."});
    rejected(&document);
}

#[test]
fn file_authority_requires_attachment_to_its_actual_owner() {
    let mut document = fixture();
    document["workspace"]["outputs"][0]["files"][0]["checks"] = json!([]);
    rejected(&document);
    document = fixture();
    document["workspace"]["outputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "kind":"profile","name":"foreign","span":{"file":"files.ts","line":9},"files":[]
        }));
    document["workspace"]["outputs"][1]["subject"] = json!("foreign");
    rejected(&document);
}

#[test]
fn file_validation_stage_and_source_are_not_optional_authority() {
    let mut document = fixture();
    document["workspace"]["outputs"][0]["files"][0]["checks"][0]["stage"] = json!("post_link");
    rejected(&document);
    document = fixture();
    let file = &mut document["workspace"]["outputs"][0]["files"][0];
    file.as_object_mut().unwrap().remove("source");
    file["content"] = json!({"kind":"literal","text":"configuration"});
    file["checks"][0]["subject"] = json!("source");
    rejected(&document);
}

#[test]
fn file_bound_check_cannot_be_reused_as_task_postcondition() {
    let mut document = fixture();
    document["workspace"]["outputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "kind":"task","name":"misuse","span":{"file":"files.ts","line":12},
            "context":{"kind":"host","mutable_paths":[]},
            "steps":[{"command":{"kind":"exec","span":{"file":"files.ts","line":13},
              "argv":[{"kind":"literal","value":"/bin/true"}]}}],"checks":["syntax"]
        }));
    rejected(&document);
}
