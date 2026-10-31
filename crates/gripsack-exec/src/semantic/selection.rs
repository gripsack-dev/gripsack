//! Per-profile contracts follow admitted declaration dependencies, not apply-wide outputs.
use super::{model::*, normalize::subject};
use std::collections::BTreeSet;

pub(super) fn profile(
    ir: &gripsack_ir::Ir,
    document: &mut SnapshotDocument,
    name: &str,
) -> Result<(), SnapshotError> {
    let root = subject("output", name);
    if !document
        .nodes
        .iter()
        .any(|node| node.id == root && node.kind == SubjectKind::Profile)
    {
        return Err(SnapshotError::Invalid(
            "selected semantic root is not a profile",
        ));
    }
    let selection =
        crate::workspace::selection::Selection::admit(&ir.workspace, &[name.to_owned()])
            .map_err(|_| SnapshotError::Invalid("selected semantic dependency closure"))?;
    let mut selected: BTreeSet<_> = selection
        .required
        .iter()
        .map(|name| subject("output", name))
        .collect();
    for edge in &document.edges {
        if edge.dependency.0.starts_with("input/") && selected.contains(&edge.consumer) {
            selected.insert(edge.dependency.clone());
        }
    }
    selected.insert(SubjectRef("workspace".into()));
    document.nodes.retain(|node| selected.contains(&node.id));
    document
        .edges
        .retain(|edge| selected.contains(&edge.consumer) && selected.contains(&edge.dependency));
    Ok(())
}
