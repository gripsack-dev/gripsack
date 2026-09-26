//! v5 workspace schema ↔ parser acceptance parity (plan/0042 §E,
//! 0052 §2). The real Draft 2020-12 schema and typed core reader
//! agree on this corpus; v3 and historical v4 retain their separate
//! versioned readers and schemas.
//!
//! Semantics (references, cycles, contexts, target floors) run in sema
//! and have source-labeled unit/CLI tests. Structural schema rules
//! such as minLength, spans and calendar patterns may reject values
//! before the typed pass; `schema_authored_constraints_execute`
//! records any parse-versus-sema asymmetry explicitly.

use gripsack_ir::{check, parse};
use jsonschema::validator_for;
use serde_json::{Value, json};

/// The canonical current schema via its tracked symlink.
const SCHEMA_V5: &str = include_str!("../schema-v5.json");

fn compiled() -> jsonschema::Validator {
    validator_for(&serde_json::from_str(SCHEMA_V5).unwrap()).unwrap()
}

/// None = the schema admits the document.
fn schema_error(validator: &jsonschema::Validator, doc: &Value) -> Option<String> {
    validator.validate(doc).err().map(|e| e.to_string())
}

// ------------------------------------------------------------------ corpus

fn span() -> Value {
    json!({"file": "grip.ts", "line": 3})
}

/// Core-injected host facts with NO hostname selector — a project needs
/// no `hosts/<name>.ts` to declare a workspace (plan/0052 §2.1).
fn host() -> Value {
    json!({"os": "linux", "arch": "x86_64"})
}

fn envelope(workspace: Value) -> Value {
    json!({"ir_version": 5, "host": host(), "workspace": workspace})
}

fn workspace(outputs: Value) -> Value {
    json!({"span": span(), "outputs": outputs})
}

