//! Operation-specific admission before opening state or starting any effects.

use crate::{Diagnostic, Ir, codes, workspace_model::WorkspaceOutput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceOperation {
    Apply,
    Plan,
    Update,
    Build,
    Run,
    Shell,
    Task,
}

impl std::fmt::Display for WorkspaceOperation {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(match self {
            Self::Apply => "apply",
            Self::Plan => "plan",
            Self::Update => "update",
            Self::Build => "build",
            Self::Run => "run",
            Self::Shell => "shell",
            Self::Task => "task",
        })
    }
}

pub(crate) fn execution_error(ir: &Ir, operation: WorkspaceOperation) -> Option<Diagnostic> {
    if ir.ir_version != crate::IR_VERSION {
        return Some(crate::parse::version_error(u64::from(ir.ir_version)));
    }
    // These consumers admit their selected closure. Unselected catalog entries
    // must neither acquire effects nor block a supported selected output.
    if !matches!(
        operation,
        WorkspaceOperation::Apply | WorkspaceOperation::Plan
    ) {
        return None;
    }
    let workspace = &ir.workspace;
    if workspace.outputs.iter().any(|output| {
        matches!(output, WorkspaceOutput::Profile(profile) if profile.schedules.is_empty())
    }) {
        return None;
    }
    let output = workspace.outputs.first();
    let span = output.map(WorkspaceOutput::span).unwrap_or(&workspace.span);
    let message = match output {
        Some(WorkspaceOutput::Profile(_) | WorkspaceOutput::Schedule(_)) => {
            "schedule registration is not available"
        }
        _ => {
            "select a profile output; packages and images use build, environments and tasks use run"
        }
    };
    Some(
        Diagnostic::error(
            codes::WORKSPACE_EXEC_UNAVAILABLE,
            format!("{operation}: {message}"),
        )
        .with_label(Some(span.clone()), "declared here"),
    )
}
