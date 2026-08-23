//! E102 — destinations must be absolute or ~/-prefixed.

use crate::diagnostic::{codes, Diagnostic};
use crate::model::Ir;

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        for entry in module.install.iter().chain(module.config.iter()) {
            if !(entry.to.starts_with('/') || entry.to.starts_with("~/")) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::BAD_DESTINATION,
                        format!(
                            "module {name:?}: destination {:?} must be absolute or start with ~/",
                            entry.to
                        ),
                    )
                    .with_label(
                        entry.span.clone().or_else(|| module.span.clone()),
                        "entry declared here",
                    ),
                );
            }
        }
    }
}
