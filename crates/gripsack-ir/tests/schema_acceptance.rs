//! Current schema/parser parity. Authored-value constraints additionally use sema;
//! serialization alone does not grant admission. No historical readers participate.
use gripsack_ir::{check, codes, parse};
use serde_json::{Value, json};

fn schema() -> jsonschema::Validator {
    jsonschema::validator_for(
        &serde_json::from_str::<Value>(include_str!("../schema-v9.json")).unwrap(),
    )
    .unwrap()
}
fn span(line: u32) -> Value {
    json!({"file":"gripsack.ts","line":line})
}
fn command() -> Value {
    json!({"kind":"exec","span":span(20),"argv":[{"kind":"literal","value":"true"}]})
}
fn document() -> Value {
    let target = json!({"os":"linux","arch":"x86_64"});
    json!({"ir_version":9,"host":target,"workspace":{"span":span(1),"outputs":[
        {"kind":"recipe","name":"build","span":span(2),
         "source":{"kind":"fetch","fetch":{"kind":"file","path":"src"},"span":span(12)},
         "execution":{"kind":"host","access":"unconfined"},"output_kind":"tree","target":target,
         "steps":[{"command":command()}]},
        {"kind":"package","name":"tool","span":span(3),"producer":{"kind":"recipe","recipe":"build"},
         "commands":{"tool":"bin/tool"},"target":target,"layout":{"kind":"relocatable"}},
        {"kind":"environment","name":"dev","span":span(4),"packages":["tool"],"target":target},
        {"kind":"task","name":"test","span":span(5),"context":{"kind":"host","mutable_paths":[]},
         "steps":[{"command":command()}],"environment":"dev"},
        {"kind":"check","name":"smoke","span":span(6),"subject":"tool","run":command()},
        {"kind":"schedule","name":"daily","span":span(7),"task":"test","scope":"user",
         "trigger":{"kind":"daily","time":"03:30"}},
        {"kind":"image","name":"image","span":span(8),"packages":["tool"],"target":target},
        {"kind":"hook","name":"refresh","span":span(9),"trigger":"post_activate","run":command()},
        {"kind":"profile","name":"home","span":span(10),"environment":"dev","hooks":["refresh"],
         "schedules":["daily"],"files":[{"span":span(11),"source":{"kind":"repo_file","path":"config"},
             "content":{"kind":"identity"},"destination":{"kind":"tracked_copy","path":"~/.config/tool"}}]}
    ]}})
}
fn admitted(validator: &jsonschema::Validator, doc: &Value) {
    assert!(validator.is_valid(doc), "schema rejected {doc}");
    let ir = parse(&doc.to_string()).unwrap();
    let saved = serde_json::to_value(ir).unwrap();
    assert!(
        validator.is_valid(&saved),
        "serialized value left schema: {saved}"
    );
    parse(&saved.to_string()).unwrap();
}
fn rejected(validator: &jsonschema::Validator, doc: &Value) {
    assert!(!validator.is_valid(doc), "schema admitted {doc}");
    assert!(parse(&doc.to_string()).is_err(), "parser admitted {doc}");
}

#[test]
fn current_output_graph_is_admitted_and_round_trips() {
    let doc = document();
    admitted(&schema(), &doc);
    check(&doc.to_string()).unwrap();
}

#[test]
fn source_and_command_unions_preserve_schema_parser_parity() {
    let validator = schema();
    for fetch in [
        json!({"kind":"file","path":"payload"}),
        json!({"kind":"git","url":"https://example.test/repo","rev":"main"}),
        json!({"kind":"tarball","url":"https://example.test/src.tgz","sha256":"a".repeat(64)}),
        json!({"kind":"github_release","repo":"owner/repo","asset":"tool-{version}.tgz"}),
        json!({"kind":"plugin","name":"custom","args":{"channel":"stable"}}),
    ] {
        let mut doc = document();
        doc["workspace"]["outputs"][0]["source"]["fetch"] = fetch;
        admitted(&validator, &doc);
    }
    for source in [
        json!({"kind":"conda_environment","channels":["conda-forge"],"packages":{"python":"==3.12.1"},"span":span(12)}),
        json!({"kind":"pixi_lock","manifest":"pixi.toml","lock":"pixi.lock","environment":"default","span":span(12)}),
    ] {
        let mut doc = document();
        doc["workspace"]["outputs"][1]["producer"] = json!({"kind":"provider","provider":source});
        doc["workspace"]["outputs"][1]["layout"] = json!({"kind":"prefix_materialized"});
        admitted(&validator, &doc);
    }
    let mut doc = document();
    doc["workspace"]["outputs"][0]["steps"] = json!([
        {"action":{"kind":"ensure_artifact","output":"tool","span":span(21)}},
        {"command":{"kind":"run_bash","span":span(20),
            "interpreter":{"kind":"package_command","package":"tool","command":"tool"},
            "options":["-e","-u","-o","pipefail"],"body":"true","line_map":[1]}}
    ]);
    admitted(&validator, &doc);
    for options in [
        json!([]),
        json!(["-e", "-u", "-o"]),
        json!(["-u", "-e", "-o", "pipefail"]),
        json!(["-e", "-u", "-o", "pipefail", "-x"]),
    ] {
        doc["workspace"]["outputs"][0]["steps"][1]["command"]["options"] = options;
        assert!(!validator.is_valid(&doc));
        assert!(check(&doc.to_string()).is_err());
    }
    for arg in [
        json!({"kind":"literal","value":"x"}),
        json!({"kind":"source","selector":"."}),
        json!({"kind":"output","selector":"."}),
        json!({"kind":"input","input":"config"}),
        json!({"kind":"artifact","output":"tool","selector":"bin/tool"}),
        json!({"kind":"package_command","package":"tool","command":"tool"}),
    ] {
        let mut doc = document();
        doc["workspace"]["outputs"][0]["steps"][0]["command"]["argv"] = json!([arg]);
        admitted(&validator, &doc);
    }
    for cwd in [
        json!({"kind":"literal","value":"."}),
        json!({"kind":"host","path":"/tmp"}),
        json!({"kind":"source","selector":"."}),
        json!({"kind":"output","selector":"."}),
        json!({"kind":"artifact","output":"tool","selector":"."}),
    ] {
        let mut doc = document();
        doc["workspace"]["outputs"][0]["steps"][0]["command"]["cwd"] = cwd;
        admitted(&validator, &doc);
    }
}