fn recipe(name: &str) -> Value {
    json!({
        "kind": "recipe", "name": name, "span": span(),
        "source": {
            "fetch": {"kind": "tarball", "url": "https://example.test/src.tgz",
                      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "span": span()
        },
        "execution": {"kind": "host", "access": "unconfined"}, "output_kind": "tree",
        "target": {"os": "linux", "arch": "x86_64"}
    })
}

fn package(name: &str, producer: &str) -> Value {
    json!({
        "kind": "package", "name": name, "span": span(),
        "producer": {"kind": "recipe", "recipe": producer},
        "commands": {name: format!("bin/{name}")},
        "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}
    })
}

/// The A1-01 positive: a workspace envelope with no hostname anywhere —
/// only core-injected os/arch facts and a package output with its
/// recipe.
fn no_hostname_envelope() -> Value {
    envelope(workspace(json!([
        recipe("build"),
        package("hello", "build"),
    ])))
}

/// Every output kind wired so every typed reference resolves.
fn maximal_workspace() -> Value {
    let mut build = recipe("build");
    build["checks"] = json!(["smoke"]);
    build["steps"] = json!([
        {"kind": "exec", "span": span(),
         "argv": [{"kind": "package_command", "package": "tools", "command": "tools"},
                  {"kind": "literal", "value": "--all"}],
         "env": {"OUT": {"kind": "artifact", "output": "tools", "selector": "."}},
         "cwd": {"kind": "literal", "value": "."}},
        {"kind": "run_bash", "span": span(),
         "interpreter": {"kind": "package_command", "package": "tools", "command": "tools"},
         "body": "echo building\ntrue\n",
         "line_map": [2, 3]}
    ]);
    let mut hello = package("hello", "build");
    hello["runtime"] = json!(["tools"]);
    envelope(workspace(json!([
        build,
        recipe("build-tools"),
        hello,
        package("tools", "build-tools"),
        {"kind": "environment", "name": "dev", "span": span(),
         "packages": ["hello", "tools"],
         "target": {"os": "linux", "arch": "x86_64"},
         "env": {"EDITOR": {"kind": "literal", "value": "hx"},
                 "HELLO_HOME": {"kind": "artifact", "output": "hello", "selector": "."}}},
        {"kind": "task", "name": "format", "span": span(),
         "run": {"kind": "run_bash", "span": span(),
                 "interpreter": {"kind": "package_command", "package": "tools", "command": "tools"},
                 "body": "true"}},
        {"kind": "task", "name": "lint", "span": span(),
         "run": {"kind": "exec", "span": span(),
                 "argv": [{"kind": "package_command", "package": "hello", "command": "hello"},
                          {"kind": "literal", "value": "--lint"}]},
         "deps": ["format"], "environment": "dev", "checks": ["smoke"]},
        {"kind": "check", "name": "smoke", "span": span(),
         "run": {"kind": "exec", "span": span(),
                 "argv": [{"kind": "package_command", "package": "hello", "command": "hello"},
                          {"kind": "literal", "value": "--version"}]},
         "subject": "hello"},
        {"kind": "schedule", "name": "nightly", "span": span(),
         "task": "lint",
         "trigger": {"kind": "weekly", "weekday": "mon", "time": "03:30"},
         "scope": "user"},
        {"kind": "image", "name": "ws", "span": span(),
         "packages": ["hello"], "target": {"os": "linux", "arch": "x86_64"}},
        {"kind": "hook", "name": "refresh", "span": span(),
         "run": {"kind": "exec", "span": span(),
                 "argv": [{"kind": "package_command", "package": "hello", "command": "hello"},
                          {"kind": "literal", "value": "refresh"}]},
         "trigger": "post_activate"},
        {"kind": "profile", "name": "me", "span": span(),
         "files": [
            {"span": span(),
             "source": {"kind": "repo_file", "path": "vim/vimrc"},
             "content": {"kind": "identity"},
             "destination": {"kind": "symlink", "path": "~/.vimrc"}},
            {"span": span(),
             "content": {"kind": "literal", "text": "# gripsack\n"},
             "destination": {"kind": "managed_block", "path": "~/.bashrc", "marker": "gripsack"}},
            {"span": span(),
             "source": {"kind": "artifact_file", "output": "hello", "selector": "share/motd"},
             "content": {"kind": "template", "template": "hello ${user}", "variables": {"user": "t"}},
             "destination": {"kind": "tracked_copy", "path": "~/.motd"}}
         ],
         "environment": "dev", "schedules": ["nightly"], "hooks": ["refresh"]},
    ])))
}

fn valid_corpus() -> Vec<(&'static str, Value)> {
    vec![
        ("no-hostname workspace envelope", no_hostname_envelope()),
        ("maximal workspace, all nine kinds", maximal_workspace()),
        (
            "profile with literal file, no synthetic package",
            envelope(workspace(json!([
                {"kind": "profile", "name": "dotfiles", "span": span(),
                 "files": [{"span": span(),
                            "content": {"kind": "literal", "text": "set number\n"},
                            "destination": {"kind": "tracked_copy", "path": "~/.vimrc"}}]},
            ]))),
        ),
        (
            "lone provider-backed package, no recipe output",
            envelope(workspace(json!([
                {"kind": "package", "name": "ripgrep", "span": span(),
                 "producer": {"kind": "provider", "provider": {
                     "fetch": {"kind": "file", "path": "vendor/ripgrep.tar.gz"},
                     "span": span()}},
                 "commands": {"rg": "bin/rg"},
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}},
            ]))),
        ),
        (
            "cross-target isolated recipe and fixed-prefix environment",
            envelope(workspace(json!([
                {"kind": "recipe", "name": "build", "span": span(),
                 "source": {"fetch": {"kind": "file", "path": "vendor/tool.bin"},
                            "span": span()},
                 "execution": {"kind": "isolated_linux", "worker": "buildkit"},
                 "output_kind": "tree",
                 "target": {"os": "linux", "arch": "x86_64", "abi": "gnu",
                            "minimum_os": {"major": 5, "minor": 15}}},
                {"kind": "package", "name": "tool", "span": span(),
                 "producer": {"kind": "recipe", "recipe": "build"},
                 "commands": {"tool": "bin/tool"},
                 "target": {"os": "linux", "arch": "x86_64", "abi": "gnu",
                            "minimum_os": {"major": 5, "minor": 15}},
                 "layout": {"kind": "fixed_prefix", "prefix": "/opt/tool"}},
                {"kind": "environment", "name": "dev", "span": span(),
                 "packages": ["tool"],
                 "target": {"os": "linux", "arch": "x86_64", "abi": "gnu",
                            "minimum_os": {"major": 6, "minor": 1}},
                 "prefix": "/opt/tool"},
            ]))),
        ),
        (
            "v5 legacy modules compatibility envelope",
            json!({"ir_version": 5, "host": host(), "resources": [{"name": "company.lock"}],
                   "modules": {"m": {"fetch": {"kind": "file", "path": "x.tgz"}}}}),
        ),
        (
            "daily schedule with env-free task",
            envelope(workspace(json!([
                {"kind": "task", "name": "t", "span": span(),
                 "run": {"kind": "exec", "span": span(),
                         "argv": [{"kind": "literal", "value": "true"}]}},
                {"kind": "schedule", "name": "s", "span": span(),
                 "task": "t", "trigger": {"kind": "daily", "time": "00:00"}, "scope": "user"},
            ]))),
        ),
    ]
}

