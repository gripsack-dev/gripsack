use gripsack_ir::{check, parse};
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

fn schema() -> jsonschema::Validator {
    jsonschema::validator_for(
        &serde_json::from_str::<Value>(include_str!("../schema-v10.json")).unwrap(),
    )
    .unwrap()
}

#[test]
fn tree_filters_are_segment_prefixes_and_only_artifacts_expand_tokens() {
    let validator = schema();
    let tokens = "v{version}-{version.bare}/{os}/{arch}/{arch.go}/{arch.x64}/{target}/{system}";
    for origin in ["repo_tree", "tree", "repo_directory"] {
        let (pointer, line) = if origin == "repo_directory" {
            ("/workspace/inputs/0/origin", 7)
        } else {
            ("/workspace/outputs/2/files/0/source", 5)
        };
        for field in ["include", "exclude"] {
            for prefix in [
                "bin",
                "bin/debug",
                tokens,
                "bin/*",
                "bin/?",
                "bin/[ab]",
                "bin/{a,b}",
                "bin/{unknown}",
                "bin/{version",
                "bin/file\x7f",
            ] {
                let mut doc = document(gripsack_ir::IR_VERSION);
                let mut selection = json!({"kind":origin,"include":["bin"],"exclude":[]});
                selection[field] = json!([prefix]);
                if origin == "tree" {
                    selection["output"] = json!("tool");
                } else {
                    selection["path"] = json!(".");
                }
                if origin == "repo_directory" {
                    doc["workspace"]["inputs"] = json!([{
                        "name":"assets","span":{"file":"gripsack.ts","line":line},"origin":selection
                    }]);
                } else {
                    *doc.pointer_mut(pointer).unwrap() = selection;
                }
                let valid =
                    matches!(prefix, "bin" | "bin/debug") || (prefix == tokens && origin == "tree");
                let case = format!("{origin}.{field}: {prefix}");
                assert_eq!(validator.is_valid(&doc), valid, "{case}");
                let result = check(&doc.to_string());
                if valid {
                    result.unwrap();
                } else {
                    let diagnostics = result.unwrap_err();
                    assert!(
                        diagnostics.iter().any(|d| {
                            d.code == gripsack_ir::codes::INVALID_WORKSPACE_VALUE
                                && d.labels.iter().any(|l| {
                                    l.span
                                        .as_ref()
                                        .is_some_and(|s| s.file == "gripsack.ts" && s.line == line)
                                })
                        }),
                        "{case}: {diagnostics:?}"
                    );
                }
            }
        }
    }
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
fn nested_new_authority_is_closed_and_labels_its_declaring_span() {
    for (authority, pointer, line) in [
        ("archive_root", "/workspace/outputs/0/archive_root", 2),
        ("repo_tree", "/workspace/outputs/2/files/0/source", 5),
        ("template_file", "/workspace/outputs/2/files/0/content", 5),
    ] {
        let mut doc = document(gripsack_ir::IR_VERSION);
        add_authority(&mut doc, authority);
        doc.pointer_mut(pointer).unwrap()["unexpected"] = json!(true);
        assert!(!schema().is_valid(&doc));
        let diagnostic = parse(&doc.to_string()).unwrap_err();
        assert!(
            diagnostic
                .labels
                .iter()
                .any(|label| { label.span.as_ref().is_some_and(|span| span.line == line) })
        );
    }
    let mut doc = document(gripsack_ir::IR_VERSION);
    add_authority(&mut doc, "launchers");
    doc["workspace"]["outputs"][2]["launchers"]["unexpected"] = json!(true);
    assert!(!schema().is_valid(&doc));
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
        let mut doc = document(gripsack_ir::IR_VERSION);
        add_authority(&mut doc, authority);
        let owner = match authority {
            "archive_root" => &mut doc["workspace"]["outputs"][0],
            "launchers" => &mut doc["workspace"]["outputs"][2],
            _ => &mut doc["workspace"]["outputs"][2]["files"][0],
        };
        owner.as_object_mut().unwrap().remove("span");
        assert!(!schema().is_valid(&doc));
        assert!(parse(&doc.to_string()).is_err());
    }
}

#[test]
fn schema_and_core_reject_invalid_policy_values() {
    let validator = schema();
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
        let mut doc = document(gripsack_ir::IR_VERSION);
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
        let mut doc = document(gripsack_ir::IR_VERSION);
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
