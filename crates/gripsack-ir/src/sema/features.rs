//! E108 — ownership modes the executor doesn't support yet (merge,
//! template). Caught at plan time with spans, never mid-apply (0001
//! §3.7; the typed exports stay — this gate is the contract until
//! implementation lands).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::{Ir, Ownership};

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        for entry in module.install.iter().chain(module.config.iter()) {
            if matches!(entry.mode, Ownership::Merge | Ownership::Template) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNSUPPORTED_MODE,
                        format!(
                            "module {name:?}: ownership mode {:?} is not implemented yet (0.2)",
                            entry.mode
                        ),
                    )
                    .with_label(
                        entry.span.clone().or_else(|| module.span.clone()),
                        "entry declared here",
                    )
                    .with_help("use owned or tracked_copy for now"),
                );
            }
        }
    }
}
