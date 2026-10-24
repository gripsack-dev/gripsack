//! The retained command wire is closed independently of historical IR readers.
//! All data bindings are frozen to exact bytes; only package executables retain
//! a typed package/command reference for repeat native layout admission.
use super::*;
use gripsack_ir::workspace_model::{WorkspaceArg, WorkspaceCommand, WorkspacePath};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Command {
    program: Program,
    arguments: Vec<Value>,
    environment: BTreeMap<String, Value>,
    directory: String,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Value {
    Text(String),
    Subject(SubjectValue),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubjectValue {
    file_subject: bool,
}

impl Value {
    fn resolve(&self, subject: Option<&str>) -> Result<String, ExecError> {
        match self {
            Self::Text(value) => Ok(value.clone()),
            Self::Subject(SubjectValue { file_subject: true }) => subject
                .map(str::to_owned)
                .ok_or_else(|| invalid("retained file check has no subject")),
            Self::Subject(_) => Err(invalid("invalid retained file-subject binding")),
        }
    }
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
    FileSubject,
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
                    .map(Value::Text)
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
            environment: environment
                .into_iter()
                .map(|(key, value)| (key, Value::Text(value)))
                .collect(),
            directory,
        })
    }

    /// Retain typed positions, never replace matching literal strings.
    pub(super) fn retain_subject(&mut self, original: &WorkspaceCommand) {
        let environment = match original {
            WorkspaceCommand::Exec { argv, env, .. } => {
                if matches!(argv.first(), Some(WorkspaceArg::Source { .. })) {
                    self.program = Program::FileSubject;
                }
                for (value, argument) in self.arguments.iter_mut().zip(argv.iter().skip(1)) {
                    if matches!(argument, WorkspaceArg::Source { .. }) {
                        *value = Value::Subject(SubjectValue { file_subject: true });
                    }
                }
                env
            }
            WorkspaceCommand::RunBash { env, .. } => env,
        };
        for (key, argument) in environment {
            if matches!(argument, WorkspaceArg::Source { .. }) {
                self.environment.insert(
                    key.clone(),
                    Value::Subject(SubjectValue { file_subject: true }),
                );
            }
        }
    }

    pub(super) fn decode(&self, subject: Option<&str>) -> Result<WorkspaceCommand, ExecError> {
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
            Program::FileSubject => WorkspaceArg::Literal {
                value: subject
                    .ok_or_else(|| invalid("retained file program has no subject"))?
                    .to_owned(),
            },
        });
        for value in &self.arguments {
            argv.push(WorkspaceArg::Literal {
                value: value.resolve(subject)?,
            });
        }
        Ok(WorkspaceCommand::Exec {
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
                    Ok((
                        key.clone(),
                        WorkspaceArg::Literal {
                            value: value.resolve(subject)?,
                        },
                    ))
                })
                .collect::<Result<_, ExecError>>()?,
            cwd: Some(WorkspacePath::Host {
                path: self.directory.clone(),
            }),
        })
    }
}

fn invalid(message: &'static str) -> ExecError {
    io::Error::new(io::ErrorKind::InvalidData, message).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_string_bindings_keep_their_original_meaning() {
        let command: Command = serde_json::from_value(serde_json::json!({
            "program":{"kind":"host","value":"/usr/bin/true"},
            "arguments":["", "literal spaces"], "environment":{"VALUE":"exact"},
            "directory":"/tmp"
        }))
        .unwrap();
        let WorkspaceCommand::Exec { argv, env, .. } = command.decode(None).unwrap() else {
            unreachable!()
        };
        assert_eq!(argv[1], WorkspaceArg::Literal { value: "".into() });
        assert_eq!(
            argv[2],
            WorkspaceArg::Literal {
                value: "literal spaces".into()
            }
        );
        assert_eq!(
            env["VALUE"],
            WorkspaceArg::Literal {
                value: "exact".into()
            }
        );
    }

    #[test]
    fn retained_subject_positions_do_not_rebind_equal_literal_paths() {
        let original = WorkspaceCommand::Exec {
            span: Span {
                file: "check.ts".into(),
                line: 1,
                col: None,
            },
            argv: vec![
                WorkspaceArg::Literal {
                    value: "/usr/bin/cmp".into(),
                },
                WorkspaceArg::Source {
                    selector: ".".into(),
                },
                WorkspaceArg::Literal {
                    value: "/temporary/source".into(),
                },
            ],
            env: BTreeMap::from([(
                "INPUT".into(),
                WorkspaceArg::Source {
                    selector: ".".into(),
                },
            )]),
            cwd: None,
        };
        let mut retained = Command::freeze(
            WorkspaceArg::Literal {
                value: "/usr/bin/cmp".into(),
            },
            vec!["/temporary/source".into(), "/temporary/source".into()],
            BTreeMap::from([("INPUT".into(), "/temporary/source".into())]),
            PathBuf::from("/tmp"),
        )
        .unwrap();
        retained.retain_subject(&original);
        let bytes = serde_json::to_vec(&retained).unwrap();
        let restored: Command = serde_json::from_slice(&bytes).unwrap();
        assert!(restored.decode(None).is_err());
        let WorkspaceCommand::Exec { argv, env, .. } =
            restored.decode(Some("/retained/source")).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(
            argv[1],
            WorkspaceArg::Literal {
                value: "/retained/source".into()
            }
        );
        assert_eq!(
            argv[2],
            WorkspaceArg::Literal {
                value: "/temporary/source".into()
            }
        );
        assert_eq!(env["INPUT"], argv[1]);
    }
}
