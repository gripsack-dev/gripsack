//! Bind the collected typed owner/target without trusting a candidate map.
use super::*;
use gripsack_policy::graph::{BoundOutputIndex, bind_output_index};

pub fn output_kind(output: &WorkspaceOutput) -> &'static str {
    match output {
        WorkspaceOutput::Recipe(_) => "recipe",
        WorkspaceOutput::Package(_) => "package",
        WorkspaceOutput::Environment(_) => "environment",
        WorkspaceOutput::Task(_) => "task",
        WorkspaceOutput::Schedule(_) => "schedule",
        WorkspaceOutput::Check(_) => "check",
        WorkspaceOutput::Image(_) => "image",
        WorkspaceOutput::Profile(_) => "profile",
        WorkspaceOutput::Hook(_) => "hook",
        WorkspaceOutput::Service(_) => "service",
    }
}
pub fn output_name(output: &WorkspaceOutput) -> &str {
    match output {
        WorkspaceOutput::Recipe(v) => &v.name,
        WorkspaceOutput::Package(v) => &v.name,
        WorkspaceOutput::Environment(v) => &v.name,
        WorkspaceOutput::Task(v) => &v.name,
        WorkspaceOutput::Schedule(v) => &v.name,
        WorkspaceOutput::Check(v) => &v.name,
        WorkspaceOutput::Image(v) => &v.name,
        WorkspaceOutput::Profile(v) => &v.name,
        WorkspaceOutput::Hook(v) => &v.name,
        WorkspaceOutput::Service(v) => &v.name,
    }
}
#[derive(Debug, Clone, Copy)]
pub struct BoundReference {
    pub from: BoundOutputIndex,
    pub to: BoundOutputIndex,
}
pub fn bind_reference(
    names: &[&str],
    edge: &Reference<'_>,
    from: Option<usize>,
    to: Option<usize>,
) -> Option<BoundReference> {
    let from = bind_output_index(names, output_name(edge.from), from)?;
    let to = bind_output_index(names, edge.to, to)?;
    Some(BoundReference { from, to })
}
