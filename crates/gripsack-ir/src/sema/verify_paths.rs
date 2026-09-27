//! E109 — verify paths are payload-relative; a destination-shaped path
//! (`/...` or `~/...`) in `binary_runs`/`file_exists` will fail
//! mid-apply against the store. Point at the payload, or use
//! `file_deployed` for destinations (0009 critique finding 1).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::{Ir, Module, Verify};
use crate::step::StepAction;

/// Every authored verification declaration, before normalization. Admission
/// must inspect even mixed/invalid styles rather than letting lowering hide one.
pub(super) fn declarations(module: &Module) -> impl Iterator<Item = &Verify> {
    module
        .verify
        .iter()
        .chain(module.steps.iter().flatten().flat_map(|step| {
            let action = match &step.action {
                StepAction::Verify { verify } => Some(verify),
                _ => None,
            };
            action.into_iter().chain(step.verify.iter())
        }))
}

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for module in ir.modules.values() {
        for verify in declarations(module) {
            let path = match verify {
                Verify::BinaryRuns { path, .. } | Verify::FileExists { path } => Some(path),
                _ => None,
            };
            if let Some(path) = path
                && (path.starts_with('/') || path.starts_with('~'))
            {
                diagnostics.push(
                    Diagnostic::error(
                        codes::VERIFY_PATH_SHAPE,
                        format!(
                            "verify path {path:?} looks like a destination, but verify paths \
                             are payload-relative"
                        ),
                    )
                    .with_label(module.span.clone(), "verify declared here")
                    .with_help(
                        "use a payload-relative path, or verify_deployed() to check the \
                         destination after deploy",
                    ),
                );
            }
        }
    }
}
