//! Compare decoded workspace references with projected graph edges.
//! This source-rooted admission check is independent of the graph
//! collector: each output streams its declared references in emission
//! order and compares role, target and selector/exported-command
//! payloads with the contiguous run of projected edges. A lost,
//! extra, reordered or substituted reference fails before the
//! verified closure can be treated as complete.
//! Successful admission performs no heap allocation; diagnostics
//! allocate only on rejection. The decode arrow (serde/tagged grammar),
//! catalog name → kernel index bridge in `index_view`, and the edge's
//! expected-kind/target-binding classifications remain unproved.

use super::super::graph::{
    ARTIFACT_KINDS, CHECK, ENVIRONMENT, EdgeRole, HOOK, PACKAGE, Projection, RECIPE, SCHEDULE,
    SUBJECT_KINDS, TASK, TargetBinding,
};
use super::super::names::Catalog;
use crate::diagnostic::{Diagnostic, codes};
use crate::span::Span;
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

/// A reference read directly from one decoded source declaration.
/// The command/file span belongs to the referring field; list-based
/// references inherit their output's declaration span.
#[derive(Clone, Copy)]
struct DeclaredReference<'a> {
    role: EdgeRole,
    target: &'a str,
    expected: &'static [&'static str],
    binding: TargetBinding,
    selector: Option<&'a str>,
    package_command: Option<&'a str>,
    at: &'a Span,
}

fn named_reference<'a>(
    role: EdgeRole,
    target: &'a str,
    at: &'a Span,
    expected: &'static [&'static str],
    binding: TargetBinding,
) -> DeclaredReference<'a> {
    DeclaredReference {
        role,
        target,
        expected,
        binding,
        selector: None,
        package_command: None,
        at,
    }
}

fn artifact_reference<'a>(
    role: EdgeRole,
    target: &'a str,
    selector: &'a str,
    at: &'a Span,
) -> DeclaredReference<'a> {
    DeclaredReference {
        selector: Some(selector),
        ..named_reference(role, target, at, ARTIFACT_KINDS, TargetBinding::None)
    }
}

fn command_reference<'a>(
    role: EdgeRole,
    package: &'a str,
    command: &'a str,
    at: &'a Span,
) -> DeclaredReference<'a> {
    DeclaredReference {
        package_command: Some(command),
        ..named_reference(role, package, at, PACKAGE, TargetBinding::None)
    }
}

/// One catalog reference decoded from an exec argv slot or a run_bash
/// interpreter pin. A literal carries no catalog reference.
fn arg_reference<'a>(
    arg: &'a WorkspaceArg,
    role: EdgeRole,
    at: &'a Span,
    emit: &mut impl FnMut(DeclaredReference<'a>),
) {
    match arg {
        WorkspaceArg::Literal { .. } => {}
        WorkspaceArg::Artifact { output, selector } => {
            emit(artifact_reference(role, output, selector, at));
        }
        WorkspaceArg::PackageCommand { package, command } => {
            emit(command_reference(role, package, command, at));
        }
    }
}

/// Artifact references in environment *value* position, in the
/// BTreeMap's key order — the same order `graph::collect` emits them.
/// A package_command value is a context violation (E128), not an edge.
fn env_references<'a>(
    env: &'a BTreeMap<String, WorkspaceArg>,
    role: EdgeRole,
    at: &'a Span,
    emit: &mut impl FnMut(DeclaredReference<'a>),
) {
    for arg in env.values() {
        if let WorkspaceArg::Artifact { .. } = arg {
            arg_reference(arg, role, at, emit);
        }
    }
}