#[test]
fn unknown_fields_and_missing_provenance_never_grant_authority() {
    let validator = schema();
    let base = document();
    for index in 0..9 {
        let pointer = format!("/workspace/outputs/{index}");
        let mut doc = base.clone();
        doc.pointer_mut(&pointer).unwrap()["unexpected"] = json!(true);
        rejected(&validator, &doc);
        let error = parse(&doc.to_string()).unwrap_err();
        assert_eq!(error.code, codes::MALFORMED);
        assert!(error.labels.iter().any(|label| {
            label
                .span
                .as_ref()
                .is_some_and(|s| s.line == index as u32 + 2)
        }));
        let mut doc = base.clone();
        doc.pointer_mut(&pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("span");
        rejected(&validator, &doc);
    }
    for (pointer, line) in [
        ("/workspace/outputs/0/source", 12),
        ("/workspace/outputs/0/source/fetch", 12),
        ("/workspace/outputs/0/execution", 2),
        ("/workspace/outputs/0/steps/0/command", 20),
        ("/workspace/outputs/0/steps/0/command/argv/0", 20),
        ("/workspace/outputs/1/producer", 3),
        ("/workspace/outputs/1/layout", 3),
        ("/workspace/outputs/5/trigger", 7),
        ("/workspace/outputs/8/files/0/source", 11),
        ("/workspace/outputs/8/files/0/content", 11),
        ("/workspace/outputs/8/files/0/destination", 11),
    ] {
        let mut doc = base.clone();
        doc.pointer_mut(pointer).unwrap()["unexpected"] = json!(true);
        rejected(&validator, &doc);
        let error = parse(&doc.to_string()).unwrap_err();
        assert!(
            error
                .labels
                .iter()
                .any(|label| label.span.as_ref().is_some_and(|s| s.line == line)),
            "{pointer}: {error:?}"
        );
    }
    for pointer in [
        "/workspace",
        "/workspace/outputs/0/source",
        "/workspace/outputs/0/steps/0/command",
        "/workspace/outputs/8/files/0",
    ] {
        let mut doc = base.clone();
        doc.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("span");
        rejected(&validator, &doc);
    }
}

#[test]
fn malformed_envelopes_and_removed_surfaces_are_rejected() {
    let validator = schema();
    for doc in [
        Value::Null,
        json!([]),
        json!(true),
        json!({}),
        json!({"ir_version":9,"modules":{}}),
        json!({"ir_version":9,"host":{},"workspace":null}),
    ] {
        rejected(&validator, &doc);
    }
    for field in ["ir_version", "host", "workspace"] {
        let mut doc = document();
        doc.as_object_mut().unwrap().remove(field);
        rejected(&validator, &doc);
    }
    for version in [json!(null), json!("9"), json!(-1), json!(4294967296u64)] {
        let mut doc = document();
        doc["ir_version"] = version;
        rejected(&validator, &doc);
    }
    for field in ["modules", "resources", "hosts", "workspce"] {
        let mut doc = document();
        doc[field] = json!({});
        rejected(&validator, &doc);
    }
    for (pointer, value) in [
        ("/host", json!(null)),
        ("/workspace/outputs", json!({})),
        (
            "/workspace/outputs/0/source",
            json!({"fetch":{"kind":"file","path":"p"},"span":span(12)}),
        ),
        (
            "/workspace/outputs/0/source/fetch",
            json!({"kind":"pixi","package":"tool"}),
        ),
        (
            "/workspace/outputs/0/source/fetch",
            json!({"kind":"brew","formula":"tool"}),
        ),
        ("/workspace/outputs/0/kind", json!("widget")),
        ("/workspace/outputs/1/layout", json!("relocatable")),
        ("/workspace/outputs/1/target/os", json!("windows")),
        ("/workspace/outputs/1/target/arch", json!("unknown")),
        ("/workspace/outputs/5/scope", json!("system")),
        ("/workspace/outputs/0/span/line", json!("2")),
        ("/workspace/outputs/0/span/line", json!(-1)),
        ("/workspace/outputs/0/span/line", json!(4294967296u64)),
    ] {
        let mut doc = document();
        *doc.pointer_mut(pointer).unwrap() = value;
        rejected(&validator, &doc);
    }
    for malformed in ["", "{", "null trailing", "{\"ir_version\":9,}"] {
        assert!(parse(malformed).is_err());
    }
}

#[test]
fn authored_value_constraints_are_enforced_by_schema_and_semantics() {
    let validator = schema();
    for (pointer, value, code) in [
        (
            "/workspace/outputs/0/span/file",
            json!(""),
            codes::BAD_WORKSPACE_SPAN,
        ),
        (
            "/workspace/outputs/0/span/line",
            json!(0),
            codes::BAD_WORKSPACE_SPAN,
        ),
        (
            "/workspace/outputs",
            json!([]),
            codes::INVALID_WORKSPACE_VALUE,
        ),
        (
            "/workspace/outputs/0/name",
            json!(""),
            codes::INVALID_WORKSPACE_VALUE,
        ),
        (
            "/workspace/outputs/5/trigger/time",
            json!("25:00"),
            codes::INVALID_WORKSPACE_VALUE,
        ),
        (
            "/workspace/outputs/1/target",
            json!({"os":"linux","arch":"x86_64","abi":"darwin"}),
            codes::INVALID_WORKSPACE_VALUE,
        ),
    ] {
        let mut doc = document();
        *doc.pointer_mut(pointer).unwrap() = value;
        assert!(!validator.is_valid(&doc));
        parse(&doc.to_string()).unwrap();
        let errors = check(&doc.to_string()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.code == code),
            "{pointer}: {errors:?}"
        );
    }
}

