//! E109 — verify paths are payload-relative; a destination-shaped path
//! (`/...` or `~/...`) in `binary_runs`/`file_exists` will fail
//! mid-apply against the store. Point at the payload, or use
//! `file_deployed` for destinations (0009 critique finding 1).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::{Ir, Verify};

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    let mut all: Vec<&Verify> = Vec::new();
    for module in ir.modules.values() {
        if let Some(v) = &module.verify {
            all.push(v);
        }
        if let Some(steps) = &module.steps {
            all.extend(steps.iter().filter_map(|s| s.verify.as_ref()));
        }
    }
    for verify in all {
        let path = match verify {
            Verify::BinaryRuns { path, .. } => Some(path),
            Verify::FileExists { path } => Some(path),
            _ => None,
        };
        if let Some(path) = path
            && (path.starts_with('/') || path.starts_with("~/"))
        {
            diagnostics.push(
                Diagnostic::error(
                    codes::VERIFY_PATH_SHAPE,
                    format!(
                        "verify path {path:?} looks like a destination, but verify paths \
                         are payload-relative"
                    ),
                )
                .with_help(
                    "use a payload-relative path, or verify_deployed() to check the \
                     destination after deploy",
                ),
            );
        }
    }
}
