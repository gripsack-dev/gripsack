use gripsack_ir::{
    check, codes,
    workspace_model::{
        WorkspaceContent, WorkspaceOutput, WorkspaceSource, graph,
        identity::{self, DefinitionDigest, PlanDigest, RecipeDigest},
        lock::DefinitionPins,
    },
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn document() -> Value {
    json!({"ir_version":8,"host":{"os":"linux","arch":"x86_64"},"workspace":{
        "span":{"file":"ergonomics.ts","line":1},"outputs":[
            {"kind":"package","name":"tool","span":{"file":"ergonomics.ts","line":2},
             "producer":{"kind":"provider","provider":{"kind":"fetch","fetch":{"kind":"file","path":"payload"},"span":{"file":"ergonomics.ts","line":2}}},
             "commands":{"tool":"bin/tool","other":"bin/other"},"target":{"os":"linux","arch":"x86_64"},"layout":{"kind":"relocatable"}},
            {"kind":"environment","name":"tools","span":{"file":"ergonomics.ts","line":3},
             "packages":["tool"],"target":{"os":"linux","arch":"x86_64"}},
            {"kind":"profile","name":"home","span":{"file":"ergonomics.ts","line":4},
             "environment":"tools","files":[{
                "span":{"file":"ergonomics.ts","line":5},"source":{"kind":"repo_file","path":"config.tmpl"},
                "content":{"kind":"identity"},"destination":{"kind":"tracked_copy","path":"~/.config/tool"}
             }]}
        ]
    }})
}

fn assert_error_at(document: &Value, code: &str, line: u32) {
    let diagnostics = check(&document.to_string()).unwrap_err();
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic.code == code
            && diagnostic.labels.iter().any(|label| label
                .span
                .as_ref()
                .is_some_and(|span| span.file == "ergonomics.ts" && span.line == line))),
        "{diagnostics:?}"
    );
}

fn plan(document: &Value) -> PlanDigest {
    let ir = check(&document.to_string()).unwrap();
    identity::plan_digest(
        ir.workspace_catalog.as_ref().unwrap(),
        &ir.host,
        &DefinitionPins {
            frontend: DefinitionDigest::parse(&"a".repeat(64)).unwrap(),
            imports: BTreeMap::new(),
        },
        &BTreeMap::new(),
        None,
    )
}

#[test]
fn captured_templates_require_source_and_bind_variables_and_result_claim() {
    let mut doc = document();
    doc["workspace"]["outputs"][2]["files"][0]["content"] =
        json!({"kind":"template_file","variables":{"name":"Alice"}});
    let original = plan(&doc);
    let ir = check(&doc.to_string()).unwrap();
    let workspace = ir.workspace_catalog.unwrap();
    assert!(workspace.requires_v8());
    assert!(!workspace.requires_v7());
    let WorkspaceOutput::Profile(profile) = &workspace.outputs[2] else {
        panic!("profile")
    };
    assert!(matches!(
        &profile.files[0].content,
        WorkspaceContent::TemplateFile {
            result_digest: None,
            ..
        }
    ));
    doc["workspace"]["outputs"][2]["files"][0]["content"]["variables"]["name"] = json!("Bob");
    assert_ne!(plan(&doc), original);
    doc["workspace"]["outputs"][2]["files"][0]["content"]["variables"] = json!({"other":"Alice"});
    assert_ne!(plan(&doc), original);
    doc["workspace"]["outputs"][2]["files"][0]["content"]["variables"] = json!({"name":"Alice"});
    doc["workspace"]["outputs"][2]["files"][0]["content"]["result_digest"] = json!("b".repeat(64));
    assert_ne!(plan(&doc), original);
    doc["workspace"]["outputs"][2]["files"][0]["content"]["result_digest"] = json!("B".repeat(64));
    assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 5);
    doc["workspace"]["outputs"][2]["files"][0]["content"]
        .as_object_mut()
        .unwrap()
        .remove("result_digest");
    doc["workspace"]["outputs"][2]["files"][0]
        .as_object_mut()
        .unwrap()
        .remove("source");
    assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 5);
}

