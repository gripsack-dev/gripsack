use gripsack_ir::{
    check,
    workspace_model::{WheelSource, lock::LockedWheel},
};
use serde_json::{Value, json};

fn wheel() -> Value {
    json!({"name":"example","version":"1!1.0+local",
        "url":"https://files.example.test/example-1.0-py3-none-any.whl","sha256":"a".repeat(64)})
}

fn document() -> Value {
    let span = json!({"file":"wheels.ts","line":3});
    json!({"ir_version":10,"host":{"os":"linux","arch":"x86_64"},"workspace":{
        "span":span,"outputs":[{"kind":"package","name":"python","span":span,
            "producer":{"kind":"provider","provider":{"kind":"conda_environment","span":span,
                "channels":["conda-forge"],"packages":{"python":"3.12.*"},"wheels":[wheel()]}},
            "commands":{"python":"bin/python"},"target":{"os":"linux","arch":"x86_64"},
            "layout":{"kind":"prefix_materialized"}}]}})
}

#[test]
fn authored_wheels_are_explicit_hash_pinned_binary_requests() {
    let validator = jsonschema::validator_for(
        &serde_json::from_str::<Value>(include_str!("../schema-v10.json")).unwrap(),
    )
    .unwrap();
    let value = document();
    assert!(validator.is_valid(&value));
    check(&value.to_string()).unwrap();
    for field in ["name", "version", "url", "sha256"] {
        let mut missing = document();
        missing["workspace"]["outputs"][0]["producer"]["provider"]["wheels"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(!validator.is_valid(&missing));
        assert!(check(&missing.to_string()).is_err());
    }
    for (field, invalid) in [
        ("name", json!("Example_Name")),
        ("version", json!("../1.0")),
        ("url", json!("http://files.example.test/example.whl")),
        (
            "url",
            json!("https://user:secret@files.example.test/example.whl"),
        ),
        ("url", json!("https://files.example.test/example.tar.gz")),
        (
            "url",
            json!("https://files.example.test/example.whl#sha256=ignored"),
        ),
        (
            "url",
            json!("https://files.example.test/example.whl?source=editable"),
        ),
        ("url", json!("https://files.example.test/ex%61mple.whl")),
        ("sha256", json!("A".repeat(64))),
    ] {
        let mut value = document();
        value["workspace"]["outputs"][0]["producer"]["provider"]["wheels"][0][field] = invalid;
        assert!(!validator.is_valid(&value), "{field}");
        assert!(check(&value.to_string()).is_err(), "{field}");
    }
    let mut duplicate = document();
    duplicate["workspace"]["outputs"][0]["producer"]["provider"]["wheels"] =
        json!([wheel(), wheel()]);
    assert!(check(&duplicate.to_string()).is_err());
}

fn rejected_at_provider(document: &Value, message: &str) {
    let errors = check(&document.to_string()).unwrap_err();
    assert!(
        errors.iter().any(|error| {
            error.message.contains(message)
                && error.labels.iter().any(|label| {
                    label
                        .span
                        .as_ref()
                        .is_some_and(|span| span.file == "sources.ts" && span.line == 3)
                })
        }),
        "{message}: {errors:?}"
    );
}

#[test]
fn provider_validation_uses_decoded_fields_and_preserves_pin_spelling() {
    let mut doc = document();
    let at = "/workspace/outputs/0/producer/provider";
    let url = "https://FILES.example.test/pkg/demo.whl";
    let span = json!({"file":"sources.ts","line":3});
    let platform = json!({"os":"linux","arch":"x86_64","minimum_os":{"major":6,"minor":1}});
    let mut with_patch = platform.clone();
    with_patch["minimum_os"]["patch"] = json!(0);
    let source = doc.pointer_mut(at).unwrap();
    source["span"] = span.clone();
    source["packages"]["python"] = json!(">=3.12,<4");
    source["wheels"][0]["url"] = json!(url);
    source["platforms"] = json!([platform, with_patch]);
    let admitted = serde_json::to_value(check(&doc.to_string()).unwrap()).unwrap();
    assert_eq!(admitted.pointer(at), doc.pointer(at));

    let mut duplicate = doc.clone();
    duplicate.pointer_mut(at).unwrap()["platforms"][1] =
        json!({"minimum_os":{"minor":1,"major":6},"arch":"x86_64","os":"linux"});
    rejected_at_provider(&duplicate, "no duplicates");
    for bad in [
        "HTTPS://host/demo.whl",
        "https:host/demo.whl",
        "https://host\\demo.whl",
        "https://@host/demo.whl",
    ] {
        let mut invalid = doc.clone();
        invalid.pointer_mut(at).unwrap()["wheels"][0]["url"] = json!(bad);
        rejected_at_provider(&invalid, "wheel URL");
    }

    let mut pixi = doc.clone();
    *pixi.pointer_mut(at).unwrap() = json!({"kind":"pixi_lock","manifest":"project manifest",
        "lock":"project-lock","environment":"GPU tools","span":span});
    pixi["workspace"]["inputs"] = json!([
        {"name":"project manifest","span":span,"origin":{"kind":"repo_file","path":"pixi.toml"}},
        {"name":"project-lock","span":span,"origin":{"kind":"repo_file","path":"pixi.lock"}}
    ]);
    check(&pixi.to_string()).unwrap();
    for text in [" padded", "padded\u{a0}", "inter\x7fnal", "inter\u{85}nal"] {
        for (path, message) in [
            ("/channels/0", "channels"),
            ("/packages/python", "MatchSpec"),
        ] {
            let mut invalid = doc.clone();
            *invalid.pointer_mut(at).unwrap().pointer_mut(path).unwrap() = json!(text);
            rejected_at_provider(&invalid, message);
        }
        for field in ["manifest", "lock", "environment"] {
            let mut invalid = pixi.clone();
            invalid.pointer_mut(at).unwrap()[field] = json!(text);
            rejected_at_provider(&invalid, "pixi imports");
        }
    }
}

#[test]
fn frozen_wheels_preserve_complete_metadata_and_reject_unsafe_records() {
    let source: WheelSource = serde_json::from_value(wheel()).unwrap();
    source.validate().unwrap();
    let mut frozen = LockedWheel {
        name: source.name,
        version: source.version,
        filename: "example-1.0-py3-none-any.whl".into(),
        url: source.url,
        sha256: source.sha256,
        size: Some(123),
        requires_python: Some(">=3.10".into()),
        requires_dist: vec!["dependency>=1".into()],
    };
    frozen.validate().unwrap();
    assert_eq!(
        serde_json::from_str::<LockedWheel>(&serde_json::to_string(&frozen).unwrap()).unwrap(),
        frozen
    );
    frozen.requires_dist.push("dependency>=1".into());
    assert!(frozen.validate().is_err());
    frozen.requires_dist.clear();
    frozen.filename = "../outside.whl".into();
    assert!(frozen.validate().is_err());
}
