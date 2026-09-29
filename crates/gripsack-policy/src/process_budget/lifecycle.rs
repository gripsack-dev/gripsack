//! Exclusive-child lifecycle decisions; wait/signal outcomes are external facts.
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum LeaderState { Running, Exited, Reaping, Reaped, Lost }
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum GroupSignalState { Pending, Attempted, Signalled }
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum ObserveAction { Observe, Exited, Unavailable }
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum SignalAction { Signal, AlreadySignalled, Unavailable }
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum ReapObservation { Reaped, NotReady, Lost }
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum CleanupDecision { Wait, Complete, Deadline, LostOwnership }

pub struct ChildLifecycle {
    leader: LeaderState,
    group: GroupSignalState,
    observed: bool,
    finished: bool,
}

impl Default for ChildLifecycle {
    fn default() -> (state: Self)
        ensures state.running(), !state.observed(), !state.signal_attempted(), !state.finished(),
    { Self::new() }
}

impl ChildLifecycle {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool {
        (self.leader == LeaderState::Exited || self.leader == LeaderState::Reaping || self.leader == LeaderState::Reaped ==> self.observed)
            && (self.leader == LeaderState::Reaping || self.leader == LeaderState::Reaped ==> self.group != GroupSignalState::Pending)
    }

    pub closed spec fn leader_state(&self) -> LeaderState { self.leader }
    pub closed spec fn group_state(&self) -> GroupSignalState { self.group }
    pub closed spec fn running(&self) -> bool { self.leader == LeaderState::Running }
    pub closed spec fn exited(&self) -> bool { self.leader == LeaderState::Exited }
    pub closed spec fn reaping(&self) -> bool { self.leader == LeaderState::Reaping }
    pub closed spec fn reaped(&self) -> bool { self.leader == LeaderState::Reaped }
    pub closed spec fn lost(&self) -> bool { self.leader == LeaderState::Lost }
    pub closed spec fn owned(&self) -> bool { self.running() || self.exited() }
    pub closed spec fn observed(&self) -> bool { self.observed }
    pub closed spec fn signal_attempted(&self) -> bool { self.group != GroupSignalState::Pending }
    pub closed spec fn signalled(&self) -> bool { self.group == GroupSignalState::Signalled }
    pub closed spec fn finished(&self) -> bool { self.finished }

    pub fn new() -> (state: Self)
        ensures state.running(), !state.observed(), !state.signal_attempted(), !state.finished(),
    { Self { leader: LeaderState::Running, group: GroupSignalState::Pending, observed: false, finished: false } }

    pub fn observe_action(&self) -> (action: ObserveAction)
        ensures action == if self.finished() || !self.owned() { ObserveAction::Unavailable }
            else if self.exited() { ObserveAction::Exited } else { ObserveAction::Observe },
        no_unwind
    {
        if self.finished { ObserveAction::Unavailable }
        else { match self.leader {
            LeaderState::Running => ObserveAction::Observe,
            LeaderState::Exited => ObserveAction::Exited,
            _ => ObserveAction::Unavailable,
        } }
    }

    pub fn observe_exit(&mut self, exited: bool)
        ensures
            final(self).finished() == old(self).finished(),
            final(self).signalled() == old(self).signalled(),
            final(self).signal_attempted() == old(self).signal_attempted(),
            old(self).owned() && !old(self).finished() ==> final(self).observed()
                && (final(self).exited() <==> old(self).exited() || exited)
                && (final(self).running() <==> old(self).running() && !exited),
            !old(self).owned() || old(self).finished() ==> final(self).lost(),
        no_unwind
    {
        if self.finished || !matches!(self.leader, LeaderState::Running | LeaderState::Exited) {
            self.leader = LeaderState::Lost;
        } else {
            self.observed = true;
            if exited { self.leader = LeaderState::Exited; }
        }
    }

    pub fn lose_ownership(&mut self)
        ensures final(self).lost(), final(self).finished() == old(self).finished(),
            final(self).signalled() == old(self).signalled(),
            final(self).signal_attempted() == old(self).signal_attempted(),
        no_unwind
    { self.leader = LeaderState::Lost; }

