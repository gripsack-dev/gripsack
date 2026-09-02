//! E103/E106/E104/E118 — step shape, ids, refs, and pinnability
//! (0007 §6).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::{Build, Ir};
use crate::step::{BARRIER_STEP_ID, SYNTHESIZED_STEP_IDS, StepAction};

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        let Some(steps) = &module.steps else {
            continue;
        };
        // E103: explicit steps + any declarative field.
        let mixed = module.fetch.is_some()
            || module.build != Build::None
            || !module.install.is_empty()
            || !module.config.is_empty()
            || !module.activate.is_empty();
        if mixed {
            diagnostics.push(
                Diagnostic::error(
                    codes::STEPS_WITH_FIELDS,
                    format!(
                        "module {name:?} mixes `steps` with declarative fields \
                         (fetch/build/install/config/activate)"
                    ),
                )
                .with_label(module.span.clone(), "module declared here")
                .with_help("pick one shape: fields (expanded for you) or explicit steps"),
            );
        }
        // E118: pinnability. The lockfile pins one fetch per module —
        // a module with several fetch steps applies unpinned (update
        // cannot resolve it) with check/plan silent about the loss.
        // One fetch step pins exactly like the declarative style;
        // more is an authoring error, not a silent downgrade.
        let fetch_steps: Vec<&str> = steps
            .iter()
            .filter(|s| matches!(s.action, StepAction::Fetch { .. }))
            .map(|s| s.id.as_str())
            .collect();
        if fetch_steps.len() > 1 {
            diagnostics.push(
                Diagnostic::error(
                    codes::UNPINNABLE_STEPS,
                    format!(
                        "module {name:?} declares {} fetch steps ({}) — the lockfile \
                         pins one fetch per module",
                        fetch_steps.len(),
                        fetch_steps.join(", ")
                    ),
                )
                .with_label(module.span.clone(), "module declared here")
                .with_help(
                    "split into one module per fetch — modules in the same wave fetch in parallel",
                ),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for step in steps {
            // E106: duplicate or reserved ids.
            if !seen.insert(step.id.as_str()) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::DUPLICATE_STEP,
                        format!("module {name:?}: duplicate step id {:?}", step.id),
                    )
                    .with_label(step.span.clone().or_else(|| module.span.clone()), ""),
                );
            }
            if step.id == BARRIER_STEP_ID {
                diagnostics.push(
                    Diagnostic::error(
                        codes::DUPLICATE_STEP,
                        format!(
                            "module {name:?}: step id {BARRIER_STEP_ID:?} is reserved \
                             (the module's barrier step)"
                        ),
                    )
                    .with_label(step.span.clone().or_else(|| module.span.clone()), ""),
                );
            }
            // E104: unknown step refs.
            for need in &step.needs {
                let unknown = match need.split_once(':') {
                    Some((target_module, target_step)) => match ir.modules.get(target_module) {
                        None => true,
                        Some(target) => {
                            !(target_step == BARRIER_STEP_ID
                                || match &target.steps {
                                    Some(target_steps) => {
                                        target_steps.iter().any(|s| s.id == target_step)
                                    }
                                    None => SYNTHESIZED_STEP_IDS.contains(&target_step),
                                })
                        }
                    },
                    // `module:done` is always valid: the barrier exists
                    // for explicit and synthesized modules alike (0007 §2).
                    None => !steps.iter().any(|s| s.id == *need),
                };
                if unknown {
                    diagnostics.push(
                        Diagnostic::error(
                            codes::UNKNOWN_STEP,
                            format!(
                                "module {name:?}: step {:?} needs unknown step {need:?}",
                                step.id
                            ),
                        )
                        .with_label(step.span.clone().or_else(|| module.span.clone()), ""),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Module;
    use crate::step::{Step, StepAction};

    fn fetch_step(id: &str) -> Step {
        Step {
            id: id.into(),
            action: StepAction::Fetch {
                fetch: crate::model::FetchSpec::File {
                    path: "payloads/p.tar.gz".into(),
                },
            },
            needs: vec![],
            resources: vec![],
            phase: None,
            verify: None,
            retries: None,
            span: None,
        }
    }

    fn ir_with_steps(steps: Vec<Step>) -> Ir {
        Ir {
            ir_version: 1,
            host: Default::default(),
            resources: vec![],
            modules: [(
                "m".to_string(),
                Module {
                    steps: Some(steps),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn single_fetch_step_pins_multiple_refused() {
        // one fetch step: pinnable, no diagnostic
        let mut diags = Vec::new();
        check(&ir_with_steps(vec![fetch_step("fetch")]), &mut diags);
        assert!(diags.is_empty());

        // two fetch steps: the lockfile pins one fetch per module
        let mut diags = Vec::new();
        check(
            &ir_with_steps(vec![fetch_step("fetch-a"), fetch_step("fetch-b")]),
            &mut diags,
        );
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_ref() == codes::UNPINNABLE_STEPS)
        );
    }
}
