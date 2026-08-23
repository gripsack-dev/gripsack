//! E107 — step resources must be declared (0007 §4).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;
use crate::step::KNOWN_RESOURCES;

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        let Some(steps) = &module.steps else {
            continue;
        };
        for step in steps {
            for resource in &step.resources {
                let declared = ir.resources.iter().any(|r| r.name == *resource)
                    || KNOWN_RESOURCES.contains(&resource.as_str());
                if !declared {
                    diagnostics.push(
                        Diagnostic::error(
                            codes::UNKNOWN_RESOURCE,
                            format!(
                                "module {name:?}: step {:?} requires undeclared resource \
                                 {resource:?}",
                                step.id
                            ),
                        )
                        .with_label(
                            step.span.clone().or_else(|| module.span.clone()),
                            "required here",
                        )
                        .with_help(format!(
                            "declare it in the IR `resources` section, or use a built-in: {}",
                            KNOWN_RESOURCES.join(", ")
                        )),
                    );
                }
            }
        }
    }
}
