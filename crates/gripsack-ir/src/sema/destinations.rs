//! E102 — destinations must be absolute or ~/-prefixed.
//! E111 — two modules may not declare the same destination (a race in
//! parallel deploy, and why-owns needs a unique owner).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    let mut owners: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    for (name, module) in &ir.modules {
        for entry in module.install.iter().chain(module.config.iter()) {
            if let Some(other) = owners.insert(entry.to.as_str(), name.as_str())
                && other != name.as_str()
            {
                diagnostics.push(
                    Diagnostic::error(
                        codes::DUPLICATE_DESTINATION,
                        format!("modules {other:?} and {name:?} both deploy to {}", entry.to),
                    )
                    .with_label(module.span.clone(), "and here")
                    .with_help("split the destination, or drop one declaration"),
                );
            }
        }
    }

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
