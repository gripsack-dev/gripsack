//! One HTTP operation: legal attempt order, cumulative waits and terminal precedence.
//! Response classification, clocks and transport effects are separate adapters.
use super::operation_budget::OperationBudget;
use vstd::prelude::*;

verus! {

pub const OPERATION_NANOSECONDS: u128 = 600000000000;
pub const RETRY_WAIT_NANOSECONDS: u64 = 30000000000;
pub const ATTEMPT_LIMIT: u8 = 3;
pub const FIRST_RETRY_NANOSECONDS: u64 = 1000000000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum RetryRefusal { NonRetryable, AttemptLimit, Deadline, WaitBudget, ServerCooldown, UnknownServerDelay, ProtocolOrder }

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
enum Phase { Ready, InFlight, Finished, Stopped(RetryRefusal) }

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum AttemptAdmission {
    Start { ordinal: u8, remaining_ns: u64 },
    Wait { nanoseconds: u64 },
    Stop(RetryRefusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum RetryDecision { RetryAfter { nanoseconds: u64 }, Stop(RetryRefusal) }

#[derive(Debug)]
pub struct RetryBudget {
    time: OperationBudget,
    attempts: u8,
    waited_ns: u64,
    ready_after: u128,
    phase: Phase,
}

impl Default for RetryBudget {
    fn default() -> (budget: Self)
        ensures budget.attempts() == 0, budget.waited() == 0, budget.ready_after() == 0,
            !budget.in_flight(), !budget.finished(), budget.stopped().is_none(),
    {
        Self::new()
    }
}

impl RetryBudget {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool {
        self.time.allowance() == OPERATION_NANOSECONDS
            && self.attempts <= ATTEMPT_LIMIT
            && self.waited_ns <= RETRY_WAIT_NANOSECONDS
            && self.ready_after < OPERATION_NANOSECONDS
            && (self.phase == Phase::InFlight ==> self.attempts > 0)
    }

    pub closed spec fn attempts(&self) -> u8 { self.attempts }
    pub closed spec fn waited(&self) -> u64 { self.waited_ns }
    pub closed spec fn ready_after(&self) -> u128 { self.ready_after }
    pub closed spec fn observed(&self) -> u128 { self.time.observed() }
    pub closed spec fn in_flight(&self) -> bool { self.phase == Phase::InFlight }
    pub closed spec fn finished(&self) -> bool { self.phase == Phase::Finished }
    pub closed spec fn stopped(&self) -> Option<RetryRefusal> {
        match self.phase { Phase::Stopped(reason) => Some(reason), _ => None }
    }

    pub closed spec fn projected_elapsed(&self, elapsed_ns: u128) -> u128 {
        if elapsed_ns >= OPERATION_NANOSECONDS { OPERATION_NANOSECONDS }
        else if elapsed_ns > self.time.observed() { elapsed_ns } else { self.time.observed() }
    }

    pub closed spec fn requested_delay(&self, server_wait_ns: Option<u128>) -> u128 {
        let backoff = if self.attempts == 1 { FIRST_RETRY_NANOSECONDS as u128 }
            else { (2 * (FIRST_RETRY_NANOSECONDS as u128)) as u128 };
        match server_wait_ns { Some(server) if server > backoff => server, _ => backoff }
    }

    pub closed spec fn expected_begin(&self, elapsed_ns: u128) -> AttemptAdmission {
        if self.stopped().is_some() { AttemptAdmission::Stop(self.stopped().unwrap()) }
        else if self.phase != Phase::Ready { AttemptAdmission::Stop(RetryRefusal::ProtocolOrder) }
        else if self.time.terminal() || elapsed_ns >= OPERATION_NANOSECONDS { AttemptAdmission::Stop(RetryRefusal::Deadline) }
        else if self.attempts >= ATTEMPT_LIMIT { AttemptAdmission::Stop(RetryRefusal::AttemptLimit) }
        else if self.projected_elapsed(elapsed_ns) < self.ready_after {
            AttemptAdmission::Wait { nanoseconds: (self.ready_after - self.projected_elapsed(elapsed_ns)) as u64 }
        } else {
            AttemptAdmission::Start { ordinal: (self.attempts + 1) as u8,
                remaining_ns: (OPERATION_NANOSECONDS - self.projected_elapsed(elapsed_ns)) as u64 }
        }
    }

    pub closed spec fn expected_retry(&self, elapsed_ns: u128, retryable: bool, server_wait_ns: Option<u128>) -> RetryDecision {
        let delay = self.requested_delay(server_wait_ns);
        if self.stopped().is_some() { RetryDecision::Stop(self.stopped().unwrap()) }
        else if !self.in_flight() { RetryDecision::Stop(RetryRefusal::ProtocolOrder) }
        else if !retryable { RetryDecision::Stop(RetryRefusal::NonRetryable) }
        else if self.time.terminal() || elapsed_ns >= OPERATION_NANOSECONDS { RetryDecision::Stop(RetryRefusal::Deadline) }
        else if self.attempts >= ATTEMPT_LIMIT { RetryDecision::Stop(RetryRefusal::AttemptLimit) }
        else if delay > RETRY_WAIT_NANOSECONDS - self.waited_ns { RetryDecision::Stop(RetryRefusal::WaitBudget) }
        else if delay >= OPERATION_NANOSECONDS - self.projected_elapsed(elapsed_ns) { RetryDecision::Stop(RetryRefusal::Deadline) }
        else { RetryDecision::RetryAfter { nanoseconds: delay as u64 } }
    }

    pub closed spec fn expected_completion(&self, elapsed_ns: u128) -> Result<(), RetryRefusal> {
        if self.stopped().is_some() { Err(self.stopped().unwrap()) }
        else if !self.in_flight() { Err(RetryRefusal::ProtocolOrder) }
        else if self.time.terminal() || elapsed_ns >= OPERATION_NANOSECONDS { Err(RetryRefusal::Deadline) }
        else { Ok(()) }
    }

    pub fn new() -> (budget: Self)
        ensures budget.attempts() == 0, budget.waited() == 0, budget.ready_after() == 0,
            !budget.in_flight(), !budget.finished(), budget.stopped().is_none(),
    {
        Self { time: OperationBudget::new(OPERATION_NANOSECONDS), attempts: 0,
            waited_ns: 0, ready_after: 0, phase: Phase::Ready }
    }

    pub fn attempt_count(&self) -> (count: u8)
        ensures count == self.attempts(), count <= ATTEMPT_LIMIT,
    {
        proof { use_type_invariant(&*self); }
        self.attempts
    }

    pub fn waited_nanoseconds(&self) -> (waited: u64)
        ensures waited == self.waited(), waited <= RETRY_WAIT_NANOSECONDS,
    {
        proof { use_type_invariant(&*self); }
        self.waited_ns
    }

    pub fn stop(&mut self, reason: RetryRefusal) -> (selected: RetryRefusal)
        ensures
            final(self).attempts() == old(self).attempts(),
            final(self).waited() == old(self).waited(),
            final(self).ready_after() == old(self).ready_after(),
            final(self).observed() == old(self).observed(),
            selected == match old(self).stopped() { Some(previous) => previous, None => reason },
            final(self).stopped() == Some(selected),
            !final(self).in_flight(), !final(self).finished(),
    {
        proof { use_type_invariant(&*self); }
        let selected = match self.phase { Phase::Stopped(previous) => previous, _ => reason };
        self.time.stop();
        self.phase = Phase::Stopped(selected);
        selected
    }

    pub fn begin(&mut self, elapsed_ns: u128) -> (admission: AttemptAdmission)
        ensures
            admission == old(self).expected_begin(elapsed_ns),
            final(self).waited() == old(self).waited(),
            final(self).ready_after() == old(self).ready_after(),
            final(self).observed() >= old(self).observed(),
            match admission {
                AttemptAdmission::Start { ordinal, remaining_ns } =>
                    !old(self).in_flight() && !old(self).finished() && old(self).stopped().is_none()
                    && final(self).in_flight() && final(self).stopped().is_none()
                    && ordinal == old(self).attempts() + 1 && ordinal == final(self).attempts()
                    && 1 <= ordinal <= ATTEMPT_LIMIT
                    && final(self).observed() >= old(self).ready_after()
                    && 0 < remaining_ns && final(self).observed() + remaining_ns == OPERATION_NANOSECONDS,
                AttemptAdmission::Wait { nanoseconds } =>
                    !final(self).in_flight() && final(self).stopped().is_none()
                    && final(self).attempts() == old(self).attempts()
                    && nanoseconds > 0 && final(self).observed() + nanoseconds == old(self).ready_after(),
                AttemptAdmission::Stop(reason) => final(self).stopped() == Some(reason)
                    && final(self).attempts() == old(self).attempts(),
            },
            old(self).stopped().is_some() ==> admission == AttemptAdmission::Stop(old(self).stopped().unwrap()),
    {
        proof { use_type_invariant(&*self); }
        if let Phase::Stopped(reason) = self.phase { return AttemptAdmission::Stop(reason); }
        if self.phase != Phase::Ready { return AttemptAdmission::Stop(self.stop(RetryRefusal::ProtocolOrder)); }
        let remaining = match self.time.observe(elapsed_ns) {
            None => return AttemptAdmission::Stop(self.stop(RetryRefusal::Deadline)),
            Some(remaining) => remaining.nanoseconds(),
        };
        if self.attempts >= ATTEMPT_LIMIT { return AttemptAdmission::Stop(self.stop(RetryRefusal::AttemptLimit)); }
        let observed = OPERATION_NANOSECONDS - remaining;
        if observed < self.ready_after {
            return AttemptAdmission::Wait { nanoseconds: (self.ready_after - observed) as u64 };
        }
        self.attempts += 1;
        self.phase = Phase::InFlight;
        AttemptAdmission::Start { ordinal: self.attempts, remaining_ns: remaining as u64 }
    }

    pub fn decide(&mut self, elapsed_ns: u128, retryable: bool, server_wait_ns: Option<u128>) -> (decision: RetryDecision)
        ensures
            decision == old(self).expected_retry(elapsed_ns, retryable, server_wait_ns),
            final(self).attempts() == old(self).attempts(),
            final(self).observed() >= old(self).observed(),
            final(self).waited() >= old(self).waited(),
            match decision {
                RetryDecision::RetryAfter { nanoseconds } =>
                    old(self).in_flight() && old(self).stopped().is_none() && retryable
                    && old(self).attempts() < ATTEMPT_LIMIT
                    && !final(self).in_flight() && final(self).stopped().is_none()
                    && 0 < nanoseconds <= RETRY_WAIT_NANOSECONDS
                    && final(self).waited() == old(self).waited() + nanoseconds
                    && final(self).ready_after() == final(self).observed() + nanoseconds
                    && final(self).ready_after() < OPERATION_NANOSECONDS
                    && (server_wait_ns.is_some() ==> nanoseconds >= server_wait_ns.unwrap()),
                RetryDecision::Stop(reason) => final(self).stopped() == Some(reason)
                    && final(self).waited() == old(self).waited(),
            },
            old(self).stopped().is_some() ==> decision == RetryDecision::Stop(old(self).stopped().unwrap()),
            old(self).in_flight() && !retryable ==> decision == RetryDecision::Stop(RetryRefusal::NonRetryable),
    {
        proof { use_type_invariant(&*self); }
        if let Phase::Stopped(reason) = self.phase { return RetryDecision::Stop(reason); }
        if self.phase != Phase::InFlight { return RetryDecision::Stop(self.stop(RetryRefusal::ProtocolOrder)); }
        if !retryable { return RetryDecision::Stop(self.stop(RetryRefusal::NonRetryable)); }
        let remaining = match self.time.observe(elapsed_ns) {
            None => return RetryDecision::Stop(self.stop(RetryRefusal::Deadline)),
            Some(remaining) => remaining.nanoseconds(),
        };
        if self.attempts >= ATTEMPT_LIMIT { return RetryDecision::Stop(self.stop(RetryRefusal::AttemptLimit)); }
        let backoff = if self.attempts == 1 { FIRST_RETRY_NANOSECONDS } else { 2 * FIRST_RETRY_NANOSECONDS };
        let delay = match server_wait_ns { Some(server) if server > backoff as u128 => server, _ => backoff as u128 };
        if delay > (RETRY_WAIT_NANOSECONDS - self.waited_ns) as u128 {
            return RetryDecision::Stop(self.stop(RetryRefusal::WaitBudget));
        }
        if !self.time.admit_wait(elapsed_ns, delay) { return RetryDecision::Stop(self.stop(RetryRefusal::Deadline)); }
        let observed = OPERATION_NANOSECONDS - remaining;
        self.ready_after = observed + delay;
        self.waited_ns += delay as u64;
        self.phase = Phase::Ready;
        RetryDecision::RetryAfter { nanoseconds: delay as u64 }
    }

    pub fn complete(&mut self, elapsed_ns: u128) -> (result: Result<(), RetryRefusal>)
        ensures
            result == old(self).expected_completion(elapsed_ns),
            final(self).attempts() == old(self).attempts(), final(self).waited() == old(self).waited(),
            final(self).observed() >= old(self).observed(),
            result.is_ok() ==> old(self).in_flight() && old(self).stopped().is_none()
                && final(self).finished() && final(self).observed() < OPERATION_NANOSECONDS,
            result.is_err() ==> final(self).stopped() == Some(result.unwrap_err()),
            old(self).stopped().is_some() ==> result == Err(old(self).stopped().unwrap()),
    {
        proof { use_type_invariant(&*self); }
        if let Phase::Stopped(reason) = self.phase { return Err(reason); }
        if self.phase != Phase::InFlight { return Err(self.stop(RetryRefusal::ProtocolOrder)); }
        if self.time.observe(elapsed_ns).is_none() { return Err(self.stop(RetryRefusal::Deadline)); }
        self.time.stop();
        self.phase = Phase::Finished;
        Ok(())
    }
}

}