#[test]
fn file_content_destination_and_execution_unions_round_trip() {
    let validator = schema();
    for (source, content) in [
        (
            json!({"kind":"repo_file","path":"config"}),
            json!({"kind":"identity"}),
        ),
        (
            json!({"kind":"artifact_file","output":"tool","selector":"share/config"}),
            json!({"kind":"identity"}),
        ),
        (
            json!({"kind":"tree","output":"tool","include":["share"],"exclude":[]}),
            json!({"kind":"identity"}),
        ),
        (
            json!({"kind":"repo_tree","path":"config"}),
            json!({"kind":"identity"}),
        ),
        (
            json!({"kind":"repo_file","path":"config"}),
            json!({"kind":"template_file","variables":{"name":"tool"}}),
        ),
    ] {
        let mut doc = document();
        doc["workspace"]["outputs"][8]["files"][0]["source"] = source;
        doc["workspace"]["outputs"][8]["files"][0]["content"] = content;
        admitted(&validator, &doc);
    }
    for content in [
        json!({"kind":"literal","text":"configuration"}),
        json!({"kind":"template","template":"hello ${name}","variables":{"name":"tool"}}),
    ] {
        for destination in [
            json!({"kind":"tracked_copy","path":"~/.config/tool"}),
            json!({"kind":"symlink","path":"~/.config/tool"}),
            json!({"kind":"managed_block","path":"~/.bashrc","marker":"gripsack"}),
        ] {
            let mut doc = document();
            let file = &mut doc["workspace"]["outputs"][8]["files"][0];
            file.as_object_mut().unwrap().remove("source");
            file["content"] = content.clone();
            file["destination"] = destination;
            admitted(&validator, &doc);
        }
    }
    let mut doc = document();
    doc["workspace"]["outputs"][0]["execution"] = json!({
        "kind":"isolated_linux","worker":"buildkit","platform":{"os":"linux","arch":"x86_64"},
        "toolchain":{"reference":format!("docker.io/library/toolchain@sha256:{}", "a".repeat(64))}
    });
    doc["workspace"]["outputs"][1]["layout"] = json!({"kind":"fixed_prefix","prefix":"/opt/tool"});
    doc["workspace"]["outputs"][2]["prefix"] = json!("/opt/tool");
    doc["workspace"]["outputs"][5]["trigger"] =
        json!({"kind":"weekly","weekday":"mon","time":"03:30"});
    admitted(&validator, &doc);
    for field in ["platform", "toolchain"] {
        let mut missing = doc.clone();
        missing["workspace"]["outputs"][0]["execution"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        rejected(&validator, &missing);
    }
}

#[test]
fn schema_patterns_remain_authoring_constraints_not_guessed_parser_rules() {
    let validator = schema();
    let mut doc = document();
    doc["workspace"]["outputs"][0]["source"]["fetch"] = json!({
        "kind":"tarball","url":"https://example.test/src.tgz","sha256":"not-hex"
    });
    assert!(!validator.is_valid(&doc));
    parse(&doc.to_string()).unwrap();
    check(&doc.to_string()).unwrap();
    doc["workspace"]["outputs"][0]["source"]["fetch"]["sha256"] = json!("a".repeat(64));
    admitted(&validator, &doc);
}
