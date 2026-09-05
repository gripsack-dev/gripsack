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
        .map(|(name, m)| {
            (
                name.clone(),
                order_by_needs(m.steps.clone().unwrap_or_else(|| expand(m))),
            )
        })
        .collect()
}

/// Steps execute in `needs` order (0033 R4): a Kahn pass over the
/// intra-module refs, declaration order breaking ties. Unknown refs
/// were E104 at sema; cycles were E120 — a cycle that arrives anyway
/// (hand-written IR) leaves the remaining steps in declaration order
/// rather than dropping them.
fn order_by_needs(steps: Vec<Step>) -> Vec<Step> {
    let mut done = vec![false; steps.len()];
    let mut out = Vec::with_capacity(steps.len());
    loop {
        let mut progressed = false;
        for (i, step) in steps.iter().enumerate() {
            if done[i] {
                continue;
            }
            let ready = step.needs.iter().all(|need| {
                match need.split_once(':') {
                    // cross-module refs order MODULES (dep_edges), not
                    // this list
                    Some(_) => true,
                    None => !steps
                        .iter()
                        .enumerate()
                        .any(|(j, s)| s.id == *need && !done[j]),
                }
            });
            if ready {
                out.push(step.clone());
                done[i] = true;
                progressed = true;
            }
        }
        if out.len() == steps.len() {
            return out;
        }
        if !progressed {
            // a cycle slipped past sema (hand-written IR): keep the
            // remainder rather than dropping steps
            out.extend(
                steps
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !done[*i])
                    .map(|(_, s)| s.clone()),
            );
            return out;
        }
    }
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
pub fn check_physical_uniqueness(
    modules: &std::collections::BTreeMap<String, gripsack_ir::Module>,
    steps_by_module: &std::collections::BTreeMap<String, Vec<gripsack_ir::Step>>,
) -> Result<(), crate::ctx::ExecError> {
    struct Seen<'a> {
        module: &'a str,
        entry: &'a gripsack_ir::Entry,
    }
    let mut owners: std::collections::BTreeMap<std::path::PathBuf, Seen> =
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
                if let Some(other) = owners.get(&key) {
                    // the same quality of error as a typo in a module:
                    // code, both spellings, spans on both declarations,
                    // and a help line (E119)
                    let diagnostic = gripsack_ir::diagnostic::Diagnostic::error(
                        gripsack_ir::diagnostic::codes::DESTINATION_ALIAS,
                        format!(
                            "{:?} (module {name:?}) and {:?} (module {:?}) resolve to the same \
                             path {} — one physical destination per run",
                            entry.to,
                            other.entry.to,
                            other.module,
                            key.display()
                        ),
                    )
                    .with_help("these spellings are one file on disk — keep a single declaration");
                    let diagnostic = diagnostic
                        .with_label(
                            modules.get(other.module).and_then(|m| m.span.clone()),
                            format!("{:?} first declared in this module", other.entry.to),
                        )
                        .with_label(
                            modules.get(name.as_str()).and_then(|m| m.span.clone()),
                            format!("{:?} aliases it from here", entry.to),
                        );
                    return Err(crate::ctx::ExecError::Gate(diagnostic));
                }
                owners.insert(
                    key,
                    Seen {
                        module: name,
                        entry,
                    },
                );
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
