//! Workspace source updates publish only the current workspace lock.
use crate::ctx::{Ctx, ExecError};
use crate::report::UpdateSurvey;
use gripsack_ir::Ir;

pub use gripsack_policy::update_survey::UpdateMode;

pub fn update(ir: &Ir, ctx: &Ctx, mode: UpdateMode) -> Result<UpdateSurvey, ExecError> {
    if let Some(diagnostic) =
        ir.workspace_execution_error(gripsack_ir::workspace::WorkspaceOperation::Update)
    {
        return Err(ExecError::Gate(diagnostic));
    }
    crate::workspace::update::update(ir, ctx, mode)
}
