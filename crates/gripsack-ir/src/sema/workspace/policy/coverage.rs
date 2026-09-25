//! Compare the decoded output grammar with the projected edge roles.
//! This is an independent source-rooted admission check, not a second
//! graph builder: `graph::collect` still owns names, kinds and spans.
//! Each output's declared references are streamed in emission order
//! and compared pairwise against that output's contiguous run of
//! projected edges — a lost, spuriously added, reordered or same-role
//! substituted reference (foo → bar at equal cardinality) fails before
//! the verified closure can be treated as a complete production graph.
//! Successful admission performs no heap allocation; diagnostics
//! allocate only on rejection. The guard binds role and target
//! identity only: the decode arrow (serde/tagged grammar), the catalog
//! name → kernel index bridge in `index_view`, and edge payloads
//! (artifact selector, package_command name) remain unproved —
//! selectors are still judged on the projection by `refs` (E130).

use super::super::graph::{EdgeRole, Projection};
use super::super::names::Catalog;
use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::{
    Workspace, WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspacePath, WorkspaceProducer,
    WorkspaceSource,
};
use std::collections::BTreeMap;

/// Per-role reference totals kept on the stack, so a successful
/// admission allocates nothing.
#[derive(Clone, Copy, Default)]
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
}

/// One catalog reference decoded from a command argument (exec argv
/// slot or run_bash interpreter pin) — a literal references nothing.
fn arg_target<'a>(arg: &'a WorkspaceArg, role: EdgeRole, emit: &mut impl FnMut(EdgeRole, &'a str)) {
    match arg {
        WorkspaceArg::Literal { .. } => {}
        WorkspaceArg::Artifact { output, .. } => emit(role, output),
        WorkspaceArg::PackageCommand { package, .. } => emit(role, package),
    }
}

/// Artifact references in environment *value* position, in the
/// BTreeMap's key order — the same order `graph::collect` emits them.
/// A package_command value is a context violation (E128), not an edge.
fn env_artifacts<'a>(
    env: &'a BTreeMap<String, WorkspaceArg>,
    role: EdgeRole,
    emit: &mut impl FnMut(EdgeRole, &'a str),
) {
    for arg in env.values() {
        if let WorkspaceArg::Artifact { output, .. } = arg {
            emit(role, output);
        }
    }
}

/// The catalog references of one command body (argv slots,
/// environment values, working directory, run_bash interpreter pin),
/// in the same order `graph::command_edges` emits them.
fn command_refs<'a>(
    command: &'a WorkspaceCommand,
    role: EdgeRole,
    emit: &mut impl FnMut(EdgeRole, &'a str),
) {
    match command {
        WorkspaceCommand::Exec { argv, env, cwd, .. } => {
            for arg in argv {
                arg_target(arg, role, emit);
            }
            env_artifacts(env, role, emit);
            if let Some(WorkspacePath::Artifact { output, .. }) = cwd {
                emit(role, output);
            }
        }
        WorkspaceCommand::RunBash {
            interpreter,
            env,
            cwd,
            ..
        } => {
            // A non-package_command interpreter is an ambient-
            // interpreter violation (E128), not an edge.
            if let WorkspaceArg::PackageCommand { package, .. } = interpreter {
                emit(role, package);
            }
            env_artifacts(env, role, emit);
            if let Some(WorkspacePath::Artifact { output, .. }) = cwd {
                emit(role, output);
            }
        }
    }
}

/// Stream the references `output` declares, in the same emission
/// order as `graph::output_edges`.
fn for_each_declared<'a>(output: &'a WorkspaceOutput, emit: &mut impl FnMut(EdgeRole, &'a str)) {
    match output {
        WorkspaceOutput::Recipe(recipe) => {
            for step in &recipe.steps {
                command_refs(step, EdgeRole::BuildInput, emit);
            }
            for check in &recipe.checks {
                emit(EdgeRole::Validation, check);
            }
        }
        WorkspaceOutput::Package(package) => {
            if let WorkspaceProducer::Recipe { recipe } = &package.producer {
                emit(EdgeRole::Production, recipe);
            }
            for runtime in &package.runtime {
                emit(EdgeRole::Runtime, runtime);
            }
        }
        WorkspaceOutput::Environment(environment) => {
            for package in &environment.packages {
                emit(EdgeRole::Runtime, package);
            }
            env_artifacts(&environment.env, EdgeRole::Runtime, emit);
        }
        WorkspaceOutput::Task(task) => {
            command_refs(&task.run, EdgeRole::Runtime, emit);
            for dep in &task.deps {
                emit(EdgeRole::TaskPrereq, dep);
            }
            if let Some(environment) = &task.environment {
                emit(EdgeRole::Runtime, environment);
            }
            for check in &task.checks {
                emit(EdgeRole::Validation, check);
            }
        }
        WorkspaceOutput::Schedule(schedule) => {
            emit(EdgeRole::Retention, &schedule.task);
        }
        WorkspaceOutput::Check(check) => {
            command_refs(&check.run, EdgeRole::Runtime, emit);
            emit(EdgeRole::Validation, &check.subject);
        }
        WorkspaceOutput::Image(image) => {
            for package in &image.packages {
                emit(EdgeRole::Runtime, package);
            }
        }
        WorkspaceOutput::Profile(profile) => {
            for file in &profile.files {
                if let Some(WorkspaceSource::ArtifactFile { output, .. }) = &file.source {
                    emit(EdgeRole::Runtime, output);
                }
            }
            if let Some(environment) = &profile.environment {
                emit(EdgeRole::Retention, environment);
            }
            for schedule in &profile.schedules {
                emit(EdgeRole::Retention, schedule);
            }
            for hook in &profile.hooks {
                emit(EdgeRole::Retention, hook);
            }
        }
        WorkspaceOutput::Hook(hook) => {
            command_refs(&hook.run, EdgeRole::Runtime, emit);
        }
    }
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

/// Cardinality divergence: the projection lost or invented references.
fn count_mismatch(
    output: &WorkspaceOutput,
    role: EdgeRole,
    required: usize,
    found: usize,
) -> Diagnostic {
    Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` declares {required} {} reference(s), but the graph projects {found}; refusing incomplete admission",
            output.kind(), output.name(), role_name(role),
        ),
    )
    .with_label(Some(output.span().clone()), "referencing output declared here")
}

/// Identity divergence at equal cardinality: the projection names a
/// different target than the source declares at the same position, so
/// the referencing output and both catalog targets are labeled when
/// the names resolve.
fn substituted_target(
    output: &WorkspaceOutput,
    catalog: &Catalog,
    role: EdgeRole,
    declared: &str,
    projected: &str,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` declares {} reference `{declared}`, but the graph projects `{projected}` in its place; refusing a substituted admitted graph",
            output.kind(), output.name(), role_name(role),
        ),
    )
    .with_label(Some(output.span().clone()), "referencing output declared here");
    if let Some(target) = catalog.outputs.get(declared) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "declared target here");
    }
    if let Some(target) = catalog.outputs.get(projected) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "projected target here");
    }
    diagnostic
}

