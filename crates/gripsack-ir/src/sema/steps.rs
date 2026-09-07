//! E103/E106/E104/E118 — step shape, ids, refs, and pinnability
//! (0007 §6).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::{Build, Ir};
use crate::step::{BARRIER_STEP_ID, Phase, StepAction};

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    let mut prepared = std::collections::BTreeMap::new();
    for (name, module) in &ir.modules {
        match crate::prepared::PreparedModule::new(module) {
            Ok(plan) => {
                prepared.insert(name.as_str(), plan);
            }
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
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
            if step.id.is_empty() || step.id.contains(':') {
                diagnostics.push(
                    Diagnostic::error(
                        codes::DUPLICATE_STEP,
                        format!(
                            "invalid step id {:?}: expected a nonempty id without ':'",
                            step.id
                        ),
                    )
                    .with_label(
                        step.span.clone().or_else(|| module.span.clone()),
                        "step declared here",
                    ),
                );
            }
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
                    Some((target_module, target_step)) => {
                        if target_module == name {
                            diagnostics.push(Diagnostic::error(codes::UNKNOWN_STEP,
                                format!("use sibling id {target_step:?}, not self-qualified {need:?}"))
                                .with_label(step.span.clone().or_else(|| module.span.clone()), "step declared here"));
                            false
                        } else {
                            match prepared.get(target_module).and_then(
                                |plan: &crate::prepared::PreparedModule| {
                                    plan.target_phase(target_step)
                                },
                            ) {
                                None => true,
                                Some(Phase::Activate) => {
                                    diagnostics.push(Diagnostic::error(codes::STEP_PHASE_ORDER,
                                        format!("step {:?} cannot wait for activation {need:?}; activation runs after the flip", step.id))
                                        .with_label(step.span.clone().or_else(|| module.span.clone()), "step declared here"));
                                    false
                                }
                                Some(_) => false,
                            }
                        }
                    }
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
        check_phase_order(name, module, steps, diagnostics);
    }
    check_module_cycles(ir, diagnostics);
}

/// Execution phase ranks (0007 §5): produce (fetch/build/custom) runs
/// before deploy (install/config), then verify, then activate. A step
/// may only `need` a step that runs no later than itself.
fn phase_rank(phase: Option<crate::step::Phase>) -> u8 {
    use crate::step::Phase;
    match phase {
        Some(Phase::Fetch) => 1,
        // custom and unset phase land in the produce sweep
        Some(Phase::Build) | Some(Phase::Custom) | None => 1,
        Some(Phase::Install) | Some(Phase::Config) => 2,
        Some(Phase::Verify) => 3,
        Some(Phase::Activate) => 4,
    }
}

// E121: cross-phase needs must respect the execution order — a
// produce-phase step needing a deploy-phase step could never see its
// output. Post-deploy effects belong in activate hooks.
fn check_phase_order(
    name: &str,
    module: &crate::model::Module,
    steps: &[crate::step::Step],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let rank_of = |id: &str| {
        steps
            .iter()
            .find(|s| s.id == id)
            .map(|s| phase_rank(Some(s.action.execution_phase())))
    };
    for step in steps {
        for need in &step.needs {
            if need.contains(':') {
                continue; // cross-module refs order modules (dep edges)
            }
            if let Some(need_rank) = rank_of(need)
                && need_rank > phase_rank(Some(step.action.execution_phase()))
            {
                diagnostics.push(
                    Diagnostic::error(
                        codes::STEP_PHASE_ORDER,
                        format!(
                            "module {name:?}: step {:?} ({:?}) needs {need:?} ({:?}), which runs later",
                            step.id,
                            step.phase,
                            steps.iter().find(|s| s.id == *need).map(|s| s.phase).unwrap_or(step.phase),
                        ),
                    )
                    .with_label(step.span.clone().or_else(|| module.span.clone()), "")
                    .with_help(
                        "post-deploy effects belong in activate hooks (customHook/service),                          which run after the flip",
                    ),
                );
            }
        }
    }
}

// Cycles in the union of purpose edges and scheduling-only needs are invalid
// even when each module's local step graph is acyclic.
fn check_module_cycles(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    let mut remaining: std::collections::BTreeSet<&str> =
        ir.modules.keys().map(String::as_str).collect();
    loop {
        let ready: Vec<_> = remaining
            .iter()
            .copied()
            .filter(|name| {
                crate::dependencies::ordering_dependencies(name, &ir.modules[*name])
                    .iter()
                    .all(|dep| !remaining.contains(dep))
            })
            .collect();
        if ready.is_empty() {
            break;
        }
        for name in ready {
            remaining.remove(name);
        }
    }
    if let Some(name) = remaining.first() {
        diagnostics.push(
            Diagnostic::error(
                codes::STEP_CYCLE,
                format!(
                    "module scheduling cycle blocks: {}",
                    remaining.iter().copied().collect::<Vec<_>>().join(", ")
                ),
            )
            .with_label(
                ir.modules[*name].span.clone(),
                "scheduling dependency declared here",
            ),
        );
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
            span: None,
        }
    }

    fn ir_with_steps(steps: Vec<Step>) -> Ir {
        Ir {
            ir_version: crate::IR_VERSION,
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
