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
    modules: &BTreeMap<String, Module>,
    steps_by_module: &BTreeMap<String, PreparedModule>,
) -> Result<(), crate::ctx::ExecError> {
    let build_only = gripsack_ir::dependencies::build_only_modules(modules);
    check_destinations(
        steps_by_module
            .iter()
            .filter(|(name, _)| !build_only.contains(*name))
            .flat_map(|(name, plan)| {
                plan.entries().map(move |entry| DestinationDeclaration {
                    owner: name,
                    path: &entry.to,
                    span: entry.span.as_ref().or_else(|| modules[name].span.as_ref()),
                    // Legacy modules retain their one-physical-destination contract.
                    block: None,
                })
            }),
    )
}

pub(crate) struct DestinationDeclaration<'a> {
    pub owner: &'a str,
    pub path: &'a str,
    pub span: Option<&'a gripsack_ir::Span>,
    pub block: Option<String>,
}

/// One physical admission boundary for legacy entries and workspace files.
/// Only explicitly distinct workspace blocks may share a hosting file.
pub(crate) fn check_destinations<'a>(
    declarations: impl IntoIterator<Item = DestinationDeclaration<'a>>,
) -> Result<(), crate::ctx::ExecError> {
    let mut owners: BTreeMap<String, Vec<DestinationDeclaration<'a>>> = BTreeMap::new();
    for declaration in declarations {
        let path = gripsack_store::canonical_dest(declaration.path).map_err(|error| {
            crate::ctx::ExecError::Gate(
                gripsack_ir::Diagnostic::error(
                    gripsack_ir::codes::BAD_DESTINATION,
                    format!("destination {:?}: {error}", declaration.path),
                )
                .with_label(declaration.span.cloned(), "destination declared here"),
            )
        })?;
        let key = path.to_string_lossy().to_lowercase();
        let previous = owners.entry(key).or_default();
        if let Some(other) = previous.iter().find(|other| {
            declaration.block.is_none() || other.block.is_none() || declaration.block == other.block
        }) {
            return Err(crate::ctx::ExecError::Gate(gripsack_ir::Diagnostic::error(
                gripsack_ir::codes::DESTINATION_ALIAS,
                format!(
                    "{:?} (owner {:?}) and {:?} (owner {:?}) resolve to conflicting ownership at {}",
                    declaration.path, declaration.owner, other.path, other.owner, path.display()
                ),
            )
            .with_label(other.span.cloned(), "first owner declared here")
            .with_label(declaration.span.cloned(), "conflicting owner declared here")
            .with_help("keep one whole-file owner or distinct managed-block markers")));
        }
        previous.push(declaration);
    }
    Ok(())
}
