//! E131 diagnostics for missing, substituted or reclassified edges;
//! successful graph admission does not build any of these messages.

use super::super::super::graph::{EdgeRole, TargetBinding};
use super::super::super::names::Catalog;
use super::declaration::DeclaredReference;
use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::WorkspaceOutput;

fn role_name(role: EdgeRole) -> &'static str {
    match role {
        EdgeRole::Production => "production",
        EdgeRole::BuildInput => "build input",
        EdgeRole::Runtime => "runtime",
        EdgeRole::Ordering => "ordering",
        EdgeRole::TaskPrereq => "task prerequisite",
        EdgeRole::Validation => "validation",
        EdgeRole::Retention => "retention",
    }
}

/// Cardinality divergence: the projection lost or invented references.
pub(super) fn count_mismatch(
    output: &WorkspaceOutput,
    role: EdgeRole,
    required: usize,
    found: usize,
) -> Diagnostic {
    Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` declares {required} {} reference(s), but the graph projects {found}; refusing incomplete admission",
            output.kind(), output.name(), role_name(role),
        ),
    )
    .with_label(Some(output.span().clone()), "referencing output declared here")
}

/// A dropped reference names both its original source site and, when
/// present, the catalog target; no closure computation is required to
/// recover those declaration sites.
pub(super) fn missing_reference(
    output: &WorkspaceOutput,
    catalog: &Catalog,
    declared: DeclaredReference<'_>,
    required: usize,
    found: usize,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` declares {required} {} reference(s), including `{}`, but the graph projects {found}; refusing incomplete admission",
            output.kind(),
            output.name(),
            role_name(declared.role),
            declared.target,
        ),
    )
    .with_label(Some(declared.at.clone()), "reference declared here");
    if let Some(target) = catalog.outputs.get(declared.target) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "target declared here");
    }
    diagnostic
}

/// Identity divergence at equal cardinality: the projection names a
/// different target than the source declares at the same position, so
/// the referencing output and both catalog targets are labeled when
/// the names resolve.
pub(super) fn substituted_target(
    output: &WorkspaceOutput,
    catalog: &Catalog,
    role: EdgeRole,
    declared: &str,
    projected: &str,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` declares {} reference `{declared}`, but the graph projects `{projected}` in its place; refusing a substituted admitted graph",
            output.kind(), output.name(), role_name(role),
        ),
    )
    .with_label(Some(output.span().clone()), "referencing output declared here");
    if let Some(target) = catalog.outputs.get(declared) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "declared target here");
    }
    if let Some(target) = catalog.outputs.get(projected) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "projected target here");
    }
    diagnostic
}

/// Role divergence at a sequence position: the graph classified the
/// reference under a different role than the source declares.
pub(super) fn reclassified_edge(
    output: &WorkspaceOutput,
    catalog: &Catalog,
    declared: (EdgeRole, &str),
    projected: (EdgeRole, &str),
) -> Diagnostic {
    let (declared_role, declared_target) = declared;
    let (projected_role, projected_target) = projected;
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` declares {} reference `{declared_target}`, but the graph projects a {} edge to `{projected_target}` in its place; refusing a mismatched admitted graph",
            output.kind(), output.name(), role_name(declared_role), role_name(projected_role),
        ),
    )
    .with_label(Some(output.span().clone()), "referencing output declared here");
    if let Some(target) = catalog.outputs.get(declared_target) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "declared target here");
    }
    if let Some(target) = catalog.outputs.get(projected_target) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "projected target here");
    }
    diagnostic
}

/// A same-target edge that silently changes the selected artifact or
/// executable is not the reference the workspace author declared.
pub(super) fn substituted_payload(
    output: &WorkspaceOutput,
    catalog: &Catalog,
    declared: DeclaredReference<'_>,
    projected_selector: Option<&str>,
    projected_command: Option<&str>,
) -> Diagnostic {
    let (field, expected, projected) = if declared.selector != projected_selector {
        (
            "artifact selector",
            declared.selector.unwrap_or("<none>"),
            projected_selector.unwrap_or("<none>"),
        )
    } else {
        (
            "package command",
            declared.package_command.unwrap_or("<none>"),
            projected_command.unwrap_or("<none>"),
        )
    };
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` declares {field} `{expected}` on `{}`, but the graph projects `{projected}`; refusing a substituted admitted graph",
            output.kind(), output.name(), declared.target,
        ),
    )
    .with_label(Some(declared.at.clone()), "reference declared here");
    if declared.at != output.span() {
        diagnostic = diagnostic.with_label(
            Some(output.span().clone()),
            "referencing output declared here",
        );
    }
    if let Some(target) = catalog.outputs.get(declared.target) {
        diagnostic = diagnostic.with_label(
            Some(target.span().clone()),
            "referenced output declared here",
        );
    }
    diagnostic
}

/// The graph may keep a reference's role, target and payload while
/// broadening its admissible output kinds or dropping its platform
/// binding. Refuse that adapter reclassification before the policy
/// kernel receives the incomplete relation.
pub(super) fn reclassified_target_rule(
    output: &WorkspaceOutput,
    catalog: &Catalog,
    declared: DeclaredReference<'_>,
    projected_expected: &[&str],
    projected_binding: TargetBinding,
) -> Diagnostic {
    let change = if declared.expected != projected_expected {
        format!(
            "target kind rule {:?} as {:?}",
            declared.expected, projected_expected
        )
    } else {
        format!(
            "target binding rule {:?} as {:?}",
            declared.binding, projected_binding
        )
    };
    let mut diagnostic = Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` projects {} reference `{}` with a reclassified {change}; refusing incomplete admission",
            output.kind(), output.name(), role_name(declared.role), declared.target,
        ),
    )
    .with_label(Some(declared.at.clone()), "reference declared here");
    if declared.at != output.span() {
        diagnostic = diagnostic.with_label(
            Some(output.span().clone()),
            "referencing output declared here",
        );
    }
    if let Some(target) = catalog.outputs.get(declared.target) {
        diagnostic = diagnostic.with_label(Some(target.span().clone()), "target declared here");
    }
    diagnostic
}

/// A projected edge may resolve to the right name and role yet carry
/// the wrong reference-site span, hiding which source declaration
/// authorized it. The source walk supplies the authoritative site.
pub(super) fn substituted_reference_site(
    output: &WorkspaceOutput,
    declared: DeclaredReference<'_>,
    projected_at: &crate::span::Span,
) -> Diagnostic {
    Diagnostic::error(
        codes::REQUIRED_WORKSPACE_EDGE_MISSING,
        format!(
            "workspace {} `{}` projects {:?} reference `{}` with a substituted reference site; refusing incorrectly attributed admission",
            output.kind(),
            output.name(),
            declared.role,
            declared.target,
        ),
    )
    .with_label(Some(declared.at.clone()), "reference declared here")
    .with_label(Some(projected_at.clone()), "graph attributed it here")
}
