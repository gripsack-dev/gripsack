//! The v5 name/index adapter (also used by v4 read-only validation).
//! Every decoded edge is classified by the production policy, not a
//! proof-only clone. Required publication checks use the validation
//! projection; producers use its build-only closure.
//! Bad or duplicate names are diagnosed before this pass, so all kernel
//! indices are bounded. No effect, lock resolution or executor here.

mod coverage;

use super::graph::{EdgeRole, Projection};
use super::names::Catalog;
use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::{Workspace, WorkspaceOutput, WorkspaceProducer};
use gripsack_policy::graph::build_closure;
use gripsack_policy::graph::roles::project_graph_roles;
use std::collections::{BTreeMap, BTreeSet};

struct IndexedGraph<'a> {
    names: Vec<&'a str>,
    indices: BTreeMap<&'a str, usize>,
    build: Vec<Vec<usize>>,
    validation: BTreeSet<(&'a str, &'a str)>,
}

/// Only resolved, kind-correct references reach the policy kernel. A
/// catalog name is indexed once; runtime/task/retention/ordering edges
/// never enter `build`, and validation remains a separate required set.
fn index_view<'a>(projection: &Projection<'a>, catalog: &Catalog<'a>) -> IndexedGraph<'a> {
    let names: Vec<&str> = catalog.outputs.keys().copied().collect();
    let indices: BTreeMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (*name, i))
        .collect();
    let roles: Vec<EdgeRole> = projection.edges.iter().map(|edge| edge.role).collect();
    let decisions = project_graph_roles(&roles);
    let mut build = vec![Vec::new(); names.len()];
    let mut validation = BTreeSet::new();
    for (edge, decision) in projection.edges.iter().zip(decisions.iter()) {
        let Some(target) = catalog.outputs.get(edge.to) else {
            continue;
        };
        if !edge.expected.contains(&target.kind()) {
            continue;
        }
        let (Some(&from), Some(&to)) = (indices.get(edge.from.name()), indices.get(edge.to)) else {
            continue;
        };
        if decision.build {
            build[from].push(to);
        }
        if decision.required_validation {
            validation.insert((edge.from.name(), edge.to));
        }
    }
    IndexedGraph {
        names,
        indices,
        build,
        validation,
    }
}

fn missing_edge(from: &WorkspaceOutput, to: &WorkspaceOutput, role: &str) -> Diagnostic {
    Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` lost required {role} edge to `{}`; refusing an incomplete admitted graph",
            from.kind(), from.name(), to.name(),
        ),
    )
    .with_label(Some(from.span().clone()), "required edge declared here")
    .with_label(Some(to.span().clone()), "target declared here")
}

/// Ordered local commands are lowered to adjacent step edges without
/// turning them into catalog dependencies or independent cache nodes.
/// If an adapter ever omits/reorders an edge, admission fails before
/// any executor can observe a shortened command sequence.
fn check_local_order(
    workspace: &Workspace,
    projection: &Projection,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut observed = projection.ordering.iter();
    for output in &workspace.outputs {
        let WorkspaceOutput::Recipe(recipe) = output else {
            continue;
        };
        for (before, pair) in recipe.steps.windows(2).enumerate() {
            let expected = observed.next();
            if !matches!(expected, Some(edge)
                if edge.recipe == recipe.name
                    && edge.before == before
                    && edge.after == before + 1
                    && edge.role == EdgeRole::Ordering
                    && edge.at == pair[1].span())
            {
                diagnostics.push(
                    Diagnostic::error(
                        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                        format!(
                            "recipe `{}` lost ordering edge from step {before} to step {}; refusing an incomplete local command sequence",
                            recipe.name, before + 1
                        ),
                    )
                    .with_label(Some(pair[0].span().clone()), "preceding command")
                    .with_label(Some(pair[1].span().clone()), "next command"),
                );
                return;
            }
        }
    }
    if let Some(extra) = observed.next() {
        diagnostics.push(
            Diagnostic::error(
                codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                format!("unexpected ordering edge in recipe `{}`", extra.recipe),
            )
            .with_label(Some(extra.at.clone()), "extra local edge"),
        );
    }
}

pub(super) fn check(
    workspace: &Workspace,
    projection: &Projection,
    catalog: &Catalog,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let indexed = index_view(projection, catalog);
    check_local_order(workspace, projection, diagnostics);
    coverage::check(workspace, projection, catalog, diagnostics);
    for output in &workspace.outputs {
        if let WorkspaceOutput::Package(package) = output
            && let WorkspaceProducer::Recipe { recipe } = &package.producer
            && let (Some(&root), Some(&producer), Some(target)) = (
                indexed.indices.get(package.name.as_str()),
                indexed.indices.get(recipe.as_str()),
                catalog.outputs.get(recipe.as_str()),
            )
        {
            let closure = build_closure(indexed.names.len(), &indexed.build, root);
            if !closure.contains(&producer) {
                diagnostics.push(missing_edge(output, target, "production"));
            }
        }
        let required: &[String] = match output {
            WorkspaceOutput::Recipe(recipe) => &recipe.checks,
            WorkspaceOutput::Task(task) => &task.checks,
            _ => &[],
        };
        for name in required {
            if let Some(target) = catalog.outputs.get(name.as_str())
                && !indexed.validation.contains(&(output.name(), name.as_str()))
            {
                diagnostics.push(missing_edge(output, target, "validation"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
        let indexed = index_view(&projection, &catalog);
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
        let indexed = index_view(&projection, &catalog);
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
                edge.role == EdgeRole::BuildInput
                    && edge.to == "src"
                    && edge.command == Some("tool")
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
}
