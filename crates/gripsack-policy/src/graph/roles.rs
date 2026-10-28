//! Classify admitted graph edges by role before closure/admission.
//! One pure implementation is used by the workspace IR adapter and Verus.
//! A required validation edge must remain in the validation projection;
//! runtime, task and retention edges never become build inputs.

use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRole {
    Production,
    BuildInput,
    Runtime,
    Ordering,
    TaskPrereq,
    Validation,
    Retention,
}

impl GraphRole {
    pub fn is_dependency(self) -> (result: bool)
        ensures result == (self == Self::Production || self == Self::BuildInput
            || self == Self::Runtime || self == Self::TaskPrereq),
    {
        match self {
            Self::Production | Self::BuildInput | Self::Runtime | Self::TaskPrereq => true,
            Self::Ordering | Self::Validation | Self::Retention => false,
        }
    }
}

/// The decision each role projects to, as spec data for composition
/// with the closure kernels.
pub open spec fn role_decision(role: GraphRole) -> RoleDecision {
    let build = role == GraphRole::Production || role == GraphRole::BuildInput;
    let required_validation = role == GraphRole::Validation;
    RoleDecision { build, required_validation }
}

/// Projection totality: every admitted edge role maps to exactly one
/// decision; required validation is never dropped and no role is both a
/// build edge and a validation gate.
pub proof fn lemma_role_decision_total(role: GraphRole)
    ensures
        role_decision(role).build == (role == GraphRole::Production || role
            == GraphRole::BuildInput),
        role_decision(role).required_validation == (role == GraphRole::Validation),
        !(role_decision(role).build && role_decision(role).required_validation),
{
}

/// One decision per input edge in the same order. The source-span/name
/// adapter remains in IR, while this kernel decides whether the edge
/// belongs to build closure or mandatory validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleDecision {
    pub build: bool,
    pub required_validation: bool,
}
/// Project one real collected reference without allocating a parallel role list.
pub fn project_graph_role(role: GraphRole) -> (decision: RoleDecision)
    ensures decision == role_decision(role),
{
    match role {
        GraphRole::Production | GraphRole::BuildInput =>
            RoleDecision { build: true, required_validation: false },
        GraphRole::Validation =>
            RoleDecision { build: false, required_validation: true },
        GraphRole::Runtime | GraphRole::Ordering | GraphRole::TaskPrereq | GraphRole::Retention =>
            RoleDecision { build: false, required_validation: false },
    }
}

}
