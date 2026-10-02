//! Resource declarations and acquisition boundaries (0007 §4, 0048 §3.3).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;
use crate::step::{KNOWN_RESOURCES, StepAction};

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        let Some(steps) = &module.steps else {
            continue;
        };
        for step in steps {
            if !step.resources.is_empty()
                && matches!(
                    step.action,
                    StepAction::Verify { .. } | StepAction::Intent { .. }
                )
            {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNSUPPORTED_STEP_RESOURCES,
                        format!(
                            "module {name:?}: step {:?} declares resources on a verify or intent action that does not acquire them",
                            step.id
                        ),
                    )
                    .with_label(
                        step.span.clone().or_else(|| module.span.clone()),
                        "resource protection cannot be enforced here",
                    )
                    .with_help(
                        "move resource-controlled work to a produce step; verification and activation intents do not acquire step resources",
                    ),
                );
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resources_require_an_action_that_actually_acquires_them() {
        for (action, rejected) in [
            (
                json!({"kind": "verify", "verify": {"kind": "shell", "script": "true"}}),
                true,
            ),
            (
                json!({"kind": "intent", "action": {"kind": "custom_shell", "script": "true"}}),
                true,
            ),
            (json!({"kind": "custom_shell", "script": "true"}), false),
        ] {
            let ir: Ir = serde_json::from_value(json!({
                "ir_version": crate::IR_VERSION,
                "modules": {"example": {"steps": [{
                    "id": "guarded",
                    "action": action,
                    "resources": ["network"],
                    "span": {"file": "example.ts", "line": 7}
                }]}}
            }))
            .unwrap();
            let mut diagnostics = Vec::new();
            check(&ir, &mut diagnostics);
            assert_eq!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == codes::UNSUPPORTED_STEP_RESOURCES),
                rejected
            );
            if rejected {
                assert_eq!(diagnostics[0].labels[0].span.as_ref().unwrap().line, 7);
            }
        }
    }
}