#[test]
fn repository_trees_default_to_all_and_have_no_artifact_reference() {
    let mut doc = document();
    doc["workspace"]["outputs"][2]["files"][0]["source"] = json!({"kind":"repo_tree","path":"."});
    let ir = check(&doc.to_string()).unwrap();
    let workspace = ir.workspace_catalog.unwrap();
    let WorkspaceOutput::Profile(profile) = &workspace.outputs[2] else {
        panic!("profile")
    };
    assert!(
        matches!(&profile.files[0].source, Some(WorkspaceSource::RepoTree { include, exclude, .. })
        if include.is_empty() && exclude.is_empty())
    );
    let edges = graph::references(&workspace);
    let profile_edges: Vec<_> = edges
        .iter()
        .filter(|edge| edge.from.name() == "home")
        .collect();
    assert_eq!(profile_edges.len(), 1);
    assert_eq!(profile_edges[0].to, "tools");
    let original = plan(&doc);
    doc["workspace"]["outputs"][2]["files"][0]["source"]["include"] = json!([]);
    doc["workspace"]["outputs"][2]["files"][0]["source"]["exclude"] = json!([]);
    assert_eq!(plan(&doc), original);
    doc["workspace"]["outputs"][2]["files"][0]["source"]["path"] = json!("config");
    assert_ne!(plan(&doc), original);
    let unselected = plan(&doc);
    doc["workspace"]["outputs"][2]["files"][0]["source"]["include"] = json!(["bin", "config/tool"]);
    let selected = plan(&doc);
    assert_ne!(selected, unselected);
    doc["workspace"]["outputs"][2]["files"][0]["source"]["include"] =
        json!(["config/tool", "bin", "bin"]);
    assert_eq!(
        plan(&doc),
        selected,
        "selection order/duplicates are not authority"
    );
    doc["workspace"]["outputs"][2]["files"][0]["source"]["exclude"] = json!(["config/tool/cache"]);
    assert_ne!(plan(&doc), selected);
    for invalid in [
        "",
        "../escape",
        "/absolute",
        "a//b",
        "a/./b",
        "a/",
        "**/*.toml",
        "a?",
        "[ab]",
        "a\\b",
        "a\nb",
    ] {
        doc["workspace"]["outputs"][2]["files"][0]["source"]["exclude"] = json!([]);
        doc["workspace"]["outputs"][2]["files"][0]["source"]["include"] = json!([invalid]);
        assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 5);
        doc["workspace"]["outputs"][2]["files"][0]["source"]["include"] = json!([]);
        doc["workspace"]["outputs"][2]["files"][0]["source"]["exclude"] = json!([invalid]);
        assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 5);
        doc["workspace"]["outputs"][2]["files"][0]["source"]["exclude"] = json!([]);
        doc["workspace"]["outputs"][2]["files"][0]["source"]["path"] = json!(invalid);
        assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 5);
        doc["workspace"]["outputs"][2]["files"][0]["source"]["path"] = json!("config");
    }
}

#[test]
fn every_tree_requires_identity_content() {
    for source in [
        json!({"kind":"repo_tree","path":"config"}),
        json!({"kind":"tree","output":"tool","include":["bin"]}),
    ] {
        for content in [
            json!({"kind":"template_file","variables":{}}),
            json!({"kind":"literal","text":"replacement"}),
            json!({"kind":"template","template":"{{value}}","variables":{"value":"x"}}),
        ] {
            let mut doc = document();
            doc["workspace"]["outputs"][2]["files"][0]["source"] = source.clone();
            doc["workspace"]["outputs"][2]["files"][0]["content"] = content;
            assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 5);
        }
    }
}

#[test]
fn launchers_require_explicit_environment_unique_exports_and_safe_destination() {
    let mut doc = document();
    doc["workspace"]["outputs"][2]["launchers"] =
        json!({"directory":"~/.local/bin","commands":["tool","other"]});
    let original = plan(&doc);
    doc["workspace"]["outputs"][2]["launchers"]["commands"] = json!(["other", "tool"]);
    assert_eq!(plan(&doc), original);
    doc["workspace"]["outputs"][2]["launchers"]["commands"] = json!(["tool"]);
    assert_ne!(plan(&doc), original);
    doc["workspace"]["outputs"][2]["launchers"]["directory"] = json!("/usr/local/bin");
    assert_ne!(plan(&doc), original);
    for invalid in [
        json!([]),
        json!([""]),
        json!(["tool", "tool"]),
        json!(["../tool"]),
        json!(["."]),
        json!(["tool\n"]),
    ] {
        doc["workspace"]["outputs"][2]["launchers"]["commands"] = invalid;
        assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 4);
    }
    doc["workspace"]["outputs"][2]["launchers"]["commands"] = json!(["missing"]);
    assert_error_at(&doc, codes::UNKNOWN_WORKSPACE_REF, 4);
    doc["workspace"]["outputs"][2]["launchers"]["commands"] = json!(["tool"]);
    for invalid in ["bin", "~", "/", "~/", "/a/../b", "/a//b", "/a/", "/a\nb"] {
        doc["workspace"]["outputs"][2]["launchers"]["directory"] = json!(invalid);
        assert_error_at(&doc, codes::BAD_DESTINATION, 4);
    }
    doc["workspace"]["outputs"][2]["launchers"]["directory"] = json!("~/.local/bin");
    let mut ambiguous = doc.clone();
    let mut second = ambiguous["workspace"]["outputs"][0].clone();
    second["name"] = json!("second");
    ambiguous["workspace"]["outputs"]
        .as_array_mut()
        .unwrap()
        .push(second);
    ambiguous["workspace"]["outputs"][1]["packages"] = json!(["tool", "second"]);
    assert_error_at(&ambiguous, codes::UNKNOWN_WORKSPACE_REF, 4);
    doc["workspace"]["outputs"][2]
        .as_object_mut()
        .unwrap()
        .remove("environment");
    assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 4);
}