#[test]
fn valid_documents_are_admitted_by_both_sides() {
    let validator = compiled();
    for (name, doc) in valid_corpus() {
        if let Some(error) = schema_error(&validator, &doc) {
            panic!("schema rejected valid document {name:?}: {error}");
        }
        let ir = parse(&doc.to_string())
            .unwrap_or_else(|d| panic!("parser rejected valid document {name:?}: {d}"));
        assert_eq!(ir.ir_version, 5);
        // Full admission (parse + sema) accepts them too.
        check(&doc.to_string())
            .unwrap_or_else(|ds| panic!("sema rejected valid document {name:?}: {ds:?}"));
    }
}

/// A future field must not silently grant authority to any output kind.
/// Start from one graph accepted by both readers; each candidate changes
/// exactly one otherwise-valid declaration and keeps its own source site.
#[test]
fn each_output_kind_rejects_undeclared_authority_with_its_own_span() {
    const KINDS: [&str; 9] = [
        "recipe",
        "package",
        "environment",
        "task",
        "schedule",
        "check",
        "image",
        "profile",
        "hook",
    ];
    let validator = compiled();
    let admitted = maximal_workspace();
    for kind in KINDS {
        let index = admitted["workspace"]["outputs"]
            .as_array()
            .expect("maximal workspace outputs")
            .iter()
            .position(|output| output["kind"].as_str() == Some(kind))
            .expect("every declared kind has an output");
        let mut hostile = admitted.clone();
        let file = format!("fixtures/{kind}.ts");
        let output = hostile["workspace"]["outputs"][index]
            .as_object_mut()
            .expect("typed output");
        output.insert(
            "span".into(),
            json!({"file": file.clone(), "line": index + 1}),
        );
        output.insert(
            "unexpected_effect".into(),
            json!({"kind": "host", "access": "unconfined"}),
        );
        assert!(
            schema_error(&validator, &hostile).is_some(),
            "{kind} schema admitted undeclared authority"
        );
        let error = parse(&hostile.to_string()).expect_err("strict parser must reject extra field");
        assert_eq!(error.code, gripsack_ir::codes::MALFORMED, "{kind}");
        let at = error.labels[0]
            .span
            .as_ref()
            .expect("own declaration labeled");
        assert_eq!(
            (at.file.as_str(), at.line),
            (
                file.as_str(),
                u32::try_from(index + 1).expect("fixture line fits")
            ),
        );
    }
}

