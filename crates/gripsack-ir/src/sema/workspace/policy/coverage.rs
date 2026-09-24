//! Compare the decoded output grammar with the projected edge roles.
//! This is an independent cardinality admission check, not a second
//! graph builder: `graph::collect` still owns names, kinds and spans.
//! A lost or spuriously added reference in any role fails before the
//! verified closure can be treated as a complete production graph.

use super::super::graph::{EdgeRole, Projection};
use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::{
    Workspace, WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspacePath, WorkspaceProducer,
    WorkspaceSource,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RoleCounts {
    production: usize,
    build_input: usize,
    runtime: usize,
    ordering: usize,
    task_prereq: usize,
    validation: usize,
    retention: usize,
}

impl RoleCounts {
    fn add(&mut self, role: EdgeRole, count: usize) {
        match role {
            EdgeRole::Production => self.production += count,
            EdgeRole::BuildInput => self.build_input += count,
            EdgeRole::Runtime => self.runtime += count,
            EdgeRole::Ordering => self.ordering += count,
            EdgeRole::TaskPrereq => self.task_prereq += count,
            EdgeRole::Validation => self.validation += count,
            EdgeRole::Retention => self.retention += count,
        }
    }

    fn get(self, role: EdgeRole) -> usize {
        match role {
            EdgeRole::Production => self.production,
            EdgeRole::BuildInput => self.build_input,
            EdgeRole::Runtime => self.runtime,
            EdgeRole::Ordering => self.ordering,
            EdgeRole::TaskPrereq => self.task_prereq,
            EdgeRole::Validation => self.validation,
            EdgeRole::Retention => self.retention,
        }
    }

    fn mismatch(self, projected: Self) -> Option<(EdgeRole, usize, usize)> {
        const ROLES: [EdgeRole; 7] = [
            EdgeRole::Production,
            EdgeRole::BuildInput,
            EdgeRole::Runtime,
            EdgeRole::Ordering,
            EdgeRole::TaskPrereq,
            EdgeRole::Validation,
            EdgeRole::Retention,
        ];
        ROLES.into_iter().find_map(|role| {
            let required = self.get(role);
            let actual = projected.get(role);
            (required != actual).then_some((role, required, actual))
        })
    }
}

fn env_artifacts(env: &BTreeMap<String, WorkspaceArg>) -> usize {
    env.values()
        .filter(|arg| matches!(arg, WorkspaceArg::Artifact { .. }))
        .count()
}

fn command_refs(command: &WorkspaceCommand) -> usize {
    let (argv_refs, env, cwd) = match command {
        WorkspaceCommand::Exec { argv, env, cwd, .. } => (
            argv.iter()
                .filter(|arg| !matches!(arg, WorkspaceArg::Literal { .. }))
                .count(),
            env,
            cwd,
        ),
        WorkspaceCommand::RunBash {
            interpreter,
            env,
            cwd,
            ..
        } => (
            usize::from(matches!(interpreter, WorkspaceArg::PackageCommand { .. })),
            env,
            cwd,
        ),
    };
    argv_refs
        + env_artifacts(env)
        + usize::from(matches!(cwd, Some(WorkspacePath::Artifact { .. })))
}

fn declared_roles(output: &WorkspaceOutput) -> RoleCounts {
    let mut counts = RoleCounts::default();
    match output {
        WorkspaceOutput::Recipe(recipe) => {
            counts.add(
                EdgeRole::BuildInput,
                recipe.steps.iter().map(command_refs).sum(),
            );
            counts.add(EdgeRole::Validation, recipe.checks.len());
        }
        WorkspaceOutput::Package(package) => {
            counts.add(
                EdgeRole::Production,
                usize::from(matches!(
                    &package.producer,
                    WorkspaceProducer::Recipe { .. }
                )),
            );
            counts.add(EdgeRole::Runtime, package.runtime.len());
        }
        WorkspaceOutput::Environment(environment) => {
            counts.add(
                EdgeRole::Runtime,
                environment.packages.len() + env_artifacts(&environment.env),
            );
        }
        WorkspaceOutput::Task(task) => {
            counts.add(
                EdgeRole::Runtime,
                command_refs(&task.run) + usize::from(task.environment.is_some()),
            );
            counts.add(EdgeRole::TaskPrereq, task.deps.len());
            counts.add(EdgeRole::Validation, task.checks.len());
        }
        WorkspaceOutput::Schedule(_) => counts.add(EdgeRole::Retention, 1),
        WorkspaceOutput::Check(check) => {
            counts.add(EdgeRole::Runtime, command_refs(&check.run));
            counts.add(EdgeRole::Validation, 1);
        }
        WorkspaceOutput::Image(image) => {
            counts.add(EdgeRole::Runtime, image.packages.len());
        }
        WorkspaceOutput::Profile(profile) => {
            counts.add(
                EdgeRole::Runtime,
                profile
                    .files
                    .iter()
                    .filter(|file| {
                        matches!(
                            file.source.as_ref(),
                            Some(WorkspaceSource::ArtifactFile { .. })
                        )
                    })
                    .count(),
            );
            counts.add(
                EdgeRole::Retention,
                usize::from(profile.environment.is_some())
                    + profile.schedules.len()
                    + profile.hooks.len(),
            );
        }
        WorkspaceOutput::Hook(hook) => {
            counts.add(EdgeRole::Runtime, command_refs(&hook.run));
        }
    }
    counts
}

fn role_name(role: EdgeRole) -> &'static str {
    match role {
        EdgeRole::Production => "production",
        EdgeRole::BuildInput => "build input",
        EdgeRole::Runtime => "runtime",
        EdgeRole::Ordering => "ordering",
        EdgeRole::TaskPrereq => "task prerequisite",
        EdgeRole::Validation => "validation",
        EdgeRole::Retention => "retention",
    }
}

