//! The retained command wire is closed independently of historical IR readers.
//! All data bindings are frozen to exact bytes; only package executables retain
//! a typed package/command reference for repeat native layout admission.
use super::*;
use gripsack_ir::workspace_v6::{WorkspaceArg, WorkspaceCommand, WorkspacePath};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Command {
    program: Program,
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
    directory: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Program {
    Host {
        value: String,
    },
    Package {
        package: String,
        command: String,
        sha256: Option<String>,
    },
}

impl Command {
    pub(super) fn freeze(
        program: WorkspaceArg,
        arguments: Vec<OsString>,
        environment: BTreeMap<String, String>,
        directory: PathBuf,
    ) -> Result<Self, ExecError> {
        let program = match program {
            WorkspaceArg::Literal { value } => Program::Host { value },
            WorkspaceArg::PackageCommand {
                package,
                command,
                sha256,
            } => Program::Package {
                package,
                command,
                sha256,
            },
            _ => return Err(invalid("retained hook program has an unresolved binding")),
        };
        let arguments = arguments
            .into_iter()
            .map(|argument| {
                argument
                    .into_string()
                    .map_err(|_| invalid("retained hook argument is not UTF-8"))
            })
            .collect::<Result<_, _>>()?;
        let directory = directory
            .into_os_string()
            .into_string()
            .map_err(|_| invalid("retained hook directory is not UTF-8"))?;
        Ok(Self {
            program,
            arguments,
            environment,
            directory,
        })
    }

    pub(super) fn decode(&self) -> WorkspaceCommand {
        let mut argv = Vec::with_capacity(self.arguments.len() + 1);
        argv.push(match &self.program {
            Program::Host { value } => WorkspaceArg::Literal {
                value: value.clone(),
            },
            Program::Package {
                package,
                command,
                sha256,
            } => WorkspaceArg::PackageCommand {
                package: package.clone(),
                command: command.clone(),
                sha256: sha256.clone(),
            },
        });
        argv.extend(self.arguments.iter().map(|value| WorkspaceArg::Literal {
            value: value.clone(),
        }));
        WorkspaceCommand::Exec {
            span: Span {
                file: "<retained-workspace-hook>".into(),
                line: 1,
                col: None,
            },
            argv,
            env: self
                .environment
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        WorkspaceArg::Literal {
                            value: value.clone(),
                        },
                    )
                })
                .collect(),
            cwd: Some(WorkspacePath::Host {
                path: self.directory.clone(),
            }),
        }
    }
}

fn invalid(message: &'static str) -> ExecError {
    io::Error::new(io::ErrorKind::InvalidData, message).into()
}