#[test]
fn rejected_documents_fail_both_sides() {
    let validator = compiled();
    let cases: Vec<(&str, Value)> = vec![
        (
            "unknown top-level field",
            json!({"ir_version": 5, "host": host(), "workspace": workspace(json!([recipe("build")])), "workspce": {}}),
        ),
        (
            "v5 declares both workspace and modules",
            json!({"ir_version": 5, "host": host(), "modules": {}, "workspace": workspace(json!([recipe("build")]))}),
        ),
        (
            "v5 declares neither workspace nor modules",
            json!({"ir_version": 5, "host": host()}),
        ),
        (
            "v5 without core-injected host facts",
            json!({"ir_version": 5, "workspace": workspace(json!([recipe("build")]))}),
        ),
        (
            "ir_version out of range",
            json!({"ir_version": 6, "host": host(), "workspace": workspace(json!([recipe("build")]))}),
        ),
        (
            "hostname selector in host facts",
            json!({"ir_version": 5, "host": {"os": "linux", "arch": "x86_64", "hostname": "laptop"},
                   "workspace": workspace(json!([recipe("build")]))}),
        ),
        (
            "outputs as an object, not an array",
            envelope(json!({"span": span(), "outputs": {"hello": package("hello", "build")}})),
        ),
        (
            "unknown output kind",
            envelope(workspace(
                json!([{"kind": "widget", "name": "w", "span": span()}]),
            )),
        ),
        (
            "missing required output field",
            envelope(workspace(json!([
                recipe("build"),
                {"kind": "package", "name": "hello", "span": span(),
                 "producer": {"kind": "recipe", "recipe": "build"},
                 "commands": {"hello": "bin/hello"}, "target": {"os": "linux", "arch": "x86_64"}},
            ]))),
        ),
        (
            "wrong scalar type in span",
            envelope(workspace(json!([
                recipe("build"),
                {"kind": "package", "name": "hello", "span": {"file": "grip.ts", "line": "3"},
                 "producer": {"kind": "recipe", "recipe": "build"}, "commands": {"hello": "bin/hello"},
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}},
            ]))),
        ),
        (
            "unknown platform os",
            envelope(workspace(json!([
                {"kind": "image", "name": "ws", "span": span(), "packages": [],
                 "target": {"os": "windows", "arch": "x86_64"}},
            ]))),
        ),
        (
            "unknown field on a command argument",
            envelope(workspace(json!([
                {"kind": "task", "name": "t", "span": span(),
                 "run": {"kind": "exec", "span": span(),
                         "argv": [{"kind": "literal", "value": "x", "shell": true}]}},
            ]))),
        ),
        (
            "unknown field inside the recipe fetch spec",
            envelope(workspace(json!([
                {"kind": "recipe", "name": "build", "span": span(),
                 "source": {"fetch": {"kind": "tarball", "url": "https://example.test/s.tgz",
                                      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                                      "baseUrl": "https://evil.test"},
                            "span": span()},
                 "execution": {"kind": "host", "access": "unconfined"}, "output_kind": "tree",
                 "target": {"os": "linux", "arch": "x86_64"}},
            ]))),
        ),
        (
            "unknown producer kind",
            envelope(workspace(json!([
                {"kind": "package", "name": "hello", "span": span(),
                 "producer": {"kind": "magic", "recipe": "build"},
                 "commands": {"hello": "bin/hello"},
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}},
            ]))),
        ),
        (
            "unknown field on the producer union",
            envelope(workspace(json!([
                recipe("build"),
                {"kind": "package", "name": "hello", "span": span(),
                 "producer": {"kind": "recipe", "recipe": "build", "branch": "main"},
                 "commands": {"hello": "bin/hello"},
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}},
            ]))),
        ),
        (
            "unknown field inside the provider fetch spec",
            envelope(workspace(json!([
                {"kind": "package", "name": "ripgrep", "span": span(),
                 "producer": {"kind": "provider", "provider": {
                     "fetch": {"kind": "file", "path": "vendor/rg.tgz", "network": true},
                     "span": span()}},
                 "commands": {"rg": "bin/rg"},
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}},
            ]))),
        ),
        (
            "schedule scope beyond user",
            envelope(workspace(json!([
                {"kind": "task", "name": "t", "span": span(),
                 "run": {"kind": "exec", "span": span(),
                         "argv": [{"kind": "literal", "value": "true"}]}},
                {"kind": "schedule", "name": "s", "span": span(), "task": "t",
                 "trigger": {"kind": "daily", "time": "09:00"}, "scope": "system"},
            ]))),
        ),
        (
            "future workspace field (name) stays rejected",
            envelope(json!({"span": span(), "name": "w", "outputs": [recipe("build")]})),
        ),
        (
            "template content without variables",
            envelope(workspace(json!([
                {"kind": "profile", "name": "p", "span": span(),
                 "files": [{"span": span(),
                            "content": {"kind": "template", "template": "x"},
                            "destination": {"kind": "symlink", "path": "~/.x"}}]},
            ]))),
        ),
    ];
    for (name, doc) in cases {
        if schema_error(&validator, &doc).is_none() {
            panic!("schema admitted invalid document {name:?}: {doc}");
        }
        if parse(&doc.to_string()).is_ok() {
            panic!("parser admitted invalid document {name:?}: {doc}");
        }
    }
}

/// v3/v4 documents ride retained readers, not the current v5 schema.
/// Version dispatch is deliberate, not a schema/parser parity gap.
#[test]
fn version_dispatch_asymmetry() {
    let validator = compiled();
    let v3 = json!({"ir_version": 3, "modules": {}});
    assert!(
        schema_error(&validator, &v3).is_some(),
        "v5 schema rejects ir_version 3"
    );
    assert!(
        parse(&v3.to_string()).is_ok(),
        "parser admits ir_version 3 via the retained reader"
    );
    // A v3 envelope carrying a workspace is rejected before serde.
    let v3_workspace = json!({"ir_version": 3, "modules": {},
        "workspace": {"span": {"file": "g", "line": 1}, "outputs": []}});
    assert!(parse(&v3_workspace.to_string()).is_err());
    let historical = json!({
        "ir_version": 4, "host": host(), "workspace": {
            "span": span(), "outputs": [
                {"kind": "package", "name": "tool", "span": span(),
                 "producer": {"kind": "provider", "provider": {
                     "fetch": {"kind": "file", "path": "tool.bin"}, "span": span()}},
                 "commands": {"tool": "bin/tool"},
                 "target": {"os": "linux", "arch": "x86_64"},
                 "layout": "relocatable"}
            ]}
    });
    assert!(schema_error(&validator, &historical).is_some());
    assert!(
        parse(&historical.to_string())
            .unwrap()
            .workspace_v4
            .is_some()
    );
}

