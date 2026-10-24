use gripsack_ir::{check, parse, workspace::WorkspaceOperation};
use serde_json::{Value, json};

fn document(version: u32) -> Value {
    json!({"ir_version":version,"host":{"os":"linux","arch":"x86_64"},"workspace":{
        "span":{"file":"gripsack.ts","line":1},"outputs":[
            {"kind":"package","name":"tool","span":{"file":"gripsack.ts","line":2},
             "producer":{"kind":"provider","provider":{"kind":"fetch","fetch":{"kind":"file","path":"payload"},"span":{"file":"gripsack.ts","line":2}}},
             "commands":{"tool":"bin/tool"},"target":{"os":"linux","arch":"x86_64"},"layout":{"kind":"relocatable"}},
            {"kind":"environment","name":"tools","span":{"file":"gripsack.ts","line":3},"packages":["tool"],"target":{"os":"linux","arch":"x86_64"}},
            {"kind":"profile","name":"home","span":{"file":"gripsack.ts","line":4},"environment":"tools","files":[
                {"span":{"file":"gripsack.ts","line":5},"source":{"kind":"repo_file","path":"config"},"content":{"kind":"identity"},"destination":{"kind":"tracked_copy","path":"~/.config/tool"}}
            ]}
        ]
    }})
}

fn schema(version: u32) -> jsonschema::Validator {
    let source = match version {
        6 => include_str!("../schema-v6.json"),
        7 => include_str!("../schema-v7.json"),
        8 => include_str!("../schema-v8.json"),
        _ => panic!("fixture version"),
    };
    jsonschema::validator_for(&serde_json::from_str::<Value>(source).unwrap()).unwrap()
}

fn add_authority(doc: &mut Value, authority: &str) {
    let outputs = &mut doc["workspace"]["outputs"];
    match authority {
        "archive_root" => outputs[0]["archive_root"] = json!({"kind":"single_directory"}),
        "launchers" => {
            outputs[2]["launchers"] = json!({"directory":"~/.local/bin","commands":["tool"]})
        }
        "repo_tree" => {
            outputs[2]["files"][0]["source"] = json!({"kind":"repo_tree","path":"config"})
        }
        "template_file" => {
            outputs[2]["files"][0]["content"] =
                json!({"kind":"template_file","variables":{"name":"tool"}})
        }
        _ => panic!("fixture authority"),
    }
}

#[test]
fn retained_catalog_versions_round_trip_and_execute_without_new_authority() {
    for version in [6, 7, 8] {
        let doc = document(version);
        assert!(schema(version).is_valid(&doc));
        let ir = check(&doc.to_string()).unwrap();
        for operation in [
            WorkspaceOperation::Update,
            WorkspaceOperation::Build,
            WorkspaceOperation::Run,
        ] {
            assert!(ir.workspace_execution_error(operation).is_none());
        }
        let saved = serde_json::to_value(&ir).unwrap();
        assert_eq!(saved["ir_version"], version);
        assert!(check(&saved.to_string()).is_ok());
    }
}

#[test]
fn v8_authority_is_rejected_by_each_retained_catalog_reader() {
    for authority in ["archive_root", "launchers", "repo_tree", "template_file"] {
        let mut current = document(8);
        add_authority(&mut current, authority);
        assert!(schema(8).is_valid(&current), "{authority}");
        let ir = check(&current.to_string()).unwrap();
        for version in [6, 7] {
            let mut old = current.clone();
            old["ir_version"] = json!(version);
            assert!(!schema(version).is_valid(&old), "{authority} in v{version}");
            let diagnostic = parse(&old.to_string()).unwrap_err();
            let line = match authority {
                "archive_root" => 2,
                "launchers" => 4,
                _ => 5,
            };
            assert!(
                diagnostic
                    .labels
                    .iter()
                    .any(|label| { label.span.as_ref().is_some_and(|span| span.line == line) }),
                "{authority} in v{version}: {diagnostic:?}"
            );
            let mut forged = ir.clone();
            forged.ir_version = version;
            assert!(serde_json::to_value(&forged).is_err());
            assert!(!gripsack_ir::sema::run(&forged).is_empty());
            assert!(
                forged
                    .workspace_execution_error(WorkspaceOperation::Run)
                    .is_some()
            );
        }
    }
}

