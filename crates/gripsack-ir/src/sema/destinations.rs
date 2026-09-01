//! E102 — destinations must be absolute or ~/-prefixed.
//! E111 — two modules may not declare the same destination (a race in
//! parallel deploy, and why-owns needs a unique owner).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;
use crate::step::StepAction;

/// Every deploy entry a module declares: the declarative install/
/// config fields plus the entries inside explicit step actions —
/// both lower to the same deploy, so both must obey the same rules.
fn entries<'a>(
    module: &'a crate::model::Module,
) -> Box<dyn Iterator<Item = &'a crate::model::Entry> + 'a> {
    let declarative = module.install.iter().chain(module.config.iter());
    let stepped = module.steps.iter().flatten().flat_map(|s| match &s.action {
        StepAction::Install { entries } | StepAction::ConfigDeploy { entries } => {
            Box::new(entries.iter()) as Box<dyn Iterator<Item = _>>
        }
        _ => Box::new(std::iter::empty()),
    });
    Box::new(declarative.chain(stepped))
}

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    let mut owners: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    for (name, module) in &ir.modules {
        for entry in entries(module) {
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
        for entry in entries(module) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Entry, Module, Ownership};
    use crate::step::{Step, StepAction};
    fn install_step(to: &str) -> Step {
        Step {
            id: "s".into(),
            action: StepAction::Install {
                entries: vec![Entry {
                    from: "bin/x".into(),
                    to: to.into(),
                    mode: Ownership::Owned,
                    vars: Default::default(),
                    marker: None,
                    span: None,
                }],
            },
            needs: vec![],
            resources: vec![],
            phase: None,
            verify: None,
            retries: None,
            span: None,
        }
    }

    fn ir_with(steps: Vec<Step>) -> Ir {
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
    fn step_entries_obey_destination_rules() {
        // relative destination in a step: E102 must fire, not pass
        let mut diags = Vec::new();
        check(&ir_with(vec![install_step("relative/bin/x")]), &mut diags);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_ref() == codes::BAD_DESTINATION)
        );

        // same destination from two steps of different modules: E111
        let mut ir = ir_with(vec![install_step("~/.local/bin/x")]);
        ir.modules.insert(
            "n".to_string(),
            Module {
                steps: Some(vec![install_step("~/.local/bin/x")]),
                ..Default::default()
            },
        );
        let mut diags = Vec::new();
        check(&ir, &mut diags);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_ref() == codes::DUPLICATE_DESTINATION)
        );
    }
}
