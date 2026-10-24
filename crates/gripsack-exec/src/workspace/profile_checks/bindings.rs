//! A file subject has no child namespace and is never a working directory.
use crate::{ExecError, workspace::file_failure};
use gripsack_ir::workspace_model::{WorkspaceArg, WorkspaceCommand, WorkspacePath};
use std::path::Path;

pub(super) fn bind(command: &WorkspaceCommand, path: &Path) -> Result<WorkspaceCommand, ExecError> {
    let value = path
        .to_str()
        .ok_or_else(|| file_failure(command.span(), "file-check subject is not UTF-8"))?;
    let mut command = command.clone();
    let span = command.span().clone();
    let replace = |argument: &mut WorkspaceArg| -> Result<(), ExecError> {
        if let WorkspaceArg::Source { selector } = argument {
            if selector != "." {
                return Err(file_failure(
                    &span,
                    "a file-check subject is one file; use sourcePath('.') without a child selector",
                ));
            }
            *argument = WorkspaceArg::Literal {
                value: value.to_owned(),
            };
        }
        Ok(())
    };
    let (env, cwd) = match &mut command {
        WorkspaceCommand::Exec { argv, env, cwd, .. } => {
            for argument in argv {
                replace(argument)?;
            }
            (env, cwd)
        }
        WorkspaceCommand::RunBash {
            interpreter,
            env,
            cwd,
            ..
        } => {
            replace(interpreter)?;
            (env, cwd)
        }
    };
    for argument in env.values_mut() {
        replace(argument)?;
    }
    if matches!(cwd, Some(WorkspacePath::Source { .. })) {
        return Err(file_failure(
            &span,
            "a file-check subject cannot be a working directory",
        ));
    }
    Ok(command)
}
