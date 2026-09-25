//! Source-correspondence and role/closure regressions over the production
//! workspace policy adapter; no proof-only graph implementation.

use super::*;
use crate::sema::workspace::testutil::{PACKAGE, RECIPE, doc};

#[test]
fn production_closure_excludes_runtime_and_preserves_validation() {
    let recipe = RECIPE.replace(
        r#""execution": {"kind": "host", "access": "unconfined"}"#,
        r#""checks": ["smoke"], "execution": {"kind": "host", "access": "unconfined"}"#,
    );
    let runtime = PACKAGE.replace(r#""name": "hello""#, r#""name": "runtime""#);
    let package = PACKAGE.replace(
        r#""commands": {"hello": "bin/hello"}"#,
        r#""commands": {"hello": "bin/hello"}, "runtime": ["runtime"]"#,
    );
    let check = r#"{
            "kind": "check", "name": "smoke", "span": {"file": "grip.ts", "line": 5},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 5},
                    "argv": [{"kind": "literal", "value": "true"}]},
            "subject": "hello"}"#;
    let document = doc(&format!("{recipe},{runtime},{package},{check}"));
    let ir = crate::check(&document).expect("valid build, runtime and validation graph");
    let workspace = ir.workspace.as_ref().unwrap();
    let mut diagnostics = Vec::new();
    let catalog = super::super::names::check(workspace, &mut diagnostics);
    let projection = super::super::graph::collect(workspace);
    let indexed = index_view(&projection, &catalog).expect("resolved typed graph");
    let root = indexed.indices["hello"];
    let closure = build_closure(indexed.names.len(), &indexed.build, root);
    assert_eq!(closure, vec![indexed.indices["build"]]);
    assert!(!closure.contains(&indexed.indices["runtime"]));
    assert!(!closure.contains(&indexed.indices["smoke"]));
    assert!(indexed.validation.contains(&("build", "smoke")));
    assert!(diagnostics.is_empty());
}

#[test]
fn omitted_required_validation_and_producer_fail_closed_with_both_sites() {
    let recipe = RECIPE.replace(
        r#""execution": {"kind": "host", "access": "unconfined"}"#,
        r#""checks": ["smoke"], "execution": {"kind": "host", "access": "unconfined"}"#,
    );
    let check = r#"{
            "kind": "check", "name": "smoke", "span": {"file": "grip.ts", "line": 8},
            "run": {"kind": "exec", "span": {"file": "grip.ts", "line": 8},
                    "argv": [{"kind": "literal", "value": "true"}]},
            "subject": "hello"}"#;
    let ir = crate::parse(&doc(&format!("{recipe},{PACKAGE},{check}"))).unwrap();
    let workspace = ir.workspace.as_ref().unwrap();
    let mut diagnostics = Vec::new();
    let catalog = super::super::names::check(workspace, &mut diagnostics);
    let mut projection = super::super::graph::collect(workspace);
    projection
        .edges
        .retain(|edge| edge.role != EdgeRole::Validation && edge.role != EdgeRole::Production);
    super::check(workspace, &projection, &catalog, &mut diagnostics);
    for (role, source, target) in [("production", 3, 2), ("validation", 2, 8)] {
        let failure = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == codes::REQUIRED_WORKSPACE_EDGE_MISSING
                    && diagnostic.message.contains(role)
                    && diagnostic.labels.len() == 2
            })
            .expect("a mandatory relation must retain both declaration sites");
        let lines: Vec<u32> = failure
            .labels
            .iter()
            .filter_map(|label| label.span.as_ref().map(|span| span.line))
            .collect();
        assert_eq!(lines, vec![source, target]);
    }
}
#[test]
fn omitted_build_input_and_runtime_edges_fail_admission() {
    let recipe = RECIPE.replace(
        r#""execution": {"kind": "host", "access": "unconfined"}"#,
        r#""steps": [{"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                 "argv": [{"kind": "artifact", "output": "src", "selector": "."}]}],
               "execution": {"kind": "host", "access": "unconfined"}"#,
    );
    let package = PACKAGE.replace(
        r#""commands": {"hello": "bin/hello"}"#,
        r#""commands": {"hello": "bin/hello"}, "runtime": ["src"]"#,
    );
    let provider = r#"{
            "kind": "package", "name": "src", "span": {"file": "grip.ts", "line": 11},
            "producer": {"kind": "provider", "provider": {
                "fetch": {"kind": "file", "path": "src.bin"},
                "span": {"file": "grip.ts", "line": 11}}},
            "commands": {"tool": "bin/tool"},
            "target": {"os": "linux", "arch": "x86_64"},
            "layout": {"kind": "relocatable"}}"#;
    let ir = crate::parse(&doc(&format!("{recipe},{package},{provider}"))).unwrap();
    let workspace = ir.workspace.as_ref().unwrap();
    let mut diagnostics = Vec::new();
    let catalog = super::super::names::check(workspace, &mut diagnostics);
    let mut projection = super::super::graph::collect(workspace);
    projection
        .edges
        .retain(|edge| edge.role != EdgeRole::BuildInput && edge.role != EdgeRole::Runtime);
    super::check(workspace, &projection, &catalog, &mut diagnostics);
    let missing: Vec<&str> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == codes::REQUIRED_WORKSPACE_EDGE_MISSING)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert!(
        missing
            .iter()
            .any(|message| message.contains("build input"))
    );
    assert!(missing.iter().any(|message| message.contains("runtime")));
}

