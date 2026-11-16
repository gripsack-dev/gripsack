//! Scheduled IR retains concrete selectors, never a dependency on live locks.
use super::{Realization, SelectorBinding, binding_error, expand};
use crate::ExecError;
use gripsack_ir::{
    Ir, Span,
    workspace_model::{
        WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspacePath, WorkspaceStep,
    },
};
use std::borrow::Cow;

pub(in crate::workspace) fn freeze_selectors(
    ir: &mut Ir,
    realization: &Realization<'_>,
) -> Result<(), ExecError> {
    if matches!(realization.selector_binding, SelectorBinding::Concrete) {
        return Ok(());
    }
    for output in &mut ir.workspace.outputs {
        match output {
            WorkspaceOutput::Environment(environment) => {
                for value in environment.env.values_mut() {
                    argument(value, None, realization, &environment.span)?;
                }
            }
            WorkspaceOutput::Task(task) => {
                for step in &mut task.steps {
                    if let WorkspaceStep::Command(value) = step {
                        command(value, None, realization)?;
                    }
                }
            }
            WorkspaceOutput::Check(check) => {
                command(&mut check.run, Some(&check.subject), realization)?;
            }
            WorkspaceOutput::Hook(hook) => command(&mut hook.run, None, realization)?,
            _ => {}
        }
    }
    Ok(())
}

fn command(
    command: &mut WorkspaceCommand,
    subject: Option<&str>,
    realization: &Realization<'_>,
) -> Result<(), ExecError> {
    let (span, args, env, cwd) = match command {
        WorkspaceCommand::Exec {
            span,
            argv,
            env,
            cwd,
        } => (span, argv.as_mut_slice(), env, cwd),
        WorkspaceCommand::RunBash {
            span,
            interpreter,
            env,
            cwd,
            ..
        } => (span, std::slice::from_mut(interpreter), env, cwd),
    };
    for value in args.iter_mut().chain(env.values_mut()) {
        argument(value, subject, realization, span)?;
    }
    match cwd {
        Some(WorkspacePath::Artifact { output, selector }) => {
            freeze(output, selector, realization, span)?;
        }
        Some(WorkspacePath::Source { selector }) => {
            if let Some(subject) = subject {
                freeze(subject, selector, realization, span)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn argument(
    argument: &mut WorkspaceArg,
    subject: Option<&str>,
    realization: &Realization<'_>,
    span: &Span,
) -> Result<(), ExecError> {
    match argument {
        WorkspaceArg::Artifact { output, selector } => freeze(output, selector, realization, span),
        WorkspaceArg::Source { selector } => match subject {
            Some(subject) => freeze(subject, selector, realization, span),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

fn freeze(
    output: &str,
    selector: &mut String,
    realization: &Realization<'_>,
    span: &Span,
) -> Result<(), ExecError> {
    if !selector.contains(['{', '}']) {
        return Ok(());
    }
    let package = realization
        .packages
        .get(output)
        .ok_or_else(|| binding_error(span, format!("artifact {output:?} is not realized")))?;
    if let Cow::Owned(value) = expand(selector, package.source_version.as_deref(), span)? {
        *selector = value;
    }
    Ok(())
}
