//! Classify admitted graph edges by role before closure/admission.
//! One pure implementation is used by the v4 IR adapter and Verus.
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

/// One decision per input edge in the same order. The source-span/name
/// adapter remains in IR, while this kernel decides whether the edge
/// belongs to build closure or mandatory validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleDecision {
    pub build: bool,
    pub required_validation: bool,
}

/// A recipe's publication checks cannot be pruned as dead build
/// inputs. Runtime/task/ordering/retention edges must not enter build
/// closure; every input receives exactly one decision.
pub fn project_graph_roles(roles: &[GraphRole]) -> (result: Vec<RoleDecision>)
    ensures
        result@.len() == roles@.len(),
        forall|j: int| 0 <= j < roles@.len() ==> (
            result@[j].build == (roles@[j] == GraphRole::Production
                || roles@[j] == GraphRole::BuildInput)
            && result@[j].required_validation == (roles@[j] == GraphRole::Validation)
        ),
{
    let mut result: Vec<RoleDecision> = Vec::new();
    let mut i: usize = 0;
    while i < roles.len()
        invariant
            i <= roles@.len(),
            result@.len() == i,
            forall|j: int| 0 <= j < i ==> (
                result@[j].build == (roles@[j] == GraphRole::Production
                    || roles@[j] == GraphRole::BuildInput)
                && result@[j].required_validation == (roles@[j] == GraphRole::Validation)
            ),
        decreases roles.len() - i,
    {
        let ghost previous = result@;
        let role = roles[i];
        let decision = match role {
            GraphRole::Production | GraphRole::BuildInput =>
                RoleDecision { build: true, required_validation: false },
            GraphRole::Validation =>
                RoleDecision { build: false, required_validation: true },
            GraphRole::Runtime | GraphRole::Ordering | GraphRole::TaskPrereq | GraphRole::Retention =>
                RoleDecision { build: false, required_validation: false },
        };
        proof {
            assert(decision.build == (role == GraphRole::Production
                || role == GraphRole::BuildInput));
            assert(decision.required_validation == (role == GraphRole::Validation));
        }
        let ghost decided = decision;
        result.push(decision);
        proof {
            assert(result@ == previous.push(decided));
            assert forall|j: int| 0 <= j < i + 1 implies (
                result@[j].build == (roles@[j] == GraphRole::Production
                    || roles@[j] == GraphRole::BuildInput)
                && result@[j].required_validation == (roles@[j] == GraphRole::Validation)
            ) by {
                if j < i {
                    assert(previous[j].build == (roles@[j] == GraphRole::Production
                        || roles@[j] == GraphRole::BuildInput));
                    assert(previous[j].required_validation ==
                        (roles@[j] == GraphRole::Validation));
                    assert(result@[j] == previous[j]);
                } else {
                    assert(j == i);
                    assert(roles@[j] == role);
                    assert(result@[j] == decided);
                    assert(decided.build == (role == GraphRole::Production
                        || role == GraphRole::BuildInput));
                    assert(decided.required_validation == (role == GraphRole::Validation));
                }
            };
        }
        i += 1;
    }
    result
}

}
