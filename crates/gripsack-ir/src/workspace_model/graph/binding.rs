//! Bind the collected typed owner/target without trusting a candidate map.
use super::*;
#[cfg(verus_keep_ghost)]
use gripsack_policy::graph::{name_index::exact_name_at, roles::role_decision};
use gripsack_policy::graph::{
    name_index::{BoundOutputIndex, bind_output_index},
    roles::{RoleDecision, project_graph_role},
};
verus! {
pub open spec fn output_name_spec(output: &WorkspaceOutput) -> Seq<char> {
    match output {
        WorkspaceOutput::Recipe(v) => v.name@, WorkspaceOutput::Package(v) => v.name@,
        WorkspaceOutput::Environment(v) => v.name@, WorkspaceOutput::Task(v) => v.name@,
        WorkspaceOutput::Schedule(v) => v.name@, WorkspaceOutput::Check(v) => v.name@,
        WorkspaceOutput::Image(v) => v.name@, WorkspaceOutput::Profile(v) => v.name@,
        WorkspaceOutput::Hook(v) => v.name@,
    }
}
pub open spec fn output_kind_spec(output: &WorkspaceOutput) -> Seq<char> {
    match output {
        WorkspaceOutput::Recipe(_) => "recipe"@, WorkspaceOutput::Package(_) => "package"@,
        WorkspaceOutput::Environment(_) => "environment"@, WorkspaceOutput::Task(_) => "task"@,
        WorkspaceOutput::Schedule(_) => "schedule"@, WorkspaceOutput::Check(_) => "check"@,
        WorkspaceOutput::Image(_) => "image"@, WorkspaceOutput::Profile(_) => "profile"@,
        WorkspaceOutput::Hook(_) => "hook"@,
    }
}
pub fn output_kind(output: &WorkspaceOutput) -> (kind: &'static str)
    ensures kind@ == output_kind_spec(output),
{
    match output {
        WorkspaceOutput::Recipe(_) => "recipe", WorkspaceOutput::Package(_) => "package",
        WorkspaceOutput::Environment(_) => "environment", WorkspaceOutput::Task(_) => "task",
        WorkspaceOutput::Schedule(_) => "schedule", WorkspaceOutput::Check(_) => "check",
        WorkspaceOutput::Image(_) => "image", WorkspaceOutput::Profile(_) => "profile",
        WorkspaceOutput::Hook(_) => "hook",
    }
}
pub fn output_name(output: &WorkspaceOutput) -> (name: &str)
    ensures name@ == output_name_spec(output),
{
    match output {
        WorkspaceOutput::Recipe(v) => &v.name, WorkspaceOutput::Package(v) => &v.name,
        WorkspaceOutput::Environment(v) => &v.name, WorkspaceOutput::Task(v) => &v.name,
        WorkspaceOutput::Schedule(v) => &v.name, WorkspaceOutput::Check(v) => &v.name,
        WorkspaceOutput::Image(v) => &v.name, WorkspaceOutput::Profile(v) => &v.name,
        WorkspaceOutput::Hook(v) => &v.name,
    }
}
#[derive(Debug, Clone, Copy)]
pub struct BoundReference {
    pub from: BoundOutputIndex,
    pub to: BoundOutputIndex,
    pub decision: RoleDecision,
    pub dependency: bool,
}
pub fn bind_reference(names: &[&str], edge: &Reference<'_>, from: Option<usize>, to: Option<usize>) -> (bound: Option<BoundReference>)
    ensures
        bound.is_some() <==> (exact_name_at(names@.map_values(|s: &str| s@), output_name_spec(edge.from), from)
            && exact_name_at(names@.map_values(|s: &str| s@), edge.to@, to)),
        match bound {
            Some(value) => value.from.verified_position() < names@.len()
                && value.to.verified_position() < names@.len()
                && names@[value.from.verified_position() as int]@ == output_name_spec(edge.from)
                && names@[value.to.verified_position() as int]@ == edge.to@
                && value.decision == role_decision(edge.role)
                && value.dependency == (edge.role == GraphRole::Production || edge.role == GraphRole::BuildInput
                    || edge.role == GraphRole::Runtime || edge.role == GraphRole::TaskPrereq),
            None => true,
        },
{
    let from = bind_output_index(names, output_name(edge.from), from)?;
    let to = bind_output_index(names, edge.to, to)?;
    Some(BoundReference { from, to, decision: project_graph_role(edge.role), dependency: edge.role.is_dependency() })
}
}
