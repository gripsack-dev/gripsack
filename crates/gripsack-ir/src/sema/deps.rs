//! E101 — module depends on an unknown module.

use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        for dep in &module.depends {
            if !ir.modules.contains_key(&dep.module) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_DEPENDENCY,
                        format!("module {name:?} depends on unknown module {:?}", dep.module),
                    )
                    .with_label(
                        dep.span.clone().or_else(|| module.span.clone()),
                        "dependency declared here",
                    ),
                );
            }
        }
    }
}
