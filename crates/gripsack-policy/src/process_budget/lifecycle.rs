//! Exclusive-child lifecycle decisions; wait/signal outcomes are external facts.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderState {
    Running,
    Exited,
    Reaping,
    Reaped,
    Lost,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupSignalState {
    Pending,
    Attempted,
    Signalled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveAction {
    Observe,
    Exited,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    Signal,
    AlreadySignalled,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapObservation {
    Reaped,
    NotReady,
    Lost,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupDecision {
    Wait,
    Complete,
    Deadline,
    LostOwnership,
}

pub struct ChildLifecycle {
    leader: LeaderState,
    group: GroupSignalState,
    observed: bool,
    finished: bool,
}

impl Default for ChildLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl ChildLifecycle {
    pub fn new() -> Self {
        Self {
            leader: LeaderState::Running,
            group: GroupSignalState::Pending,
            observed: false,
            finished: false,
        }
    }

    pub fn observe_action(&self) -> ObserveAction {
        if self.finished {
            ObserveAction::Unavailable
        } else {
            match self.leader {
                LeaderState::Running => ObserveAction::Observe,
                LeaderState::Exited => ObserveAction::Exited,
                _ => ObserveAction::Unavailable,
            }
        }
    }

    pub fn observe_exit(&mut self, exited: bool) {
        if self.finished || !matches!(self.leader, LeaderState::Running | LeaderState::Exited) {
            self.leader = LeaderState::Lost;
        } else {
            self.observed = true;
            if exited {
                self.leader = LeaderState::Exited;
            }
        }
    }

    pub fn lose_ownership(&mut self) {
        self.leader = LeaderState::Lost;
    }

    pub fn signal_action(&mut self) -> SignalAction {
        if self.finished
            || !self.observed
            || !matches!(self.leader, LeaderState::Running | LeaderState::Exited)
        {
            return SignalAction::Unavailable;
        }
        if matches!(self.group, GroupSignalState::Signalled) {
            return SignalAction::AlreadySignalled;
        }
        self.group = GroupSignalState::Attempted;
        SignalAction::Signal
    }

    /// Only after `signal_action` returned `Signal` and the kill succeeded.
    pub fn signal_succeeded(&mut self) {
        debug_assert!(
            !self.finished && self.observed && matches!(self.group, GroupSignalState::Attempted)
        );
        self.group = GroupSignalState::Signalled;
    }

    pub fn needs_termination(&self) -> bool {
        !self.finished
            && matches!(self.leader, LeaderState::Running | LeaderState::Exited)
            && !matches!(self.group, GroupSignalState::Signalled)
    }

    pub fn should_observe_for_reap(&self) -> bool {
        !self.finished
            && matches!(self.leader, LeaderState::Running | LeaderState::Exited)
            && (matches!(self.group, GroupSignalState::Signalled)
                || matches!(self.leader, LeaderState::Exited))
    }

    pub fn begin_reap(&mut self) -> bool {
        if self.finished
            || !matches!(self.leader, LeaderState::Exited)
            || matches!(self.group, GroupSignalState::Pending)
        {
            return false;
        }
        self.leader = LeaderState::Reaping;
        true
    }

    /// Only after `begin_reap` returned true.
    pub fn observe_reap(&mut self, observation: ReapObservation) {
        debug_assert!(!self.finished && matches!(self.leader, LeaderState::Reaping));
        self.leader = match observation {
            ReapObservation::Reaped => LeaderState::Reaped,
            ReapObservation::NotReady => LeaderState::Exited,
            ReapObservation::Lost => LeaderState::Lost,
        };
    }

    pub fn cleanup_decision(&self, drained: bool, budget_remaining: bool) -> CleanupDecision {
        if !budget_remaining {
            CleanupDecision::Deadline
        } else if matches!(self.leader, LeaderState::Lost) && drained {
            CleanupDecision::LostOwnership
        } else if matches!(self.leader, LeaderState::Reaped) && drained {
            CleanupDecision::Complete
        } else {
            CleanupDecision::Wait
        }
    }

    pub fn finish(&mut self) {
        self.finished = true;
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_ownership_never_signals_and_completes_as_lost() {
        let mut child = ChildLifecycle::new();
        child.observe_exit(false);
        child.lose_ownership();
        assert_eq!(child.observe_action(), ObserveAction::Unavailable);
        assert_eq!(child.signal_action(), SignalAction::Unavailable);
        assert!(!child.needs_termination());
        assert_eq!(
            child.cleanup_decision(true, true),
            CleanupDecision::LostOwnership
        );
    }

    #[test]
    fn the_group_is_signalled_before_the_leader_is_reaped() {
        let mut child = ChildLifecycle::new();
        child.observe_exit(true);
        assert!(
            !child.begin_reap(),
            "an exited leader is not reaped before its group is signalled"
        );
        assert_eq!(child.signal_action(), SignalAction::Signal);
        assert!(child.begin_reap());
        child.observe_reap(ReapObservation::Reaped);
        assert_eq!(
            child.cleanup_decision(true, true),
            CleanupDecision::Complete
        );
        assert_eq!(
            child.cleanup_decision(true, false),
            CleanupDecision::Deadline
        );
    }

    #[test]
    fn a_finished_lifecycle_stays_finished() {
        let mut child = ChildLifecycle::new();
        child.observe_exit(false);
        child.finish();
        assert!(child.is_finished());
        assert_eq!(child.observe_action(), ObserveAction::Unavailable);
        assert_eq!(child.signal_action(), SignalAction::Unavailable);
        assert!(!child.begin_reap());
    }
}