/// Catalog references of one command body (argv slots, environment
/// values, working directory, run_bash interpreter pin), in the same
/// order `graph::command_edges` emits them.
fn command_refs<'a>(
    command: &'a WorkspaceCommand,
    role: EdgeRole,
    emit: &mut impl FnMut(DeclaredReference<'a>),
) {
    let at = command.span();
    match command {
        WorkspaceCommand::Exec { argv, env, cwd, .. } => {
            for arg in argv {
                arg_reference(arg, role, at, emit);
            }
            env_references(env, role, at, emit);
            if let Some(WorkspacePath::Artifact { output, selector }) = cwd {
                emit(artifact_reference(role, output, selector, at));
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
            if let WorkspaceArg::PackageCommand { .. } = interpreter {
                arg_reference(interpreter, role, at, emit);
            }
            env_references(env, role, at, emit);
            if let Some(WorkspacePath::Artifact { output, selector }) = cwd {
                emit(artifact_reference(role, output, selector, at));
            }
        }
    }
}

/// Stream the references `output` declares, in the same emission
/// order as `graph::output_edges`.
fn for_each_declared<'a>(
    output: &'a WorkspaceOutput,
    emit: &mut impl FnMut(DeclaredReference<'a>),
) {
    let at = output.span();
    match output {
        WorkspaceOutput::Recipe(recipe) => {
            for step in &recipe.steps {
                command_refs(step, EdgeRole::BuildInput, emit);
            }
            for check in &recipe.checks {
                emit(named_reference(
                    EdgeRole::Validation,
                    check,
                    at,
                    CHECK,
                    TargetBinding::None,
                ));
            }
        }
        WorkspaceOutput::Package(package) => {
            if let WorkspaceProducer::Recipe { recipe } = &package.producer {
                emit(named_reference(
                    EdgeRole::Production,
                    recipe,
                    at,
                    RECIPE,
                    TargetBinding::Producer,
                ));
            }
            for runtime in &package.runtime {
                emit(named_reference(
                    EdgeRole::Runtime,
                    runtime,
                    at,
                    PACKAGE,
                    TargetBinding::None,
                ));
            }
        }
        WorkspaceOutput::Environment(environment) => {
            for package in &environment.packages {
                emit(named_reference(
                    EdgeRole::Runtime,
                    package,
                    at,
                    PACKAGE,
                    TargetBinding::Selection,
                ));
            }
            env_references(&environment.env, EdgeRole::Runtime, at, emit);
        }
        WorkspaceOutput::Task(task) => {
            command_refs(&task.run, EdgeRole::Runtime, emit);
            for dep in &task.deps {
                emit(named_reference(
                    EdgeRole::TaskPrereq,
                    dep,
                    at,
                    TASK,
                    TargetBinding::None,
                ));
            }
            if let Some(environment) = &task.environment {
                emit(named_reference(
                    EdgeRole::Runtime,
                    environment,
                    at,
                    ENVIRONMENT,
                    TargetBinding::None,
                ));
            }
            for check in &task.checks {
                emit(named_reference(
                    EdgeRole::Validation,
                    check,
                    at,
                    CHECK,
                    TargetBinding::None,
                ));
            }
        }
        WorkspaceOutput::Schedule(schedule) => {
            emit(named_reference(
                EdgeRole::Retention,
                &schedule.task,
                at,
                TASK,
                TargetBinding::None,
            ));
        }
        WorkspaceOutput::Check(check) => {
            command_refs(&check.run, EdgeRole::Runtime, emit);
            emit(named_reference(
                EdgeRole::Validation,
                &check.subject,
                at,
                SUBJECT_KINDS,
                TargetBinding::None,
            ));
        }
        WorkspaceOutput::Image(image) => {
            for package in &image.packages {
                emit(named_reference(
                    EdgeRole::Runtime,
                    package,
                    at,
                    PACKAGE,
                    TargetBinding::Selection,
                ));
            }
        }
        WorkspaceOutput::Profile(profile) => {
            for file in &profile.files {
                if let Some(WorkspaceSource::ArtifactFile { output, selector }) = &file.source {
                    emit(artifact_reference(
                        EdgeRole::Runtime,
                        output,
                        selector,
                        &file.span,
                    ));
                }
            }
            if let Some(environment) = &profile.environment {
                emit(named_reference(
                    EdgeRole::Retention,
                    environment,
                    at,
                    ENVIRONMENT,
                    TargetBinding::None,
                ));
            }
            for schedule in &profile.schedules {
                emit(named_reference(
                    EdgeRole::Retention,
                    schedule,
                    at,
                    SCHEDULE,
                    TargetBinding::None,
                ));
            }
            for hook in &profile.hooks {
                emit(named_reference(
                    EdgeRole::Retention,
                    hook,
                    at,
                    HOOK,
                    TargetBinding::None,
                ));
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

/// A same-target edge that silently changes the selected artifact or
/// executable is not the reference the workspace author declared.
fn substituted_payload(
    output: &WorkspaceOutput,
    catalog: &Catalog,
    declared: DeclaredReference<'_>,
    projected_selector: Option<&str>,
    projected_command: Option<&str>,
) -> Diagnostic {
    let (field, expected, projected) = if declared.selector != projected_selector {
        (
            "artifact selector",
            declared.selector.unwrap_or("<none>"),
            projected_selector.unwrap_or("<none>"),
        )
    } else {
        (
            "package command",
            declared.package_command.unwrap_or("<none>"),
            projected_command.unwrap_or("<none>"),
        )
    };
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` declares {field} `{expected}` on `{}`, but the graph projects `{projected}`; refusing a substituted admitted graph",
            output.kind(), output.name(), declared.target,
        ),
    )
    .with_label(Some(declared.at.clone()), "reference declared here");
    if declared.at != output.span() {
        diagnostic = diagnostic.with_label(
            Some(output.span().clone()),
            "referencing output declared here",
        );
    }
    if let Some(target) = catalog.outputs.get(declared.target) {
        diagnostic = diagnostic.with_label(
            Some(target.span().clone()),
            "referenced output declared here",
        );
    }
    diagnostic
}

/// The graph may keep a reference's role, target and payload while
/// broadening its admissible output kinds or dropping its platform
/// binding. Refuse that adapter reclassification before the policy
/// kernel receives the incomplete relation.
fn reclassified_target_rule(
    output: &WorkspaceOutput,
    catalog: &Catalog,
    declared: DeclaredReference<'_>,
    projected_expected: &[&str],
    projected_binding: TargetBinding,
) -> Diagnostic {
    let change = if declared.expected != projected_expected {
        format!(
            "target kind rule {:?} as {:?}",
            declared.expected, projected_expected
        )
    } else {
        format!(
            "target binding rule {:?} as {:?}",
            declared.binding, projected_binding
        )
    };
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` projects {} reference `{}` with a reclassified {change}; refusing incomplete admission",
            output.kind(), output.name(), role_name(declared.role), declared.target,
        ),
    )
    .with_label(Some(declared.at.clone()), "reference declared here");
    if declared.at != output.span() {
        diagnostic = diagnostic.with_label(
            Some(output.span().clone()),
            "referencing output declared here",
        );
    }
    if let Some(target) = catalog.outputs.get(declared.target) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "target declared here");
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
    /// Same role and target, but a different artifact selector or
    /// exported package command.
    Payload {
        declared: DeclaredReference<'a>,
        projected_selector: Option<&'a str>,
        projected_command: Option<&'a str>,
    },
    /// Same declared reference, but the projection changes which
    /// target kinds or platform binding its consumer must admit.
    Classification {
        declared: DeclaredReference<'a>,
        projected_expected: &'static [&'static str],
        projected_binding: TargetBinding,
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
        for_each_declared(output, &mut |reference| {
            declared_counts.add(reference.role, 1);
            if divergence.is_some() {
                // Keep counting declared totals for the diagnostic,
                // but consume nothing further.
                return;
            }
            match edges.next_if(|edge| std::ptr::eq(edge.from, output)) {
                Some(edge)
                    if edge.role == reference.role
                        && edge.to == reference.target
                        && edge.selector == reference.selector
                        && edge.command == reference.package_command
                        && edge.expected == reference.expected
                        && edge.binding == reference.binding =>
                {
                    matched_counts.add(reference.role, 1);
                }
                Some(edge)
                    if edge.role == reference.role
                        && edge.to == reference.target
                        && edge.selector == reference.selector
                        && edge.command == reference.package_command =>
                {
                    divergence = Some(Divergence::Classification {
                        declared: reference,
                        projected_expected: edge.expected,
                        projected_binding: edge.binding,
                    });
                }
                Some(edge) if edge.role == reference.role && edge.to == reference.target => {
                    divergence = Some(Divergence::Payload {
                        declared: reference,
                        projected_selector: edge.selector,
                        projected_command: edge.command,
                    });
                }
                Some(edge) if edge.role == reference.role => {
                    divergence = Some(Divergence::Substituted {
                        role: reference.role,
                        declared: reference.target,
                        projected: edge.to,
                    });
                }
                Some(edge) => {
                    divergence = Some(Divergence::Reclassified {
                        declared: (reference.role, reference.target),
                        projected: (edge.role, edge.to),
                    });
                }
                None => {
                    divergence = Some(Divergence::Missing {
                        role: reference.role,
                    })
                }
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
            Some(Divergence::Classification {
                declared,
                projected_expected,
                projected_binding,
            }) => Some(reclassified_target_rule(
                output,
                catalog,
                declared,
                projected_expected,
                projected_binding,
            )),
            Some(Divergence::Payload {
                declared,
                projected_selector,
                projected_command,
            }) => Some(substituted_payload(
                output,
                catalog,
                declared,
                projected_selector,
                projected_command,
            )),
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
