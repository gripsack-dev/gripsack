//! The v5 name/index adapter (also used by v4 read-only validation).
//! Every decoded edge is classified by the production policy, not a
//! proof-only clone. Required publication checks use the validation
//! projection; producers use its build-only closure.
//! Bad or duplicate names are diagnosed before this pass, so all kernel
//! indices are bounded. No effect, lock resolution or executor here.

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
        let missing: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|d| d.code == codes::REQUIRED_WORKSPACE_EDGE_MISSING)
            .collect();
        assert_eq!(missing.len(), 2, "both mandatory relations fail admission");
        for diagnostic in missing {
            assert_eq!(diagnostic.labels.len(), 2);
            assert!(diagnostic.message.contains("required"));
        }
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
}
