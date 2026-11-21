//! Raw IR counterparts to typescript/test/workspace_graph.test.ts: no SDK preflight.
use gripsack_ir::{check, codes};
use serde_json::{Value, json};

fn span(line: u32) -> Value {
    json!({"file":"graph.ts","line":line})
}

fn package(name: &str, line: u32) -> Value {
    json!({"kind":"package","name":name,"span":span(line),"commands":{},
        "producer":{"kind":"provider","provider":{"kind":"fetch","fetch":{"kind":"file","path":"tool"},"span":span(line)}},
        "target":{"os":"linux","arch":"x86_64"},"layout":{"kind":"relocatable"}})
}

fn environment(artifact: bool) -> Value {
    let mut env = json!({"kind":"environment","name":"dev","span":span(20),
        "target":{"os":"linux","arch":"x86_64"},"packages":[]});
    if artifact {
        env["env"] = json!({"ROOT":{"kind":"artifact","output":"lib","selector":"."}});
    } else {
        env["packages"] = json!(["lib"]);
    }
    env
}

fn document(outputs: &[Value]) -> String {
    json!({"ir_version":10,"host":{"os":"linux","arch":"x86_64"},"workspace":{
    "span":span(1),"outputs":outputs,"inputs":[
        {"name":"manifest","span":span(2),"origin":{"kind":"repo_file","path":"pixi.toml"}},
        {"name":"lock","span":span(3),"origin":{"kind":"repo_file","path":"pixi.lock"}}
    ]}})
    .to_string()
}

fn rejects(outputs: &[Value], code: &str, lines: &[u32]) {
    let errors = check(&document(outputs)).unwrap_err();
    assert!(
        errors.iter().any(|error| {
            error.code == code
                && error
                    .labels
                    .iter()
                    .filter_map(|label| label.span.as_ref().map(|span| span.line))
                    .eq(lines.iter().copied())
        }),
        "{errors:?}"
    );
}

#[test]
fn target_bearing_edges_compare_provider_abi_and_minimum_os() {
    let mut provider = package("lib", 10);
    provider["target"]["abi"] = json!("gnu");
    provider["target"]["minimum_os"] = json!({"major":6,"minor":1,"patch":2});
    let mut runtime = package("tool", 20);
    runtime["runtime"] = json!(["lib"]);
    for mut consumer in [runtime, environment(false)] {
        consumer["target"] = provider["target"].clone();
        consumer["target"]["minimum_os"] = json!({"major":6,"minor":2});
        check(&document(&[provider.clone(), consumer.clone()])).unwrap();
        consumer["target"]["abi"] = json!("musl");
        rejects(
            &[provider.clone(), consumer.clone()],
            codes::BAD_WORKSPACE_CONTEXT,
            &[20, 10],
        );
        consumer["target"]["abi"] = json!("gnu");
        consumer["target"]["minimum_os"] = json!({"major":6,"minor":1});
        rejects(
            &[provider.clone(), consumer],
            codes::BAD_WORKSPACE_CONTEXT,
            &[20, 10],
        );
    }
    let mut passive = environment(true);
    passive["target"]["abi"] = json!("musl");
    check(&document(&[provider, passive])).unwrap();
}

#[test]
fn producer_layout_and_environment_prefix_matrix() {
    let mut pkg = package("lib", 10);
    let producers = [
        (pkg["producer"]["provider"].clone(), [true, true, false]),
        (
            json!({"kind":"conda_environment","span":span(11),"channels":["conda-forge"],"packages":{"python":"*"}}),
            [false, false, true],
        ),
        (
            json!({"kind":"pixi_lock","span":span(11),"manifest":"manifest","lock":"lock","environment":"default"}),
            [false, false, true],
        ),
        (
            json!({"kind":"package","span":span(11),"manifest":"a".repeat(64)}),
            [true, true, true],
        ),
    ];
    for (provider, allowed) in producers {
        pkg["producer"]["provider"] = provider;
        for (layout, valid) in [
            json!({"kind":"relocatable"}),
            json!({"kind":"fixed_prefix","prefix":"/opt/tools"}),
            json!({"kind":"prefix_materialized"}),
        ]
        .into_iter()
        .zip(allowed)
        {
            pkg["layout"] = layout;
            if !valid {
                rejects(
                    std::slice::from_ref(&pkg),
                    codes::INVALID_WORKSPACE_VALUE,
                    &[10],
                );
                continue;
            }
            for artifact in [false, true] {
                for prefix in [None, Some("/opt/tools"), Some("/other")] {
                    let mut env = environment(artifact);
                    if let Some(path) = prefix {
                        env["prefix"] = json!(path);
                    }
                    let outputs = [pkg.clone(), env];
                    let allowed = match pkg["layout"]["kind"].as_str().unwrap() {
                        "fixed_prefix" => prefix == Some("/opt/tools"),
                        "prefix_materialized" => prefix.is_none(),
                        _ => true,
                    };
                    if allowed {
                        check(&document(&outputs)).unwrap();
                    } else {
                        rejects(&outputs, codes::BAD_WORKSPACE_CONTEXT, &[20]);
                    }
                }
            }
        }
    }
}

#[test]
fn native_environment_has_no_production_bindings() {
    for kind in ["source", "output"] {
        let mut env = environment(true);
        env["env"]["ROOT"] = json!({"kind":kind,"selector":"."});
        rejects(&[env], codes::BAD_WORKSPACE_CONTEXT, &[20]);
    }
}