    pub fn signal_action(&mut self) -> (action: SignalAction)
        ensures
            final(self).leader_state() == old(self).leader_state(), final(self).finished() == old(self).finished(),
            final(self).observed() == old(self).observed(),
            action == if old(self).finished() || !old(self).owned() || !old(self).observed() {
                SignalAction::Unavailable
            } else if old(self).signalled() { SignalAction::AlreadySignalled } else { SignalAction::Signal },
            action == SignalAction::Signal ==> final(self).signal_attempted() && !final(self).signalled(),
            action != SignalAction::Signal ==> final(self).group_state() == old(self).group_state(),
        no_unwind
    {
        if self.finished || !self.observed || !matches!(self.leader, LeaderState::Running | LeaderState::Exited) {
            return SignalAction::Unavailable;
        }
        if matches!(self.group, GroupSignalState::Signalled) { return SignalAction::AlreadySignalled; }
        self.group = GroupSignalState::Attempted;
        SignalAction::Signal
    }

    pub fn signal_succeeded(&mut self)
        requires old(self).owned(), old(self).observed(), old(self).signal_attempted(), !old(self).finished(),
        ensures final(self).signalled(), final(self).leader_state() == old(self).leader_state(),
            final(self).observed() == old(self).observed(), final(self).finished() == old(self).finished(),
        no_unwind
    { self.group = GroupSignalState::Signalled; }

    pub fn needs_termination(&self) -> (needed: bool)
        ensures needed <==> !self.finished() && self.owned() && !self.signalled(),
        no_unwind
    { !self.finished && matches!(self.leader, LeaderState::Running | LeaderState::Exited) && !matches!(self.group, GroupSignalState::Signalled) }

    pub fn should_observe_for_reap(&self) -> (observe: bool)
        ensures observe <==> !self.finished() && self.owned() && (self.signalled() || self.exited()),
        no_unwind
    { !self.finished && matches!(self.leader, LeaderState::Running | LeaderState::Exited)
        && (matches!(self.group, GroupSignalState::Signalled) || matches!(self.leader, LeaderState::Exited)) }

    pub fn begin_reap(&mut self) -> (admitted: bool)
        ensures
            admitted <==> !old(self).finished() && old(self).exited() && old(self).signal_attempted(),
            admitted ==> final(self).reaping(),
            !admitted ==> final(self).leader_state() == old(self).leader_state(),
            final(self).group_state() == old(self).group_state(), final(self).observed() == old(self).observed(),
            final(self).finished() == old(self).finished(),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        if self.finished || !matches!(self.leader, LeaderState::Exited) || matches!(self.group, GroupSignalState::Pending) { return false; }
        self.leader = LeaderState::Reaping;
        true
    }

    pub fn observe_reap(&mut self, observation: ReapObservation)
        requires old(self).reaping(), !old(self).finished(),
        ensures
            final(self).reaped() <==> observation == ReapObservation::Reaped,
            final(self).exited() <==> observation == ReapObservation::NotReady,
            final(self).lost() <==> observation == ReapObservation::Lost,
            final(self).group_state() == old(self).group_state(), final(self).observed() == old(self).observed(),
            final(self).finished() == old(self).finished(),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        self.leader = match observation {
            ReapObservation::Reaped => LeaderState::Reaped,
            ReapObservation::NotReady => LeaderState::Exited,
            ReapObservation::Lost => LeaderState::Lost,
        };
    }

    pub fn cleanup_decision(&self, drained: bool, budget_remaining: bool) -> (decision: CleanupDecision)
        ensures decision == if !budget_remaining { CleanupDecision::Deadline }
            else if self.lost() && drained { CleanupDecision::LostOwnership }
            else if self.reaped() && drained { CleanupDecision::Complete }
            else { CleanupDecision::Wait },
        no_unwind
    {
        if !budget_remaining { CleanupDecision::Deadline }
        else if matches!(self.leader, LeaderState::Lost) && drained { CleanupDecision::LostOwnership }
        else if matches!(self.leader, LeaderState::Reaped) && drained { CleanupDecision::Complete }
        else { CleanupDecision::Wait }
    }

    pub fn finish(&mut self)
        ensures final(self).finished(), final(self).leader_state() == old(self).leader_state(),
            final(self).group_state() == old(self).group_state(), final(self).observed() == old(self).observed(),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        self.finished = true;
    }

    pub fn is_finished(&self) -> (finished: bool) ensures finished == self.finished(), no_unwind { self.finished }
}

}
