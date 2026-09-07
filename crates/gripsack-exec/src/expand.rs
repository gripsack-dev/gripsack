//! Physical destination admission and command-level preparation (0041).

use gripsack_ir::{Module, prepared::PreparedModule};
use std::collections::BTreeMap;

pub fn expand_all(
    modules: &BTreeMap<String, Module>,
) -> Result<BTreeMap<String, PreparedModule>, crate::ctx::ExecError> {
    modules
        .iter()
        .map(|(name, module)| {
            PreparedModule::new(module)
                .map(|plan| (name.clone(), plan))
                .map_err(crate::ctx::ExecError::Gate)
        })
        .collect()
}

pub fn dep_edges(modules: &BTreeMap<String, Module>) -> Vec<(String, String)> {
    modules
        .iter()
        .flat_map(|(name, module)| {
            gripsack_ir::dependencies::ordering_dependencies(name, module)
                .into_iter()
                .map(move |dep| (name.clone(), dep.to_owned()))
        })
        .collect()
}

/// Physical destination uniqueness (0030 §P0-1): expand `~/`,
/// normalize, and canonicalize the deepest existing ancestor — two
/// declarations resolving to one directory entry are a hard error
pub fn check_physical_uniqueness(
    modules: &std::collections::BTreeMap<String, gripsack_ir::Module>,
    steps_by_module: &std::collections::BTreeMap<String, PreparedModule>,
) -> Result<(), crate::ctx::ExecError> {
    struct Seen<'a> {
        module: &'a str,
        entry: &'a gripsack_ir::Entry,
    }
    let mut owners: std::collections::BTreeMap<std::path::PathBuf, Seen> =
        std::collections::BTreeMap::new();
    let build_only = gripsack_ir::dependencies::build_only_modules(modules);
    for (name, steps) in steps_by_module {
        if build_only.contains(name) {
            continue;
        }
        for step in steps.steps() {
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
