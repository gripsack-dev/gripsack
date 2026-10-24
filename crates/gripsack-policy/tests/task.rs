use gripsack_policy::task::{TaskState, admit_conflict, admit_task_edge, prerequisite_succeeded};

#[test]
fn every_task_state_has_explicit_readiness() {
    for (state, expected) in [
        (TaskState::Pending, false),
        (TaskState::Succeeded, true),
        (TaskState::Failed, false),
        (TaskState::Blocked, false),
        (TaskState::Cancelled, false),
        (TaskState::SkippedBusy, false),
        (TaskState::Indeterminate, false),
    ] {
        assert_eq!(prerequisite_succeeded(state), expected, "{state:?}");
    }
}

#[test]
fn every_conflict_observation_fails_closed() {
    for (held, unknown, expected) in [
        (false, false, true),
        (false, true, false),
        (true, false, false),
        (true, true, false),
    ] {
        assert_eq!(admit_conflict(held, unknown), expected);
    }
}

#[test]
fn every_task_edge_role_and_context_is_checked() {
    for (task, host, expected) in [
        (false, false, false),
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        assert_eq!(admit_task_edge(task, host), expected);
    }
}