#[test]
fn adjacent_local_steps_lower_to_ordering_not_build_edges() {
    let recipe = RECIPE.replace(
        r#""execution": {"kind": "host", "access": "unconfined"}"#,
        r#""steps": [
                    {"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                     "argv": [{"kind": "literal", "value": "first"}]},
                    {"kind": "exec", "span": {"file": "grip.ts", "line": 5},
                     "argv": [{"kind": "literal", "value": "second"}]}],
                "execution": {"kind": "host", "access": "unconfined"}"#,
    );
    let document = doc(&format!("{recipe},{PACKAGE}"));
    crate::check(&document).expect("ordered local commands admit");
    let ir = crate::parse(&document).unwrap();
    let workspace = ir.workspace.as_ref().unwrap();
    let mut diagnostics = Vec::new();
    let catalog = super::super::names::check(workspace, &mut diagnostics);
    let mut projection = super::super::graph::collect(workspace);
    assert_eq!(projection.ordering.len(), 1);
    assert_eq!(
        (projection.ordering[0].before, projection.ordering[0].after),
        (0, 1)
    );
    let indexed = index_view(&projection, &catalog).expect("resolved typed graph");
    let package = indexed.indices["hello"];
    assert_eq!(
        build_closure(indexed.names.len(), &indexed.build, package),
        vec![indexed.indices["build"]]
    );
    projection.ordering.clear();
    super::check(workspace, &projection, &catalog, &mut diagnostics);
    let order = diagnostics
        .iter()
        .find(|d| d.code == codes::REQUIRED_WORKSPACE_EDGE_MISSING)
        .expect("dropped ordering edge blocks admission");
    let lines: Vec<u32> = order
        .labels
        .iter()
        .filter_map(|label| label.span.as_ref().map(|s| s.line))
        .collect();
    assert_eq!(lines, vec![4, 5]);
}

