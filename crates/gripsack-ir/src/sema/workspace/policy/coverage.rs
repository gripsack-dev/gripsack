//! Compare decoded workspace references with projected graph edges.
//! This source-rooted admission check is independent of the graph
//! collector: each output streams its declared references in emission
//! order and compares role, target and selector/exported-command
//! payloads with the contiguous run of projected edges. A lost,
//! extra, reordered or substituted reference fails before the
//! verified closure can be treated as complete.
//! Successful admission performs no heap allocation; diagnostics
//! allocate only on rejection. The decode arrow (serde/tagged grammar),
//! catalog name → kernel index bridge in `index_view`, and the edge's
//! expected-kind/target-binding classifications remain unproved.

mod declaration;
mod diagnostics;

use super::super::graph::{EdgeRole, Projection, TargetBinding};
use super::super::names::Catalog;
use crate::diagnostic::{Diagnostic, codes};
use crate::workspace::Workspace;
use declaration::{DeclaredReference, for_each_declared};
use diagnostics::{
    count_mismatch, reclassified_edge, reclassified_target_rule, substituted_payload,
    substituted_target,
};

/// Per-role reference totals kept on the stack, so a successful
/// admission allocates nothing.
#[derive(Clone, Copy, Default)]
struct RoleCounts {
    production: usize,
    build_input: usize,
    runtime: usize,
    ordering: usize,
    task_prereq: usize,
    validation: usize,
    retention: usize,
}

impl RoleCounts {
    fn add(&mut self, role: EdgeRole, count: usize) {
        match role {
            EdgeRole::Production => self.production += count,
            EdgeRole::BuildInput => self.build_input += count,
            EdgeRole::Runtime => self.runtime += count,
            EdgeRole::Ordering => self.ordering += count,
            EdgeRole::TaskPrereq => self.task_prereq += count,
            EdgeRole::Validation => self.validation += count,
            EdgeRole::Retention => self.retention += count,
        }
    }

    fn get(self, role: EdgeRole) -> usize {
        match role {
            EdgeRole::Production => self.production,
            EdgeRole::BuildInput => self.build_input,
            EdgeRole::Runtime => self.runtime,
            EdgeRole::Ordering => self.ordering,
            EdgeRole::TaskPrereq => self.task_prereq,
            EdgeRole::Validation => self.validation,
            EdgeRole::Retention => self.retention,
        }
    }
}

/// The first sequence divergence found for one output, recorded while
/// the declared walk finishes counting totals for the diagnostic.
enum Divergence<'a> {
    /// A declared reference has no projected edge left in this
    /// output's run — the graph lost it.
    Missing { role: EdgeRole },
    /// Same role, different target at the same sequence position.
    Substituted {
        role: EdgeRole,
        declared: &'a str,
        projected: &'a str,
    },
    /// Different role at the same sequence position.
    Reclassified {
        declared: (EdgeRole, &'a str),
        projected: (EdgeRole, &'a str),
    },
    /// Same role and target, but a different artifact selector or
    /// exported package command.
    Payload {
        declared: DeclaredReference<'a>,
        projected_selector: Option<&'a str>,
        projected_command: Option<&'a str>,
    },
    /// Same declared reference, but the projection changes which
    /// target kinds or platform binding its consumer must admit.
    Classification {
        declared: DeclaredReference<'a>,
        projected_expected: &'static [&'static str],
        projected_binding: TargetBinding,
    },
}

pub(super) fn check(
    workspace: &Workspace,
    projection: &Projection,
    catalog: &Catalog,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut edges = projection.edges.iter().peekable();
    for output in &workspace.outputs {
        let mut declared_counts = RoleCounts::default();
        let mut matched_counts = RoleCounts::default();
        let mut divergence: Option<Divergence> = None;
        for_each_declared(output, &mut |reference| {
            declared_counts.add(reference.role, 1);
            if divergence.is_some() {
                // Keep counting declared totals for the diagnostic,
                // but consume nothing further.
                return;
            }
            match edges.next_if(|edge| std::ptr::eq(edge.from, output)) {
                Some(edge)
                    if edge.role == reference.role
                        && edge.to == reference.target
                        && edge.selector == reference.selector
                        && edge.command == reference.package_command
                        && edge.expected == reference.expected
                        && edge.binding == reference.binding =>
                {
                    matched_counts.add(reference.role, 1);
                }
                Some(edge)
                    if edge.role == reference.role
                        && edge.to == reference.target
                        && edge.selector == reference.selector
                        && edge.command == reference.package_command =>
                {
                    divergence = Some(Divergence::Classification {
                        declared: reference,
                        projected_expected: edge.expected,
                        projected_binding: edge.binding,
                    });
                }
                Some(edge) if edge.role == reference.role && edge.to == reference.target => {
                    divergence = Some(Divergence::Payload {
                        declared: reference,
                        projected_selector: edge.selector,
                        projected_command: edge.command,
                    });
                }
                Some(edge) if edge.role == reference.role => {
                    divergence = Some(Divergence::Substituted {
                        role: reference.role,
                        declared: reference.target,
                        projected: edge.to,
                    });
                }
                Some(edge) => {
                    divergence = Some(Divergence::Reclassified {
                        declared: (reference.role, reference.target),
                        projected: (edge.role, edge.to),
                    });
                }
                None => {
                    divergence = Some(Divergence::Missing {
                        role: reference.role,
                    })
                }
            }
        });
        // Drop the rest of this output's projected run so the next
        // output starts aligned; without an earlier divergence a
        // non-empty rest is a spuriously added edge.
        let mut extras = RoleCounts::default();
        let mut first_extra = None;
        while let Some(edge) = edges.next_if(|edge| std::ptr::eq(edge.from, output)) {
            extras.add(edge.role, 1);
            first_extra = first_extra.or(Some(edge.role));
        }
        let diagnostic = match divergence {
            Some(Divergence::Missing { role }) => Some(count_mismatch(
                output,
                role,
                declared_counts.get(role),
                matched_counts.get(role),
            )),
            Some(Divergence::Substituted {
                role,
                declared,
                projected,
            }) => Some(substituted_target(
                output, catalog, role, declared, projected,
            )),
            Some(Divergence::Reclassified {
                declared,
                projected,
            }) => Some(reclassified_edge(output, catalog, declared, projected)),
            Some(Divergence::Classification {
                declared,
                projected_expected,
                projected_binding,
            }) => Some(reclassified_target_rule(
                output,
                catalog,
                declared,
                projected_expected,
                projected_binding,
            )),
            Some(Divergence::Payload {
                declared,
                projected_selector,
                projected_command,
            }) => Some(substituted_payload(
                output,
                catalog,
                declared,
                projected_selector,
                projected_command,
            )),
            None => first_extra.map(|role| {
                count_mismatch(
                    output,
                    role,
                    declared_counts.get(role),
                    matched_counts.get(role) + extras.get(role),
                )
            }),
        };
        if let Some(diagnostic) = diagnostic {
            diagnostics.push(diagnostic);
        }
    }
    // An edge whose referencing output never claimed it — the
    // projection invented an edge or reordered across outputs.
    if let Some(edge) = edges.next() {
        diagnostics.push(
            Diagnostic::error(
                codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                "a projected edge has no catalog source; refusing an incomplete graph",
            )
            .with_label(Some(edge.from.span().clone()), "edge declared here"),
        );
    }
}
