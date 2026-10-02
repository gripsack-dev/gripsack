//! The v6 command value family (0052 §2.2 CommandSpec, v6-design §2.2):
//! commands are *descriptions only* — the enclosing recipe, task, check
//! or hook admits the execution context. v6 adds the `input` argument
//! binding, the optional `package_command` resolved-pin claim, the
//! mandatory run_bash `options` set, and the externally tagged
//! command/action step grammar.

use crate::span::Span;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The shared v6 command value — a description only; the enclosing
/// recipe, task, check or hook admits the execution context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceCommand {
    Exec {
        span: Span,
        argv: Vec<WorkspaceArg>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, WorkspaceArg>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<WorkspacePath>,
    },
    RunBash {
        span: Span,
        /// The pinned tool reference providing bash — a `package_command`
        /// argument, never an ambient host lookup (sema E128).
        interpreter: WorkspaceArg,
        /// The fixed strict option set, in canonical order — a required
        /// field in v6 (v6-design §2.2 [RESPELL]); the exact set is
        /// enforced by sema (E130).
        options: Vec<String>,
        /// Literal text only; dynamic values enter through typed
        /// `env`/`argv` bindings, never interpolation.
        body: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, WorkspaceArg>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<WorkspacePath>,
        /// Dedent mapping: generated line → original source line
        /// (diagnostics map back through it, 0052 §A1-06).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        line_map: Vec<u32>,
    },
}

impl WorkspaceCommand {
    /// The mandatory declaration span.
    pub fn span(&self) -> &Span {
        match self {
            WorkspaceCommand::Exec { span, .. } | WorkspaceCommand::RunBash { span, .. } => span,
        }
    }
    /// Borrow every typed binding without flattening it into command text.
    pub fn arguments(&self) -> impl Iterator<Item = &WorkspaceArg> {
        let (arguments, env) = match self {
            Self::Exec { argv, env, .. } => (argv.as_slice(), env),
            Self::RunBash {
                interpreter, env, ..
            } => (std::slice::from_ref(interpreter), env),
        };
        arguments.iter().chain(env.values())
    }

    pub fn working_directory(&self) -> Option<&WorkspacePath> {
        match self {
            Self::Exec { cwd, .. } | Self::RunBash { cwd, .. } => cwd.as_ref(),
        }
    }
}

/// A command argument or environment value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceArg {
    Literal {
        value: String,
    },
    /// An artifact of another output (`output`, `selector`) — the output
    /// must exist in the catalog (sema E126).
    Artifact {
        output: String,
        selector: String,
    },
    /// Invoke an exported command of a `package` output. Legal in exec
    /// argv and as a run_bash interpreter pin only — never an environment
    /// value (sema E126/E128). `sha256` is an optional resolved-pin
    /// CLAIM: exactly 64 lowercase hex when present (sema E130); claims
    /// never authorize execution (v6-design §2.2).
    PackageCommand {
        package: String,
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    /// A workspace input by name — data, legal in argv and env values,
    /// never as a run_bash interpreter (sema E126/E128).
    Input {
        input: String,
    },
    /// The operation's immutable source: acquired source for a producer, subject for a check.
    Source {
        selector: String,
    },
    /// The enclosing operation's writable staging output, never a live host path.
    Output {
        selector: String,
    },
}

impl WorkspaceArg {
    pub(crate) fn policy_origin(&self) -> gripsack_policy::workspace_command::ArgumentOrigin {
        use gripsack_policy::workspace_command::ArgumentOrigin;
        match self {
            Self::Literal { .. } => ArgumentOrigin::Literal,
            Self::Artifact { .. } => ArgumentOrigin::Artifact,
            Self::PackageCommand { .. } => ArgumentOrigin::PackageCommand,
            Self::Input { .. } => ArgumentOrigin::CapturedInput,
            Self::Source { .. } => ArgumentOrigin::ProductionSource,
            Self::Output { .. } => ArgumentOrigin::StagingOutput,
        }
    }
}

/// A working-directory reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspacePath {
    Literal {
        value: String,
    },
    /// An artifact of another output — must exist in the catalog
    /// (sema E126).
    Artifact {
        output: String,
        selector: String,
    },
    Host {
        path: String,
    },
    Source {
        selector: String,
    },
    Output {
        selector: String,
    },
}

/// One recipe or task step (v6-design §2.2): externally tagged —
/// exactly one of `command` / `action`, closed by the tagged-field
/// pre-pass (E000).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkspaceStep {
    #[serde(rename = "command")]
    Command(WorkspaceCommand),
    #[serde(rename = "action")]
    Action(WorkspaceAction),
}

impl WorkspaceStep {
    /// The mandatory declaration span of the wrapped node.
    pub fn span(&self) -> &Span {
        match self {
            WorkspaceStep::Command(command) => command.span(),
            WorkspaceStep::Action(action) => action.span(),
        }
    }
}

/// A realization request through the common service — never a new
/// producer identity (v6-design §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceAction {
    EnsureArtifact { output: String, span: Span },
}

impl WorkspaceAction {
    /// The mandatory declaration span.
    pub fn span(&self) -> &Span {
        match self {
            WorkspaceAction::EnsureArtifact { span, .. } => span,
        }
    }
}

/// A task's execution context (v6-design §2.3): required, closed to
/// unconfined host with declared mutable paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskContext {
    Host { mutable_paths: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span {
            file: "gripsack.ts".to_string(),
            line: 7,
            col: None,
        }
    }

    #[test]
    fn run_bash_options_are_required() {
        let json = serde_json::json!({
            "kind": "run_bash",
            "span": span(),
            "interpreter": {"kind": "package_command", "package": "bash", "command": "bash"},
            "body": "echo hi"
        });
        assert!(serde_json::from_value::<WorkspaceCommand>(json).is_err());

        let json = serde_json::json!({
            "kind": "run_bash",
            "span": span(),
            "interpreter": {"kind": "package_command", "package": "bash", "command": "bash", "sha256": "a".repeat(64)},
            "options": ["-e", "-u", "-o", "pipefail"],
            "body": "echo hi"
        });
        let command: WorkspaceCommand = serde_json::from_value(json).unwrap();
        match &command {
            WorkspaceCommand::RunBash {
                options,
                interpreter,
                ..
            } => {
                assert_eq!(options, &["-e", "-u", "-o", "pipefail"]);
                assert!(matches!(
                    interpreter,
                    WorkspaceArg::PackageCommand {
                        sha256: Some(_),
                        ..
                    }
                ));
            }
            WorkspaceCommand::Exec { .. } => panic!("expected run_bash"),
        }
    }

    #[test]
    fn task_context_is_closed() {
        let json = serde_json::json!({"kind": "host", "mutable_paths": ["~/src"]});
        let context: TaskContext = serde_json::from_value(json).unwrap();
        assert_eq!(
            context,
            TaskContext::Host {
                mutable_paths: vec!["~/src".to_string()]
            }
        );
        assert!(
            serde_json::from_value::<TaskContext>(
                serde_json::json!({"kind": "isolated_linux", "mutable_paths": []})
            )
            .is_err()
        );
    }
}