#[test]
fn substituted_graph_references_fail_closed_at_declaration() {
    // Count-only admission misses a same-role target substitution.
    // Role/target-only admission still misses a valid but substituted
    // artifact selector or command exported by the same package.
    let recipe = RECIPE.replace(
        r#""execution": {"kind": "host", "access": "unconfined"}"#,
        r#""steps": [
                    {"kind": "exec", "span": {"file": "grip.ts", "line": 4},
                     "argv": [{"kind": "artifact", "output": "src", "selector": "."}]},
                    {"kind": "exec", "span": {"file": "grip.ts", "line": 5},
                     "argv": [{"kind": "artifact", "output": "alt", "selector": "."}]},
                    {"kind": "exec", "span": {"file": "grip.ts", "line": 6},
                     "argv": [{"kind": "package_command", "package": "src", "command": "tool"}]}],
               "execution": {"kind": "host", "access": "unconfined"}"#,
    );
    let package = PACKAGE.replace(
        r#""commands": {"hello": "bin/hello"}"#,
        r#""commands": {"hello": "bin/hello"}, "runtime": ["rt"]"#,
    );
    let provider = |name: &str, line: u32| {
        format!(
            r#"{{
            "kind": "package", "name": "{name}", "span": {{"file": "grip.ts", "line": {line}}},
            "producer": {{"kind": "provider", "provider": {{
                "fetch": {{"kind": "file", "path": "{name}.bin"}},
                "span": {{"file": "grip.ts", "line": {line}}}}}}},
            "commands": {{"tool": "bin/tool", "other": "bin/other"}},
            "target": {{"os": "linux", "arch": "x86_64"}},
            "layout": {{"kind": "relocatable"}}}}"#
        )
    };
    let (src, alt, rt) = (provider("src", 11), provider("alt", 12), provider("rt", 13));
    let document = doc(&format!("{recipe},{package},{src},{alt},{rt}"));
    // The unmodified graph admits through the full production
    // pipeline — the mutations below are adapter defects, not
    // authoring errors.
    crate::check(&document).expect("distinct build inputs and runtime admit");
    let ir = crate::parse(&document).unwrap();
    let workspace = ir.workspace.as_ref().unwrap();
    let mut diagnostics = Vec::new();
    let catalog = super::super::names::check(workspace, &mut diagnostics);
    let projection = super::super::graph::collect(workspace);
    super::check(workspace, &projection, &catalog, &mut diagnostics);
    assert!(diagnostics.is_empty(), "unmodified projection admits");

    for (role, declared, projected, source, declared_line, projected_line) in [
        (EdgeRole::BuildInput, "src", "alt", 2, 11, 12),
        (EdgeRole::Runtime, "rt", "alt", 3, 13, 12),
    ] {
        let mut projection = super::super::graph::collect(workspace);
        let edge = projection
            .edges
            .iter_mut()
            .find(|edge| edge.role == role && edge.to == declared)
            .expect("the fixture carries the edge to swap");
        edge.to = projected;
        let mut diagnostics = Vec::new();
        super::check(workspace, &projection, &catalog, &mut diagnostics);
        let failure = diagnostics
            .iter()
            .find(|d| {
                d.code == codes::REQUIRED_WORKSPACE_EDGE_MISSING
                    && d.message.contains(&format!("`{declared}`"))
                    && d.message.contains(&format!("`{projected}`"))
            })
            .expect("same-role substitution blocks admission");
        let lines: Vec<u32> = failure
            .labels
            .iter()
            .filter_map(|label| label.span.as_ref().map(|span| span.line))
            .collect();
        assert_eq!(lines, vec![source, declared_line, projected_line]);
    }

    let mut projection = super::super::graph::collect(workspace);
    projection
        .edges
        .iter_mut()
        .find(|edge| {
            edge.role == EdgeRole::BuildInput && edge.to == "src" && edge.selector == Some(".")
        })
        .expect("artifact selector edge")
        .selector = Some("share/other");
    let mut diagnostics = Vec::new();
    super::check(workspace, &projection, &catalog, &mut diagnostics);
    let selector = diagnostics
        .iter()
        .find(|d| {
            d.code == codes::REQUIRED_WORKSPACE_EDGE_MISSING
                && d.message.contains("selector")
                && d.message.contains("share/other")
        })
        .expect("same-target artifact selector substitution blocks admission");
    assert!(
        selector
            .labels
            .iter()
            .any(|label| label.span.as_ref().is_some_and(|span| span.line == 4)),
        "original command declaration is labeled"
    );

    let mut projection = super::super::graph::collect(workspace);
    projection
        .edges
        .iter_mut()
        .find(|edge| {
            edge.role == EdgeRole::BuildInput && edge.to == "src" && edge.command == Some("tool")
        })
        .expect("exported package command edge")
        .command = Some("other");
    let mut diagnostics = Vec::new();
    super::check(workspace, &projection, &catalog, &mut diagnostics);
    let command = diagnostics
        .iter()
        .find(|d| {
            d.code == codes::REQUIRED_WORKSPACE_EDGE_MISSING
                && d.message.contains("package command")
                && d.message.contains("other")
        })
        .expect("same-package exported command substitution blocks admission");
    assert!(
        command
            .labels
            .iter()
            .any(|label| label.span.as_ref().is_some_and(|span| span.line == 6)),
        "original command declaration is labeled"
    );
}

