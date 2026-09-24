//! v4 workspace schema ↔ parser acceptance parity — the v4 twin of
//! `schema_acceptance.rs` (plan/0042 §E, plan/0052 §2). Every corpus
//! document is admitted or rejected by BOTH `schema/ir/v4.json` and the
//! real core admission (`parse` + the envelope/tagged-field passes).
//! Semantics (reference kinds, cycles, contexts) live in sema and are
//! covered by `sema/workspace.rs` unit tests — absent here by design.
//!
//! The schema is checked with the real `jsonschema` crate through the
//! tracked symlink `crates/gripsack-ir/schema-v4.json →
//! ../../schema/ir/v4.json` (same strategy as the v3 twin).
//!
//! Known, documented asymmetries (excluded from the parity corpus):
//! - ir_version 3 documents: the v4 schema's `const: 4` rejects them;
//!   the parser admits them through the retained v3 reader (version
//!   dispatch, plan/0052 §1). Pinned in `version_dispatch_asymmetry`.
//! - Authored-value constraints (minLength, span minimums, calendar
//!   pattern, outputs minItems, sha256 hex) are stricter than pass 1:
//!   `parse` admits them, the schema rejects them, and sema rejects the
//!   structural ones (spans, empty catalog/names, calendar clocks) with
//!   E129/E130. Pinned in `schema_authored_constraints_execute`.
//! - Explicit `null` for Option fields (`task.environment`, …): serde
//!   admits it (crate-wide convention, see the v3 file's explicit-null
//!   sweep); the schema's strict `type` rejects it.

use gripsack_ir::{check, parse};
use jsonschema::validator_for;
use serde_json::{Value, json};

/// The canonical v4 schema via the tracked symlink.
const SCHEMA_V4: &str = include_str!("../schema-v4.json");

fn compiled() -> jsonschema::Validator {
    validator_for(&serde_json::from_str(SCHEMA_V4).unwrap()).unwrap()
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
    json!({"ir_version": 4, "host": host(), "workspace": workspace})
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
        "execution": "native", "output_kind": "tree",
        "target": {"os": "linux", "arch": "x86_64"}
    })
}

