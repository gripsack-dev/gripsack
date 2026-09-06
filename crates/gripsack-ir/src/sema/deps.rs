//! E101 — module depends on an unknown module.
//! E123 — build-closure dependencies must have distinct environment identifiers.

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
        let mut exports = std::collections::BTreeMap::new();
        for dependency in crate::dependencies::build_closure_names(&ir.modules, name) {
            let var = crate::dependencies::build_dep_var(dependency);
            if let Some(other) = exports.insert(var.clone(), dependency) {
                diagnostics.push(Diagnostic::error(
                    codes::BUILD_DEP_ENV_COLLISION,
                    format!("module {name:?}: build dependencies {other:?} and {dependency:?} both export {var}"),
                ).with_label(module.span.clone(), "consumer declares this build closure")
                 .with_help("rename one dependency so its GRIP_DEP_* identifier is distinct"));
            }
        }
    }
}
