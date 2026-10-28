//! Offline workspace preview uses the same ownership planners as apply.

use super::*;
use crate::ctx::ExecError;

pub fn preview_ops(
    ir: &gripsack_ir::Ir,
    repository: &crate::Repository,
    prev: Option<&store::Generation>,
    adopting: &std::collections::BTreeSet<String>,
    limits: gripsack_fetch::FetchLimits,
    selected: &[String],
) -> Result<Vec<Op>, ExecError> {
    if let Some(diagnostic) =
        ir.workspace_execution_error(gripsack_ir::workspace::WorkspaceOperation::Plan)
    {
        return Err(ExecError::Gate(diagnostic));
    }
    let home = store::gripsack_home();
    let native =
        crate::workspace::NativeProfiles::prepare_preview(ir, repository, &home, selected, limits)?;
    let migration = native.migration(&home, prev.map(|generation| generation.number), prev)?;
    native.preview(&home, prev, adopting, migration.as_ref())
}
