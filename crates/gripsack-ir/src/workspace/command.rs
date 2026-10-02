//! The shared command value family (0052 §2.2 CommandSpec): commands
//! are *descriptions only* — the enclosing recipe, task, check or hook
//! admits the execution context. Arguments and working-directory
//! references are the typed bindings dynamic values enter through;
//! `run_bash` bodies stay literal text.

use crate::span::Span;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The shared command value (0052 §2.2 CommandSpec) — a description
/// only; the enclosing recipe, task, check or hook admits the execution
/// context.
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
    /// value (sema E126/E128).
    PackageCommand {
        package: String,
        command: String,
    },
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
}