#[test]
fn nested_new_authority_is_closed_and_labels_its_declaring_span() {
    for (authority, pointer, line) in [
        ("archive_root", "/workspace/outputs/0/archive_root", 2),
        ("repo_tree", "/workspace/outputs/2/files/0/source", 5),
        ("template_file", "/workspace/outputs/2/files/0/content", 5),
    ] {
        let mut doc = document(8);
        add_authority(&mut doc, authority);
        doc.pointer_mut(pointer).unwrap()["unexpected"] = json!(true);
        assert!(!schema(8).is_valid(&doc));
        let diagnostic = parse(&doc.to_string()).unwrap_err();
        assert!(
            diagnostic
                .labels
                .iter()
                .any(|label| { label.span.as_ref().is_some_and(|span| span.line == line) })
        );
    }
    let mut doc = document(8);
    add_authority(&mut doc, "launchers");
    doc["workspace"]["outputs"][2]["launchers"]["unexpected"] = json!(true);
    assert!(!schema(8).is_valid(&doc));
    let diagnostic = parse(&doc.to_string()).unwrap_err();
    assert!(
        diagnostic
            .labels
            .iter()
            .any(|label| { label.span.as_ref().is_some_and(|span| span.line == 4) })
    );
}

#[test]
fn new_declarations_still_require_source_provenance() {
    for authority in ["archive_root", "launchers", "repo_tree", "template_file"] {
        let mut doc = document(8);
        add_authority(&mut doc, authority);
        let owner = match authority {
            "archive_root" => &mut doc["workspace"]["outputs"][0],
            "launchers" => &mut doc["workspace"]["outputs"][2],
            _ => &mut doc["workspace"]["outputs"][2]["files"][0],
        };
        owner.as_object_mut().unwrap().remove("span");
        assert!(!schema(8).is_valid(&doc));
        assert!(parse(&doc.to_string()).is_err());
    }
}

#[test]
fn schema_and_core_reject_invalid_v8_policy_values() {
    let validator = schema(8);
    for (authority, pointer, invalid) in [
        (
            "archive_root",
            "/workspace/outputs/0/archive_root",
            json!(null),
        ),
        (
            "archive_root",
            "/workspace/outputs/0/archive_root",
            json!({"kind":"first_directory"}),
        ),
        (
            "archive_root",
            "/workspace/outputs/0/layout",
            json!({"kind":"prefix_materialized"}),
        ),
        ("launchers", "/workspace/outputs/2/launchers", json!(null)),
        (
            "launchers",
            "/workspace/outputs/2/launchers/commands",
            json!([]),
        ),
        (
            "launchers",
            "/workspace/outputs/2/launchers/commands",
            json!(["tool", "tool"]),
        ),
        (
            "launchers",
            "/workspace/outputs/2/launchers/commands",
            json!(["../tool"]),
        ),
        (
            "launchers",
            "/workspace/outputs/2/launchers/directory",
            json!("relative"),
        ),
        (
            "repo_tree",
            "/workspace/outputs/2/files/0/source/path",
            json!("../outside"),
        ),
        (
            "repo_tree",
            "/workspace/outputs/2/files/0/source",
            json!({"kind":"repo_tree","path":".","include":["**"]}),
        ),
        (
            "repo_tree",
            "/workspace/outputs/2/files/0/content",
            json!({"kind":"literal","text":"not a tree"}),
        ),
        (
            "template_file",
            "/workspace/outputs/2/files/0/content",
            json!({"kind":"template_file","variables":{},"result_digest":null}),
        ),
    ] {
        let mut doc = document(8);
        add_authority(&mut doc, authority);
        *doc.pointer_mut(pointer).unwrap() = invalid.clone();
        assert!(
            !validator.is_valid(&doc),
            "schema admitted {pointer}: {invalid}"
        );
        assert!(
            check(&doc.to_string()).is_err(),
            "core admitted {pointer}: {invalid}"
        );
    }
    for (authority, pointer, missing) in [
        ("launchers", "/workspace/outputs/2", "environment"),
        ("template_file", "/workspace/outputs/2/files/0", "source"),
    ] {
        let mut doc = document(8);
        add_authority(&mut doc, authority);
        doc.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(missing);
        assert!(!validator.is_valid(&doc));
        assert!(check(&doc.to_string()).is_err());
    }
}
