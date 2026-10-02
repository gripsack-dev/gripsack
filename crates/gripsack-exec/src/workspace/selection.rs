//! Selection is an outer consumer closure, not a scheduler for producer steps.
//! Production, runtime and required-validation references come from the same
//! decoded projection used by IR admission; BuildKit owns internal scheduling.
use crate::ExecError;
use gripsack_ir::{
    Diagnostic, codes,
    workspace_v6::{
        WorkspaceOutput, WorkspaceV6,
        graph::{Reference, references},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ops::Range,
};

pub(super) struct Selection<'a> {
    pub workspace: &'a WorkspaceV6,
    pub outputs: BTreeMap<&'a str, &'a WorkspaceOutput>,
    pub required: BTreeSet<&'a str>,
    edges: Vec<Reference<'a>>,
    by_owner: BTreeMap<&'a str, Range<usize>>,
}
impl<'a> Selection<'a> {
    pub fn admit(workspace: &'a WorkspaceV6, requested: &[String]) -> Result<Self, ExecError> {
        if requested.is_empty() {
            return Err(ExecError::Gate(
                Diagnostic::error(
                    codes::INVALID_WORKSPACE_VALUE,
                    "select at least one named workspace output",
                )
                .with_label(Some(workspace.span.clone()), "workspace declared here"),
            ));
        }
        let mut outputs = BTreeMap::new();
        for output in &workspace.outputs {
            if let Some(previous) = outputs.insert(output.name(), output) {
                return Err(ExecError::Gate(
                    Diagnostic::error(
                        codes::DUPLICATE_WORKSPACE_OUTPUT,
                        "duplicate output in realization catalog",
                    )
                    .with_label(Some(previous.span().clone()), "first declaration")
                    .with_label(Some(output.span().clone()), "conflicting declaration"),
                ));
            }
        }
        let mut edges = references(workspace);
        edges.sort_unstable_by_key(|edge| edge.from.name());
        let mut by_owner: BTreeMap<&str, Range<usize>> = BTreeMap::new();
        for (index, edge) in edges.iter().enumerate() {
            by_owner
                .entry(edge.from.name())
                .and_modify(|range| range.end = index + 1)
                .or_insert(index..index + 1);
            if !outputs.contains_key(edge.to) {
                return Err(ExecError::Gate(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!("unknown output {:?}", edge.to),
                    )
                    .with_label(Some(edge.at.clone()), "reference declared here"),
                ));
            }
        }
        let mut pending = VecDeque::new();
        for name in requested {
            let output = outputs.get(name.as_str()).ok_or_else(|| {
                ExecError::Gate(
                    Diagnostic::error(
                        codes::UNKNOWN_WORKSPACE_REF,
                        format!("no workspace output named {name:?}"),
                    )
                    .with_label(Some(workspace.span.clone()), "workspace declared here"),
                )
            })?;
            pending.push_back(output.name());
        }
        let mut required = BTreeSet::new();
        while let Some(name) = pending.pop_front() {
            if !required.insert(name) {
                continue;
            }
            if let Some(range) = by_owner.get(name) {
                for edge in &edges[range.clone()] {
                    pending.push_back(edge.to);
                }
            }
        }
        Ok(Self {
            workspace,
            outputs,
            required,
            edges,
            by_owner,
        })
    }
    pub fn references(&self, name: &str) -> &[Reference<'a>] {
        self.by_owner
            .get(name)
            .map_or(&[], |range| &self.edges[range.clone()])
    }
}
