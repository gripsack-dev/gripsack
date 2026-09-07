//! Recipe and overlay identity from the same prepared source view (0041).

use crate::ctx::ExecError;
use gripsack_ir::{Module, StepAction, prepared::PreparedModule};
use gripsack_store as store;
use std::path::Path;

pub(super) fn without_spans(module: &Module) -> Module {
    let mut projected = module.clone();
    projected.span = None;
    for entry in projected.install.iter_mut().chain(&mut projected.config) {
        entry.span = None;
    }
    for dep in &mut projected.depends {
        dep.span = None;
    }
    for step in projected.steps.iter_mut().flatten() {
        step.span = None;
        if let StepAction::Install { entries } | StepAction::ConfigDeploy { entries } =
            &mut step.action
        {
            for entry in entries {
                entry.span = None;
            }
        }
    }
    projected
}

pub(crate) fn repo_overlay(
    plan: &PreparedModule,
    repo: &Path,
) -> Result<Option<String>, ExecError> {
    let mut froms = Vec::new();
    for entry in plan.entries() {
        match repo.join(&entry.from).symlink_metadata() {
            Ok(_) => froms.push(entry.from.clone()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    if froms.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        store::canonical_overlay_hash(repo, &froms)?.to_string(),
    ))
}