#[test]
fn archive_root_is_native_relocatable_authority_and_changes_package_identity() {
    let mut doc = document();
    let original = plan(&doc);
    doc["workspace"]["outputs"][0]["archive_root"] = json!({"kind":"single_directory"});
    assert_ne!(plan(&doc), original);
    let ir = check(&doc.to_string()).unwrap();
    let WorkspaceOutput::Package(mut package) = ir.workspace_catalog.unwrap().outputs.remove(0)
    else {
        panic!("package")
    };
    package.producer = gripsack_ir::workspace_model::WorkspaceProducer::Recipe {
        recipe: "build".into(),
    };
    let recipe = RecipeDigest::parse(&"b".repeat(64)).unwrap();
    let selected = identity::recipe_package_digest(&package, recipe, &BTreeMap::new()).unwrap();
    package.archive_root = None;
    assert_ne!(
        selected,
        identity::recipe_package_digest(&package, recipe, &BTreeMap::new()).unwrap()
    );
    doc["workspace"]["outputs"][0]["layout"] = json!({"kind":"fixed_prefix","prefix":"/opt/tool"});
    assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 2);
    doc["workspace"]["outputs"][0]["layout"] = json!({"kind":"prefix_materialized"});
    doc["workspace"]["outputs"][0]["producer"]["provider"] = json!({
        "kind":"conda_environment","channels":["conda-forge"],"packages":{"tool":"==1"},
        "span":{"file":"ergonomics.ts","line":2}
    });
    assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 2);
    let source = doc["workspace"]["outputs"][0]["producer"]["provider"].clone();
    doc["workspace"]["outputs"][0]["layout"] = json!({"kind":"relocatable"});
    doc["workspace"]["outputs"][0]["producer"] = json!({"kind":"recipe","recipe":"build"});
    doc["workspace"]["outputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "kind":"recipe","name":"build","span":{"file":"ergonomics.ts","line":6},
            "source":source,"execution":{"kind":"host","access":"unconfined"},
            "output_kind":"tree","target":{"os":"linux","arch":"x86_64"}
        }));
    assert_error_at(&doc, codes::INVALID_WORKSPACE_VALUE, 2);
}

#[test]
fn declaration_locations_do_not_change_new_identities() {
    let mut doc = document();
    doc["workspace"]["outputs"][0]["archive_root"] = json!({"kind":"single_directory"});
    doc["workspace"]["outputs"][2]["launchers"] =
        json!({"directory":"~/.local/bin","commands":["tool"]});
    doc["workspace"]["outputs"][2]["files"][0]["content"] =
        json!({"kind":"template_file","variables":{}});
    let original = plan(&doc);
    doc["workspace"]["outputs"][0]["span"]["line"] = json!(100);
    doc["workspace"]["outputs"][2]["span"]["line"] = json!(101);
    doc["workspace"]["outputs"][2]["files"][0]["span"]["line"] = json!(102);
    assert_eq!(plan(&doc), original);
}

#[test]
fn explicit_null_cannot_erase_new_optional_authority() {
    let base = document();
    for (index, field) in [(0, "archive_root"), (2, "launchers")] {
        let mut output = base["workspace"]["outputs"][index].clone();
        assert!(serde_json::from_value::<WorkspaceOutput>(output.clone()).is_ok());
        output[field] = Value::Null;
        assert!(serde_json::from_value::<WorkspaceOutput>(output).is_err());
    }
    assert!(
        serde_json::from_value::<WorkspaceContent>(json!({
            "kind":"template_file","variables":{},"result_digest":null
        }))
        .is_err()
    );
}
