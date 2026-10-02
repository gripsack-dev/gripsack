//! Monotone elapsed-time admission with irreversible terminal failure.
//! Callers own the original clock origin/deadline and supply observations.
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct RemainingTime { nanoseconds: u128 }

impl RemainingTime {
    #[verifier::type_invariant]
    spec fn positive(&self) -> bool { self.nanoseconds > 0 }

    pub closed spec fn view(self) -> u128 { self.nanoseconds }

    pub fn nanoseconds(self) -> (nanoseconds: u128)
        ensures nanoseconds == self.view(), nanoseconds > 0,
        no_unwind
    {
        proof { use_type_invariant(&self); }
        self.nanoseconds
    }
}

#[derive(Debug)]
pub struct OperationBudget {
    allowance: u128,
    observed: u128,
    terminal: bool,
}

impl OperationBudget {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool {
        self.observed <= self.allowance && (self.observed == self.allowance ==> self.terminal)
    }

    pub closed spec fn allowance(&self) -> u128 { self.allowance }
    pub closed spec fn observed(&self) -> u128 { self.observed }
    pub closed spec fn terminal(&self) -> bool { self.terminal }

    pub fn new(allowance_ns: u128) -> (budget: Self)
        ensures budget.allowance() == allowance_ns, budget.observed() == 0,
            budget.terminal() <==> allowance_ns == 0,
    {
        Self { allowance: allowance_ns, observed: 0, terminal: allowance_ns == 0 }
    }

    pub fn observe(&mut self, elapsed_ns: u128) -> (remaining: Option<RemainingTime>)
        ensures
            final(self).allowance() == old(self).allowance(),
            final(self).observed() >= old(self).observed(),
            final(self).observed() == if elapsed_ns >= old(self).allowance() { old(self).allowance() }
                else if elapsed_ns > old(self).observed() { elapsed_ns } else { old(self).observed() },
            final(self).terminal() <==> old(self).terminal() || elapsed_ns >= old(self).allowance(),
            remaining.is_some() <==> !final(self).terminal(),
            remaining.is_some() ==> remaining.unwrap().view() == final(self).allowance() - final(self).observed(),
            remaining.is_some() ==> final(self).observed() < final(self).allowance(),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        if elapsed_ns >= self.allowance {
            self.terminal = true;
            self.observed = self.allowance;
        } else if elapsed_ns > self.observed {
            self.observed = elapsed_ns;
        }
        if self.terminal { None }
        else { Some(RemainingTime { nanoseconds: self.allowance - self.observed }) }
    }

    pub fn admit_wait(&mut self, elapsed_ns: u128, wait_ns: u128) -> (admitted: bool)
        ensures
            final(self).allowance() == old(self).allowance(),
            final(self).observed() >= old(self).observed(),
            final(self).observed() == if elapsed_ns >= old(self).allowance() { old(self).allowance() }
                else if elapsed_ns > old(self).observed() { elapsed_ns } else { old(self).observed() },
            admitted ==> !old(self).terminal() && !final(self).terminal(),
            admitted ==> final(self).observed() + wait_ns < old(self).allowance(),
            !admitted ==> final(self).terminal(),
            admitted <==> !old(self).terminal() && elapsed_ns < old(self).allowance()
                && wait_ns < old(self).allowance() - if elapsed_ns > old(self).observed() { elapsed_ns } else { old(self).observed() },
        no_unwind
    {
        match self.observe(elapsed_ns) {
            None => false,
            Some(remaining) => {
                if wait_ns < remaining.nanoseconds() { true }
                else { self.terminal = true; false }
            }
        }
    }

    pub fn stop(&mut self)
        ensures final(self).allowance() == old(self).allowance(),
            final(self).observed() == old(self).observed(), final(self).terminal(),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        self.terminal = true;
    }
}

}