#[test]
fn projected_kind_or_binding_reclassification_fails_closed() {
    let document = doc(&format!("{RECIPE},{PACKAGE}"));
    crate::check(&document).expect("unmodified production edge admits");
    let ir = crate::parse(&document).unwrap();
    let workspace = ir.workspace.as_ref().unwrap();
    let mut diagnostics = Vec::new();
    let catalog = super::super::names::check(workspace, &mut diagnostics);
    assert!(diagnostics.is_empty());

    for classification in ["target kind", "target binding"] {
        let mut projection = super::super::graph::collect(workspace);
        let edge = projection
            .edges
            .iter_mut()
            .find(|edge| edge.role == EdgeRole::Production)
            .expect("package has a recipe producer");
        if classification == "target kind" {
            // The target still matches the broadened kinds, so
            // refs::check alone would accept the wrong adapter rule.
            edge.expected = &["recipe", "package"];
        } else {
            edge.binding = super::super::graph::TargetBinding::None;
        }
        let mut failures = Vec::new();
        super::check(workspace, &projection, &catalog, &mut failures);
        let failure = failures
            .iter()
            .find(|d| {
                d.code == codes::REQUIRED_WORKSPACE_EDGE_MISSING
                    && d.message.contains(classification)
            })
            .expect("projected reference classification must match the decoded source");
        assert_eq!(
            failure
                .labels
                .iter()
                .filter_map(|label| label.span.as_ref().map(|s| s.line))
                .collect::<Vec<_>>(),
            vec![3, 2],
            "both consumer and referenced recipe must be labeled"
        );
    }
}

#[test]
fn catalog_index_corruption_rejects_before_closure() {
    let document = doc(&format!("{RECIPE},{PACKAGE}"));
    crate::check(&document).expect("the unmodified workspace admits");
    let ir = crate::parse(&document).unwrap();
    let workspace = ir.workspace.as_ref().unwrap();
    let projection = super::super::graph::collect(workspace);

    for (mutation, source_line) in [
        ("missing producer", 2),
        ("missing consumer", 3),
        ("substituted producer", 2),
        ("extra catalog entry", 1),
    ] {
        let mut diagnostics = Vec::new();
        let mut catalog = super::super::names::check(workspace, &mut diagnostics);
        assert!(diagnostics.is_empty());
        match mutation {
            "missing producer" => {
                catalog.outputs.remove("build");
            }
            "missing consumer" => {
                catalog.outputs.remove("hello");
            }
            "substituted producer" => {
                catalog.outputs.insert("build", &workspace.outputs[1]);
            }
            "extra catalog entry" => {
                catalog.outputs.insert("unexpected", &workspace.outputs[0]);
            }
            _ => unreachable!(),
        }
        super::check(workspace, &projection, &catalog, &mut diagnostics);
        let rejection = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == codes::REQUIRED_WORKSPACE_EDGE_MISSING)
            .unwrap_or_else(|| panic!("{mutation} must fail closed before graph closure"));
        assert!(
            rejection
                .labels
                .iter()
                .filter_map(|label| label.span.as_ref())
                .any(|span| span.line == source_line),
            "{mutation} must identify the affected source declaration"
        );
    }
}