/// Role divergence at a sequence position: the graph classified the
/// reference under a different role than the source declares.
fn reclassified_edge(
    output: &WorkspaceOutput,
    catalog: &Catalog,
    declared: (EdgeRole, &str),
    projected: (EdgeRole, &str),
) -> Diagnostic {
    let (declared_role, declared_target) = declared;
    let (projected_role, projected_target) = projected;
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` declares {} reference `{declared_target}`, but the graph projects a {} edge to `{projected_target}` in its place; refusing a mismatched admitted graph",
            output.kind(), output.name(), role_name(declared_role), role_name(projected_role),
        ),
    )
    .with_label(Some(output.span().clone()), "referencing output declared here");
    if let Some(target) = catalog.outputs.get(declared_target) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "declared target here");
    }
    if let Some(target) = catalog.outputs.get(projected_target) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "projected target here");
    }
    diagnostic
}

/// The first sequence divergence found for one output, recorded while
/// the declared walk finishes counting totals for the diagnostic.
enum Divergence<'a> {
    /// A declared reference has no projected edge left in this
    /// output's run — the graph lost it.
    Missing { role: EdgeRole },
    /// Same role, different target at the same sequence position.
    Substituted {
        role: EdgeRole,
        declared: &'a str,
        projected: &'a str,
    },
    /// Different role at the same sequence position.
    Reclassified {
        declared: (EdgeRole, &'a str),
        projected: (EdgeRole, &'a str),
    },
}

pub(super) fn check(
    workspace: &Workspace,
    projection: &Projection,
    catalog: &Catalog,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut edges = projection.edges.iter().peekable();
    for output in &workspace.outputs {
        let mut declared_counts = RoleCounts::default();
        let mut matched_counts = RoleCounts::default();
        let mut divergence: Option<Divergence> = None;
        for_each_declared(output, &mut |role, target| {
            declared_counts.add(role, 1);
            if divergence.is_some() {
                // Keep counting declared totals for the diagnostic,
                // but consume nothing further.
                return;
            }
            match edges.next_if(|edge| std::ptr::eq(edge.from, output)) {
                Some(edge) if edge.role == role && edge.to == target => {
                    matched_counts.add(role, 1);
                }
                Some(edge) if edge.role == role => {
                    divergence = Some(Divergence::Substituted {
                        role,
                        declared: target,
                        projected: edge.to,
                    });
                }
                Some(edge) => {
                    divergence = Some(Divergence::Reclassified {
                        declared: (role, target),
                        projected: (edge.role, edge.to),
                    });
                }
                None => divergence = Some(Divergence::Missing { role }),
            }
        });
        // Drop the rest of this output's projected run so the next
        // output starts aligned; without an earlier divergence a
        // non-empty rest is a spuriously added edge.
        let mut extras = RoleCounts::default();
        let mut first_extra = None;
        while let Some(edge) = edges.next_if(|edge| std::ptr::eq(edge.from, output)) {
            extras.add(edge.role, 1);
            first_extra = first_extra.or(Some(edge.role));
        }
        let diagnostic = match divergence {
            Some(Divergence::Missing { role }) => Some(count_mismatch(
                output,
                role,
                declared_counts.get(role),
                matched_counts.get(role),
            )),
            Some(Divergence::Substituted {
                role,
                declared,
                projected,
            }) => Some(substituted_target(
                output, catalog, role, declared, projected,
            )),
            Some(Divergence::Reclassified {
                declared,
                projected,
            }) => Some(reclassified_edge(output, catalog, declared, projected)),
            None => first_extra.map(|role| {
                count_mismatch(
                    output,
                    role,
                    declared_counts.get(role),
                    matched_counts.get(role) + extras.get(role),
                )
            }),
        };
        if let Some(diagnostic) = diagnostic {
            diagnostics.push(diagnostic);
        }
    }
    // An edge whose referencing output never claimed it — the
    // projection invented an edge or reordered across outputs.
    if let Some(edge) = edges.next() {
        diagnostics.push(
            Diagnostic::error(
                codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                "a projected edge has no catalog source; refusing an incomplete graph",
            )
            .with_label(Some(edge.from.span().clone()), "edge declared here"),
        );
    }
}
