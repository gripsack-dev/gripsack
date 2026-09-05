//! Expansion pass (0007 §1): declarative module fields → the
//! conventional pipeline as explicit steps. After this pass the
//! executor only ever sees steps.

#[cfg(test)]
use gripsack_ir::Entry;
use gripsack_ir::{Dependency, Module, Phase, Step, StepAction, Verify};

/// The conventional pipeline: fetch → build → install → config →
/// verify → activate. Sequentially chained — explicit steps are the
/// escape for anything more exotic.
pub fn expand(module: &Module) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    let mut push = |id: &str, action: StepAction, phase: Phase, verify: Option<Verify>| {
        let needs = steps.last().map(|s| vec![s.id.clone()]).unwrap_or_default();
        steps.push(Step {
            id: id.to_string(),
            action,
            needs,
            resources: Vec::new(),
            phase: Some(phase),
            verify,
            retries: None,
            span: None,
        });
    };

    if let Some(fetch) = &module.fetch {
        push(
            "fetch",
            StepAction::Fetch {
                fetch: fetch.clone(),
            },
            Phase::Fetch,
            None,
        );
    }
    if module.build != gripsack_ir::Build::None {
        push(
            "build",
            StepAction::Build {
                spec: module.build.clone(),
            },
            Phase::Build,
            None,
        );
    }
    if !module.install.is_empty() {
        push(
            "install",
            StepAction::Install {
                entries: module.install.clone(),
            },
            Phase::Install,
            None,
        );
    }
    if !module.config.is_empty() {
        push(
            "config",
            StepAction::ConfigDeploy {
                entries: module.config.clone(),
            },
            Phase::Config,
            None,
        );
    }
    if let Some(verify) = &module.verify {
        push(
            "verify",
            StepAction::Verify {
                verify: verify.clone(),
            },
            Phase::Verify,
            None,
        );
    }
    for intent in &module.activate {
        push(
            "activate",
            StepAction::Intent {
                action: Box::new(intent.action.clone()),
            },
            Phase::Activate,
            None,
        );
    }
    steps
}

/// Expand every declarative module; explicit-step modules pass through.
pub fn expand_all(
    modules: &std::collections::BTreeMap<String, Module>,
) -> std::collections::BTreeMap<String, Vec<Step>> {
    modules
        .iter()
        .map(|(name, m)| (name.clone(), m.steps.clone().unwrap_or_else(|| expand(m))))
        .collect()
}

/// A module's effective steps: declared ones pass through, declarative
/// fields expand — the same view expand_all gives the scheduler.
/// Anything walking module structure (the lockfile resolver) must see
/// this or explicit-steps modules become invisible to it.
pub fn steps_of(module: &Module) -> Vec<Step> {
    module.steps.clone().unwrap_or_else(|| expand(module))
}

/// Module dependency edges as (dependent, dependency) pairs — used by
/// the executor to scope subset applies (0001 §3.6).
pub fn dep_edges(modules: &std::collections::BTreeMap<String, Module>) -> Vec<(String, String)> {
    let mut edges = Vec::new();
    for (name, m) in modules {
        for Dependency { module: dep, .. } in &m.depends {
            edges.push((name.clone(), dep.clone()));
        }
    }
    edges
}

/// Physical destination uniqueness (0030 §P0-1): expand `~/`,
/// normalize, and canonicalize the deepest existing ancestor — two
/// declarations resolving to one directory entry are a hard error
/// BEFORE any mutation. They would race in the scheduler and
/// double-journal one object. Runs in `grip check` and apply.
pub fn check_physical_uniqueness(
    steps_by_module: &std::collections::BTreeMap<String, Vec<gripsack_ir::Step>>,
) -> Result<(), crate::ctx::ExecError> {
    let mut owners: std::collections::BTreeMap<std::path::PathBuf, (&str, &str)> =
        std::collections::BTreeMap::new();
    for (name, steps) in steps_by_module {
        for step in steps {
            let entries: &[gripsack_ir::Entry] = match &step.action {
                gripsack_ir::StepAction::Install { entries }
                | gripsack_ir::StepAction::ConfigDeploy { entries } => entries,
                _ => continue,
            };
            for entry in entries {
                let key = gripsack_store::canonical_dest(&entry.to).map_err(|e| {
                    crate::ctx::ExecError::Step {
                        module: name.clone(),
                        step: "plan".into(),
                        detail: format!("destination {:?}: {e}", entry.to),
                    }
                })?;
                if let Some((other_module, other_to)) = owners.get(&key) {
                    return Err(crate::ctx::ExecError::Step {
                        module: name.clone(),
                        step: "plan".into(),
                        detail: format!(
                            "{:?} and {:?} (module {other_module:?}) resolve to the same path {} \
                             — one physical destination per run",
                            entry.to,
                            other_to,
                            key.display()
                        ),
                    });
                }
                owners.insert(key, (name.as_str(), entry.to.as_str()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::FetchSpec;

    #[test]
    fn declarative_module_expands_in_pipeline_order() {
        let module = Module {
            fetch: Some(FetchSpec::File { path: "/x".into() }),
            install: vec![Entry {
                from: "bin/hx".into(),
                to: "~/.local/bin/hx".into(),
                mode: Default::default(),
                vars: Default::default(),
                marker: None,
                span: None,
            }],
            ..Default::default()
        };
        let steps = expand(&module);
        let ids: Vec<_> = steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["fetch", "install"]);
        assert!(steps[0].needs.is_empty());
        assert_eq!(steps[1].needs, ["fetch"]);
    }

    #[test]
    fn dotfiles_only_module_expands_to_config() {
        let module = Module {
            config: vec![Entry {
                from: "config.toml".into(),
                to: "~/.config/helix/config.toml".into(),
                mode: Default::default(),
                vars: Default::default(),
                marker: None,
                span: None,
            }],
            ..Default::default()
        };
        let steps = expand(&module);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].id, "config");
    }
}
