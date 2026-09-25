//! Independent source traversal: stream catalog references directly from
//! decoded declarations in their emission order, without graph allocation.

use super::super::super::graph::{
    ARTIFACT_KINDS, CHECK, ENVIRONMENT, EdgeRole, HOOK, PACKAGE, RECIPE, SCHEDULE, SUBJECT_KINDS,
    TASK, TargetBinding,
};
use crate::span::Span;
use crate::workspace::{
    WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspacePath, WorkspaceProducer,
    WorkspaceSource,
};
use std::collections::BTreeMap;

/// A reference read directly from one decoded source declaration.
/// The command/file span belongs to the referring field; list-based
/// references inherit their output's declaration span.
#[derive(Clone, Copy)]
pub(super) struct DeclaredReference<'a> {
    pub(super) role: EdgeRole,
    pub(super) target: &'a str,
    pub(super) expected: &'static [&'static str],
    pub(super) binding: TargetBinding,
    pub(super) selector: Option<&'a str>,
    pub(super) package_command: Option<&'a str>,
    pub(super) at: &'a Span,
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
pub(super) fn for_each_declared<'a>(
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
