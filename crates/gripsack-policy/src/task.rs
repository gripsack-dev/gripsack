//! Manual task graph decisions shared by the runner and verifier.
//! The adapter owns dependency enumeration, lock observations and process cleanup;
//! these contracts do not establish that those observations are complete.
use vstd::prelude::*;

verus! {
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
    SkippedBusy,
    Indeterminate,
}

/// Only confirmed success satisfies a prerequisite, including after cleanup.
pub fn prerequisite_succeeded(state: TaskState) -> (ready: bool)
    ensures ready == (state == TaskState::Succeeded),
{
    matches!(state, TaskState::Succeeded)
}

/// Unknown ownership retains exclusion even when no live holder was observed.
pub fn admit_conflict(held: bool, unknown: bool) -> (admitted: bool)
    ensures admitted == (!held && !unknown),
{
    !held && !unknown
}

/// Manual prerequisite edges require task targets in the host context.
pub fn admit_task_edge(target_is_task: bool, host_context: bool) -> (admitted: bool)
    ensures admitted == (target_is_task && host_context),
{
    target_is_task && host_context
}
}