fn package(name: &str, producer: &str) -> Value {
    json!({
        "kind": "package", "name": name, "span": span(),
        "producer": {"kind": "recipe", "recipe": producer},
        "commands": {name: format!("bin/{name}")},
        "target": {"os": "linux", "arch": "x86_64"}, "layout": "relocatable"
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
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": "relocatable"},
            ]))),
        ),
        (
            "v4 legacy modules compatibility envelope",
            json!({"ir_version": 4, "host": host(), "resources": [{"name": "company.lock"}],
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
        assert_eq!(ir.ir_version, 4);
        // Full admission (parse + sema) accepts them too.
        check(&doc.to_string())
            .unwrap_or_else(|ds| panic!("sema rejected valid document {name:?}: {ds:?}"));
    }
}

#[test]
fn rejected_documents_fail_both_sides() {
    let validator = compiled();
    let cases: Vec<(&str, Value)> = vec![
        (
            "unknown top-level field",
            json!({"ir_version": 4, "host": host(), "workspace": workspace(json!([])), "workspce": {}}),
        ),
        (
            "v4 declares both workspace and modules",
            json!({"ir_version": 4, "host": host(), "modules": {}, "workspace": workspace(json!([]))}),
        ),
        (
            "v4 declares neither workspace nor modules",
            json!({"ir_version": 4, "host": host()}),
        ),
        (
            "v4 without core-injected host facts",
            json!({"ir_version": 4, "workspace": workspace(json!([]))}),
        ),
        (
            "ir_version out of range",
            json!({"ir_version": 5, "host": host(), "workspace": workspace(json!([]))}),
        ),
        (
            "hostname selector in host facts",
            json!({"ir_version": 4, "host": {"os": "linux", "arch": "x86_64", "hostname": "laptop"},
                   "workspace": workspace(json!([]))}),
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
            "unknown field on an output",
            envelope(workspace(json!([
                recipe("build"),
                {"kind": "package", "name": "hello", "span": span(), "producer": "build",
                 "commands": {"hello": "bin/hello"}, "target": {"os": "linux", "arch": "x86_64"},
                 "layout": "relocatable", "stage": "deploy"},
            ]))),
        ),
        (
            "missing required output field",
            envelope(workspace(json!([
                recipe("build"),
                {"kind": "package", "name": "hello", "span": span(), "producer": "build",
                 "commands": {"hello": "bin/hello"}, "target": {"os": "linux", "arch": "x86_64"}},
            ]))),
        ),
        (
            "wrong scalar type in span",
            envelope(workspace(json!([
                {"kind": "package", "name": "hello", "span": {"file": "grip.ts", "line": "3"},
                 "producer": "build", "commands": {"hello": "bin/hello"},
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": "relocatable"},
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
                                      "baseUrl": "https://evil.test"},
                            "span": span()},
                 "execution": "native", "output_kind": "tree",
                 "target": {"os": "linux", "arch": "x86_64"}},
            ]))),
        ),
        (
            "unknown producer kind",
            envelope(workspace(json!([
                {"kind": "package", "name": "hello", "span": span(),
                 "producer": {"kind": "magic", "recipe": "build"},
                 "commands": {"hello": "bin/hello"},
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": "relocatable"},
            ]))),
        ),
        (
            "unknown field on the producer union",
            envelope(workspace(json!([
                recipe("build"),
                {"kind": "package", "name": "hello", "span": span(),
                 "producer": {"kind": "recipe", "recipe": "build", "branch": "main"},
                 "commands": {"hello": "bin/hello"},
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": "relocatable"},
            ]))),
        ),
        (
            "unknown field inside the provider fetch spec",
            envelope(workspace(json!([
                {"kind": "package", "name": "ripgrep", "span": span(),
                 "producer": {"kind": "provider", "provider": {
                     "fetch": {"kind": "file", "path": "vendor/rg.tgz", "sha256": "deadbeef"},
                     "span": span()}},
                 "commands": {"rg": "bin/rg"},
                 "target": {"os": "linux", "arch": "x86_64"}, "layout": "relocatable"},
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
            envelope(json!({"span": span(), "name": "w", "outputs": []})),
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

/// A1-06 mirror of the e2e case: a decoded profile output carrying an
/// injected field is rejected by BOTH sides, and the parser's E000
/// labels the profile's own declaration span — runtime rejection with
/// source, never a bare `unknown field`.
#[test]
fn unknown_field_rejection_carries_declaring_span() {
    let validator = compiled();
    let doc = envelope(workspace(json!([
        {"kind": "profile", "name": "p", "span": {"file": "gripsack.ts", "line": 9},
         "unexpected_effect": true},
    ])));
    assert!(
        schema_error(&validator, &doc).is_some(),
        "schema rejects the injected field"
    );
    let diagnostic = parse(&doc.to_string()).unwrap_err();
    assert_eq!(diagnostic.code, gripsack_ir::codes::MALFORMED);
    let span = diagnostic.labels[0]
        .span
        .as_ref()
        .expect("declaring span labeled");
    assert_eq!((span.file.as_str(), span.line), ("gripsack.ts", 9));
}

/// v3 documents ride the retained v3 reader; the v4 schema's `const: 4`
/// rejects them. Version dispatch is deliberate, not a parity gap.
#[test]
fn version_dispatch_asymmetry() {
    let validator = compiled();
    let v3 = json!({"ir_version": 3, "modules": {}});
    assert!(
        schema_error(&validator, &v3).is_some(),
        "v4 schema rejects ir_version 3"
    );
    assert!(
        parse(&v3.to_string()).is_ok(),
        "parser admits ir_version 3 via the retained reader"
    );
    // A v3 envelope carrying a workspace is rejected before serde.
    let v3_workspace = json!({"ir_version": 3, "modules": {},
        "workspace": {"span": {"file": "g", "line": 1}, "outputs": []}});
    assert!(parse(&v3_workspace.to_string()).is_err());
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
            "non-hex sha256 (frontend authoring rule; sema admits)",
            envelope(workspace(json!([
                {"kind": "recipe", "name": "build", "span": span(),
                 "source": {"fetch": {"kind": "tarball", "url": "https://example.test/s.tgz",
                                      "sha256": "not-hex"},
                            "span": span()},
                 "execution": "native", "output_kind": "tree",
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
