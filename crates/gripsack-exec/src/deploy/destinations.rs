//! Physical workspace destination admission.
use std::collections::BTreeMap;

pub(crate) struct DestinationDeclaration<'a> {
    pub owner: &'a str,
    pub path: &'a str,
    pub span: Option<&'a gripsack_ir::Span>,
    pub block: Option<String>,
}

pub(crate) fn check_destinations_with<'a>(
    declarations: impl IntoIterator<Item = DestinationDeclaration<'a>>,
    resolve: impl Fn(&str) -> std::io::Result<std::path::PathBuf>,
) -> Result<(), crate::ctx::ExecError> {
    let mut owners: BTreeMap<String, Vec<DestinationDeclaration<'a>>> = BTreeMap::new();
    for declaration in declarations {
        let path = resolve(declaration.path).map_err(|error| {
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
