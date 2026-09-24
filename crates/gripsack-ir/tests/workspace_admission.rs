//! Public admission surface for current v5 workspaces: named outputs
//! do not touch legacy module expansion; provider-backed packages
//! need no synthetic recipe; serialization preserves versioned shapes.
//! v3 and historical v4 round trips have separate compatibility tests.

use gripsack_ir::{check, parse};
use serde_json::{Value, json};

fn span() -> Value {
    json!({"file": "grip.ts", "line": 3})
}

/// The A1-01 envelope: core-injected os/arch facts (no hostname
/// selector), a recipe and its package.
fn no_hostname_envelope() -> Value {
    json!({
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {"span": span(), "outputs": [
            {"kind": "recipe", "name": "build", "span": span(),
             "source": {"fetch": {"kind": "tarball", "url": "https://example.test/src.tgz"},
                        "span": span()},
             "execution": {"kind": "host", "access": "unconfined"}, "output_kind": "tree",
             "target": {"os": "linux", "arch": "x86_64"}},
            {"kind": "package", "name": "hello", "span": span(),
             "producer": {"kind": "recipe", "recipe": "build"},
             "commands": {"hello": "bin/hello"},
             "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}},
        ]}
    })
}

/// The CLI listing surface: `Ir.workspace` exposes named outputs with
/// their kinds; legacy module expansion stays untouched (empty).
#[test]
fn admitted_workspace_exposes_named_outputs() {
    let ir = check(&no_hostname_envelope().to_string()).unwrap();
    let workspace = ir.workspace.as_ref().expect("workspace admitted");
    assert!(ir.modules.is_empty());
    let names: Vec<&str> = workspace.outputs.iter().map(|o| o.name()).collect();
    let kinds: Vec<&str> = workspace.outputs.iter().map(|o| o.kind()).collect();
    assert_eq!(names, ["build", "hello"]);
    assert_eq!(kinds, ["recipe", "package"]);
    assert_eq!(ir.host.os, "linux");
}

/// A provider-backed package is one admitted output — no synthetic
/// recipe (plan/0052 §2.2 producer = recipe ref OR provider).
#[test]
fn provider_backed_package_lists_as_single_output() {
    let doc = json!({
        "ir_version": 5,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {"span": span(), "outputs": [
            {"kind": "package", "name": "ripgrep", "span": span(),
             "producer": {"kind": "provider", "provider": {
                 "fetch": {"kind": "file", "path": "vendor/ripgrep.tar.gz"},
                 "span": span()}},
             "commands": {"rg": "bin/rg"},
             "target": {"os": "linux", "arch": "x86_64"}, "layout": {"kind": "relocatable"}},
        ]}
    });
    let ir = check(&doc.to_string()).unwrap();
    let workspace = ir.workspace.as_ref().unwrap();
    assert_eq!(workspace.outputs.len(), 1);
    assert_eq!(workspace.outputs[0].name(), "ripgrep");
    assert_eq!(workspace.outputs[0].kind(), "package");
}

/// A v3 empty module map round-trips WITH `modules`; a current v5
/// workspace omits it (the XOR envelope forbids both keys together).
#[test]
fn serialization_preserves_each_version_envelope() {
    let v3 = parse(r#"{"ir_version": 3, "modules": {}}"#).unwrap();
    let v3_json = serde_json::to_value(&v3).unwrap();
    assert!(
        v3_json.get("modules").is_some(),
        "v3 keeps its (empty) module map"
    );
    assert!(v3_json.get("workspace").is_none());

    let v5 = parse(&no_hostname_envelope().to_string()).unwrap();
    let v5_json = serde_json::to_value(&v5).unwrap();
    assert!(
        v5_json.get("modules").is_none(),
        "v5 workspace omits modules"
    );
    assert!(v5_json.get("workspace").is_some());

    // Both shapes re-enter their own admission unchanged.
    parse(&v3_json.to_string()).unwrap();
    check(&v5_json.to_string()).unwrap();
}