pub(super) fn check(
    workspace: &Workspace,
    projection: &Projection,
    indices: &BTreeMap<&str, usize>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut projected = vec![RoleCounts::default(); indices.len()];
    for edge in &projection.edges {
        let Some(&index) = indices.get(edge.from.name()) else {
            diagnostics.push(
                Diagnostic::error(
                    codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                    "a projected edge has no catalog source; refusing an incomplete graph",
                )
                .with_label(Some(edge.from.span().clone()), "edge declared here"),
            );
            continue;
        };
        let Some(counts) = projected.get_mut(index) else {
            diagnostics.push(
                Diagnostic::error(
                    codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                    "a projected edge has an out-of-range catalog index",
                )
                .with_label(Some(edge.from.span().clone()), "edge declared here"),
            );
            continue;
        };
        counts.add(edge.role, 1);
    }
    for output in &workspace.outputs {
        let Some(&index) = indices.get(output.name()) else {
            diagnostics.push(
                Diagnostic::error(
                    codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                    format!(
                        "workspace {} `{}` has no catalog index",
                        output.kind(),
                        output.name()
                    ),
                )
                .with_label(Some(output.span().clone()), "output declared here"),
            );
            continue;
        };
        let Some(&actual) = projected.get(index) else {
            diagnostics.push(
                Diagnostic::error(
                    codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                    format!(
                        "workspace {} `{}` has an out-of-range catalog index",
                        output.kind(),
                        output.name()
                    ),
                )
                .with_label(Some(output.span().clone()), "output declared here"),
            );
            continue;
        };
        if let Some((role, required, found)) = declared_roles(output).mismatch(actual) {
            diagnostics.push(
                Diagnostic::error(
                    codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                    format!(
                        "workspace {} `{}` declares {required} {} reference(s), but the graph projects {found}; refusing incomplete admission",
                        output.kind(), output.name(), role_name(role),
                    ),
                )
                .with_label(Some(output.span().clone()), "referencing output declared here"),
            );
        }
    }
}
