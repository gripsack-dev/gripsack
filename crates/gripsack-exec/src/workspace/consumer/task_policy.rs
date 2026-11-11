//! Manual task graph decisions shared by the runner and the lock planner.
//! The adapter owns dependency enumeration, lock observations and process cleanup;
//! these contracts do not establish that those observations are complete.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
    SkippedBusy,
    Indeterminate,
}

/// Only confirmed success satisfies a prerequisite, including after cleanup.
pub fn prerequisite_succeeded(state: TaskState) -> bool {
    matches!(state, TaskState::Succeeded)
}

/// Unknown ownership retains exclusion even when no live holder was observed.
pub fn admit_conflict(held: bool, unknown: bool) -> bool {
    !held && !unknown
}

/// Manual prerequisite edges require task targets in the host context.
pub fn admit_task_edge(target_is_task: bool, host_context: bool) -> bool {
    target_is_task && host_context
}

#[cfg(test)]
mod tests;
