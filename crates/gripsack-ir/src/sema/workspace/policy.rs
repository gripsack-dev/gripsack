//! The v5 name/index adapter (also used by v4 read-only validation).
//! Every decoded edge is classified by the production policy, not a
//! proof-only clone. Required publication checks use the validation
//! projection; producers use its build-only closure.
//! Bad or duplicate names are diagnosed before this pass, so all kernel
//! indices are bounded. No effect, lock resolution or executor here.

mod coverage;

use super::graph::{Edge, EdgeRole, Projection};
use super::names::Catalog;
use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::{Workspace, WorkspaceOutput, WorkspaceProducer};
use gripsack_policy::graph::build_closure;
use gripsack_policy::graph::name_index::bind_output_index;
use gripsack_policy::graph::roles::project_graph_roles;
use std::collections::{BTreeMap, BTreeSet};

struct IndexedGraph<'a> {
    names: Vec<&'a str>,
    indices: BTreeMap<&'a str, usize>,
    build: Vec<Vec<usize>>,
    validation: BTreeSet<(&'a str, &'a str)>,
}

/// The name table is produced from these declarations, not an
/// independent source of authority. Require a one-to-one mapping
/// before projecting any edge into the policy kernel's index domain.
fn check_catalog(workspace: &Workspace, catalog: &Catalog) -> Result<(), Diagnostic> {
    for output in &workspace.outputs {
        let indexed = catalog.outputs.get(output.name()).copied();
        if !indexed.is_some_and(|found| std::ptr::eq(found, output)) {
            let mut diagnostic = Diagnostic::error(
                codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                format!(
                    "workspace {} `{}` is missing or substituted in the catalog; refusing an incomplete name/index mapping",
                    output.kind(),
                    output.name()
                ),
            )
            .with_label(Some(output.span().clone()), "source output declared here");
            if let Some(found) = indexed {
                diagnostic = diagnostic
                    .with_label(Some(found.span().clone()), "catalog points here instead");
            }
            return Err(diagnostic);
        }
    }
    if catalog.outputs.len() != workspace.outputs.len() {
        return Err(Diagnostic::error(
            codes::REQUIRED_WORKSPACE_EDGE_MISSING,
            "catalog contains an output absent from the decoded workspace; refusing an incomplete name/index mapping",
        )
        .with_label(Some(workspace.span.clone()), "workspace declared here"));
    }
    Ok(())
}

fn unresolved_projection(
    edge: &Edge,
    target: Option<&WorkspaceOutput>,
    reason: &str,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` projects a {:?} edge to `{}` with {reason}; refusing an incomplete name/index mapping",
            edge.from.kind(), edge.from.name(), edge.role, edge.to,
        ),
    )
    .with_label(Some(edge.at.clone()), "reference declared here");
    if let Some(target) = target {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "target declared here");
    }
    diagnostic
}

/// Only resolved, kind-correct references reach the policy kernel. A
/// catalog name is indexed once; runtime/task/retention/ordering edges
/// never enter `build`, and validation remains a separate required set.
/// Any failed resolution is E131, never a silently omitted edge.
fn index_view<'a>(
    projection: &Projection<'a>,
    catalog: &Catalog<'a>,
) -> Result<IndexedGraph<'a>, Diagnostic> {
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
        if !catalog
            .outputs
            .get(edge.from.name())
            .is_some_and(|source| std::ptr::eq(*source, edge.from))
        {
            return Err(unresolved_projection(edge, None, "an unknown source"));
        }
        let Some(target) = catalog.outputs.get(edge.to) else {
            return Err(unresolved_projection(edge, None, "an unknown target"));
        };
        if !edge.expected.contains(&target.kind()) {
            return Err(unresolved_projection(
                edge,
                Some(target),
                "an incompatible target kind",
            ));
        }
        let (Some(from), Some(to)) = (
            bind_output_index(
                &names,
                edge.from.name(),
                indices.get(edge.from.name()).copied(),
            ),
            bind_output_index(&names, edge.to, indices.get(edge.to).copied()),
        ) else {
            return Err(unresolved_projection(
                edge,
                Some(target),
                "a missing or mismatched source/target index",
            ));
        };
        if decision.build {
            build[from.position()].push(to.position());
        }
        if decision.required_validation {
            validation.insert((edge.from.name(), edge.to));
        }
    }
    Ok(IndexedGraph {
        names,
        indices,
        build,
        validation,
    })
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
    if let Err(diagnostic) = check_catalog(workspace, catalog) {
        diagnostics.push(diagnostic);
        return;
    }
    // Refuse missing, reclassified or misattributed decoded references
    // before any edge enters the indexed policy kernel or build closure.
    check_local_order(workspace, projection, diagnostics);
    coverage::check(workspace, projection, catalog, diagnostics);
    if !diagnostics.is_empty() {
        return;
    }
    let indexed = match index_view(projection, catalog) {
        Ok(indexed) => indexed,
        Err(diagnostic) => {
            diagnostics.push(diagnostic);
            return;
        }
    };
    for output in &workspace.outputs {
        if let WorkspaceOutput::Package(package) = output
            && let WorkspaceProducer::Recipe { recipe } = &package.producer
        {
            let root = bind_output_index(
                &indexed.names,
                package.name.as_str(),
                indexed.indices.get(package.name.as_str()).copied(),
            );
            let producer = bind_output_index(
                &indexed.names,
                recipe.as_str(),
                indexed.indices.get(recipe.as_str()).copied(),
            );
            let target = catalog.outputs.get(recipe.as_str()).copied();
            if let (Some(root), Some(producer), Some(target)) = (root, producer, target) {
                let closure = build_closure(indexed.names.len(), &indexed.build, root.position());
                if !closure.contains(&producer.position()) {
                    diagnostics.push(missing_edge(output, target, "production"));
                }
            } else {
                let mut diagnostic = Diagnostic::error(
                    codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                    format!(
                        "package `{}` has producer `{recipe}` without an exact catalog name/index binding",
                        package.name
                    ),
                )
                .with_label(Some(output.span().clone()), "producer referenced here");
                if let Some(target) = target {
                    diagnostic =
                        diagnostic.with_label(Some(target.span().clone()), "target declared here");
                }
                diagnostics.push(diagnostic);
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
mod tests;
