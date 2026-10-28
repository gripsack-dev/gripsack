//! Workspace realization, profile deployment, tasks, and retained-generation lifecycle.

pub mod activate;
pub mod apply;
pub mod ctx;
pub mod deploy;
mod embedded_frontend;
pub mod env;
pub mod facts;
pub mod frontend;
pub mod gc;
pub mod managed_blocks;
pub mod ops;
pub mod report;
mod repository;
pub use repository::Repository;
pub mod rollback;
mod source;
pub mod template;
pub mod update;
pub mod util;
pub mod verify_store;
mod workspace;

pub use apply::apply;
pub use ctx::{Ctx, ExecError, Outcome, ProgressCallback};
pub use env::render_env_file;
pub use frontend::{ensure_deno, ensure_ts_frontend};
pub use gc::{GcReport, PathOwner, gc, why_owns};
pub use report::{
    ApplyResult, ReportKind, StepReport, UpdateCheckOutcome, UpdateReport, UpdateStatus,
    UpdateSummary, UpdateSurvey,
};
pub use rollback::rollback_generation;
pub use source::preflight::{
    DeferredLayoutCheck, LayoutEvidence, inspect_known as inspect_known_layouts,
};
pub use update::{UpdateMode, update};
pub use util::LifecycleSession;
pub use workspace::roots::{BuildRecovery, recover_builder_roots};
pub use workspace::{
    AttemptId, BusyPolicy, InvocationId, TaskExecutionBinding, TaskNodeReport, TaskOutputMode,
    TaskPlan, TaskReport, TaskRevision, TaskRunPolicy, TaskStatus, TaskStdin, plan_task,
    read_task_log, read_task_report, run_task,
};
pub use workspace::{BuildOptions, BuildResult, BuiltOutput, build_workspace};
pub use workspace::{ConsumerOutcome, ConsumerRequest, consume, run_package_command};

use gripsack_ir::Ir;

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("{0}")]
    WorkspaceUnavailable(gripsack_ir::Diagnostic),
}

/// Stable workspace output order for plan rendering.
pub fn build_order(ir: &Ir) -> Result<Vec<String>, PlanError> {
    if let Some(diagnostic) =
        ir.workspace_execution_error(gripsack_ir::workspace::WorkspaceOperation::Plan)
    {
        return Err(PlanError::WorkspaceUnavailable(diagnostic));
    }
    let mut names: Vec<String> = ir
        .workspace
        .outputs
        .iter()
        .map(|output| output.name().to_owned())
        .collect();
    names.sort();
    Ok(names)
}

/// Workspace plan rendering groups the admitted catalog into one display wave.
pub fn waves(ir: &Ir) -> Result<Vec<Vec<String>>, PlanError> {
    Ok(vec![build_order(ir)?])
}

/// The ownership-lineage model (plan/0029) — drives the real
/// `plan_copy` decision function over every action sequence.
#[cfg(test)]
mod lineage_model;

#[cfg(all(test, unix))]
mod mode_model;