/// Authored-value constraints the schema pins tighter than pass 1:
/// `parse` admits, the schema rejects, and sema closes the structural
/// ones (spans, empty catalog/names, calendar clocks) with E129/E130.
#[test]
fn schema_authored_constraints_execute() {
    use gripsack_ir::codes;
    let validator = compiled();
    // (name, document, sema code that rejects it — None = sema also admits)
    let cases: Vec<(&str, Value, Option<&str>)> = vec![
        (
            "span with empty file",
            envelope(workspace(json!([
                {"kind": "check", "name": "c", "span": {"file": "", "line": 1},
                 "run": {"kind": "exec", "span": span(),
                         "argv": [{"kind": "literal", "value": "true"}]},
                 "subject": "s"},
            ]))),
            Some(codes::BAD_WORKSPACE_SPAN),
        ),
        (
            "span with line 0",
            envelope(workspace(json!([
                {"kind": "check", "name": "c", "span": {"file": "grip.ts", "line": 0},
                 "run": {"kind": "exec", "span": span(),
                         "argv": [{"kind": "literal", "value": "true"}]},
                 "subject": "s"},
            ]))),
            Some(codes::BAD_WORKSPACE_SPAN),
        ),
        (
            "empty outputs catalog",
            envelope(workspace(json!([]))),
            Some(codes::INVALID_WORKSPACE_VALUE),
        ),
        (
            "empty output name",
            envelope(workspace(json!([
                {"kind": "check", "name": "", "span": span(),
                 "run": {"kind": "exec", "span": span(),
                         "argv": [{"kind": "literal", "value": "true"}]},
                 "subject": "s"},
            ]))),
            Some(codes::INVALID_WORKSPACE_VALUE),
        ),
        (
            "calendar clock outside HH:MM",
            envelope(workspace(json!([
                {"kind": "task", "name": "t", "span": span(),
                 "run": {"kind": "exec", "span": span(),
                         "argv": [{"kind": "literal", "value": "true"}]}},
                {"kind": "schedule", "name": "s", "span": span(), "task": "t",
                 "trigger": {"kind": "daily", "time": "25:00"}, "scope": "user"},
            ]))),
            Some(codes::INVALID_WORKSPACE_VALUE),
        ),
        (
            "Linux target with Darwin ABI",
            envelope(workspace(json!([
                {"kind": "image", "name": "bad-target", "span": span(), "packages": [],
                 "target": {"os": "linux", "arch": "x86_64", "abi": "darwin"}},
            ]))),
            Some(codes::INVALID_WORKSPACE_VALUE),
        ),
        (
            "non-hex sha256 (frontend authoring rule; sema admits)",
            envelope(workspace(json!([
                {"kind": "recipe", "name": "build", "span": span(),
                 "source": {"fetch": {"kind": "tarball", "url": "https://example.test/s.tgz",
                                      "sha256": "not-hex"},
                            "span": span()},
                 "execution": {"kind": "host", "access": "unconfined"}, "output_kind": "tree",
                 "target": {"os": "linux", "arch": "x86_64"}},
            ]))),
            None,
        ),
    ];
    for (name, doc, sema_code) in cases {
        assert!(
            schema_error(&validator, &doc).is_some(),
            "schema must reject {name:?}"
        );
        let text = doc.to_string();
        assert!(
            parse(&text).is_ok(),
            "pass 1 admits {name:?} (constraint is sema/authoring-level)"
        );
        match sema_code {
            Some(code) => {
                let diagnostics = check(&text).unwrap_err();
                assert!(
                    diagnostics.iter().any(|d| d.code == code),
                    "sema must reject {name:?} with {code}, got {diagnostics:?}"
                );
            }
            None => assert!(check(&text).is_ok(), "sema admits {name:?} by design"),
        }
    }
}
