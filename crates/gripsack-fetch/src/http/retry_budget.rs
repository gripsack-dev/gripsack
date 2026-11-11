//! One HTTP operation: legal attempt order, cumulative waits and terminal precedence.
//! Response classification, clocks and transport effects are separate adapters.
use gripsack_host::process::operation_budget::OperationBudget;

pub const OPERATION_NANOSECONDS: u128 = 600000000000;
pub const RETRY_WAIT_NANOSECONDS: u64 = 30000000000;
pub const ATTEMPT_LIMIT: u8 = 3;
pub const FIRST_RETRY_NANOSECONDS: u64 = 1000000000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryRefusal {
    NonRetryable,
    AttemptLimit,
    Deadline,
    WaitBudget,
    ServerCooldown,
    UnknownServerDelay,
    ProtocolOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready,
    InFlight,
    Finished,
    Stopped(RetryRefusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptAdmission {
    Start { ordinal: u8, remaining_ns: u64 },
    Wait { nanoseconds: u64 },
    Stop(RetryRefusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    RetryAfter { nanoseconds: u64 },
    Stop(RetryRefusal),
}

#[derive(Debug)]
pub struct RetryBudget {
    time: OperationBudget,
    attempts: u8,
    waited_ns: u64,
    ready_after: u128,
    phase: Phase,
}

impl Default for RetryBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl RetryBudget {
    pub fn new() -> Self {
        Self {
            time: OperationBudget::new(OPERATION_NANOSECONDS),
            attempts: 0,
            waited_ns: 0,
            ready_after: 0,
            phase: Phase::Ready,
        }
    }

    pub fn attempt_count(&self) -> u8 {
        self.attempts
    }

    pub fn waited_nanoseconds(&self) -> u64 {
        self.waited_ns
    }

    pub fn stop(&mut self, reason: RetryRefusal) -> RetryRefusal {
        let selected = match self.phase {
            Phase::Stopped(previous) => previous,
            _ => reason,
        };
        self.time.stop();
        self.phase = Phase::Stopped(selected);
        selected
    }

    pub fn begin(&mut self, elapsed_ns: u128) -> AttemptAdmission {
        if let Phase::Stopped(reason) = self.phase {
            return AttemptAdmission::Stop(reason);
        }
        if self.phase != Phase::Ready {
            return AttemptAdmission::Stop(self.stop(RetryRefusal::ProtocolOrder));
        }
        let remaining = match self.time.observe(elapsed_ns) {
            None => return AttemptAdmission::Stop(self.stop(RetryRefusal::Deadline)),
            Some(remaining) => remaining.nanoseconds(),
        };
        if self.attempts >= ATTEMPT_LIMIT {
            return AttemptAdmission::Stop(self.stop(RetryRefusal::AttemptLimit));
        }
        let observed = OPERATION_NANOSECONDS - remaining;
        if observed < self.ready_after {
            return AttemptAdmission::Wait {
                nanoseconds: (self.ready_after - observed) as u64,
            };
        }
        self.attempts += 1;
        self.phase = Phase::InFlight;
        AttemptAdmission::Start {
            ordinal: self.attempts,
            remaining_ns: remaining as u64,
        }
    }

    pub fn decide(
        &mut self,
        elapsed_ns: u128,
        retryable: bool,
        server_wait_ns: Option<u128>,
    ) -> RetryDecision {
        if let Phase::Stopped(reason) = self.phase {
            return RetryDecision::Stop(reason);
        }
        if self.phase != Phase::InFlight {
            return RetryDecision::Stop(self.stop(RetryRefusal::ProtocolOrder));
        }
        if !retryable {
            return RetryDecision::Stop(self.stop(RetryRefusal::NonRetryable));
        }
        let remaining = match self.time.observe(elapsed_ns) {
            None => return RetryDecision::Stop(self.stop(RetryRefusal::Deadline)),
            Some(remaining) => remaining.nanoseconds(),
        };
        if self.attempts >= ATTEMPT_LIMIT {
            return RetryDecision::Stop(self.stop(RetryRefusal::AttemptLimit));
        }
        let backoff = if self.attempts == 1 {
            FIRST_RETRY_NANOSECONDS
        } else {
            2 * FIRST_RETRY_NANOSECONDS
        };
        let delay = match server_wait_ns {
            Some(server) if server > backoff as u128 => server,
            _ => backoff as u128,
        };
        if delay > (RETRY_WAIT_NANOSECONDS - self.waited_ns) as u128 {
            return RetryDecision::Stop(self.stop(RetryRefusal::WaitBudget));
        }
        if !self.time.admit_wait(elapsed_ns, delay) {
            return RetryDecision::Stop(self.stop(RetryRefusal::Deadline));
        }
        let observed = OPERATION_NANOSECONDS - remaining;
        self.ready_after = observed + delay;
        self.waited_ns += delay as u64;
        self.phase = Phase::Ready;
        RetryDecision::RetryAfter {
            nanoseconds: delay as u64,
        }
    }

    pub fn complete(&mut self, elapsed_ns: u128) -> Result<(), RetryRefusal> {
        if let Phase::Stopped(reason) = self.phase {
            return Err(reason);
        }
        if self.phase != Phase::InFlight {
            return Err(self.stop(RetryRefusal::ProtocolOrder));
        }
        if self.time.observe(elapsed_ns).is_none() {
            return Err(self.stop(RetryRefusal::Deadline));
        }
        self.time.stop();
        self.phase = Phase::Finished;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u128 = FIRST_RETRY_NANOSECONDS as u128;

    #[test]
    fn attempts_never_overlap() {
        let mut budget = RetryBudget::new();
        assert!(matches!(
            budget.begin(0),
            AttemptAdmission::Start { ordinal: 1, .. }
        ));
        assert_eq!(
            budget.begin(0),
            AttemptAdmission::Stop(RetryRefusal::ProtocolOrder)
        );
        assert_eq!(
            budget.begin(0),
            AttemptAdmission::Stop(RetryRefusal::ProtocolOrder),
            "sticky"
        );
    }

    #[test]
    fn backoff_doubles_and_waits_accumulate_up_to_the_attempt_limit() {
        let mut budget = RetryBudget::new();
        assert!(matches!(
            budget.begin(0),
            AttemptAdmission::Start { ordinal: 1, .. }
        ));
        assert_eq!(
            budget.decide(0, true, None),
            RetryDecision::RetryAfter {
                nanoseconds: SECOND as u64
            }
        );
        assert_eq!(
            budget.begin(SECOND / 2),
            AttemptAdmission::Wait {
                nanoseconds: (SECOND / 2) as u64
            }
        );
        assert!(matches!(
            budget.begin(SECOND),
            AttemptAdmission::Start { ordinal: 2, .. }
        ));
        assert_eq!(
            budget.decide(SECOND, true, None),
            RetryDecision::RetryAfter {
                nanoseconds: 2 * SECOND as u64
            }
        );
        assert_eq!(budget.waited_nanoseconds(), 3 * SECOND as u64);
        assert!(matches!(
            budget.begin(3 * SECOND),
            AttemptAdmission::Start { ordinal: 3, .. }
        ));
        assert_eq!(
            budget.decide(3 * SECOND, true, None),
            RetryDecision::Stop(RetryRefusal::AttemptLimit)
        );
        assert_eq!(budget.attempt_count(), ATTEMPT_LIMIT);
    }

    #[test]
    fn server_waits_count_against_the_cumulative_wait_budget() {
        let mut budget = RetryBudget::new();
        let twenty = 20 * SECOND;
        budget.begin(0);
        assert_eq!(
            budget.decide(0, true, Some(twenty)),
            RetryDecision::RetryAfter {
                nanoseconds: twenty as u64
            }
        );
        assert!(matches!(
            budget.begin(twenty),
            AttemptAdmission::Start { ordinal: 2, .. }
        ));
        assert_eq!(
            budget.decide(twenty, true, Some(twenty)),
            RetryDecision::Stop(RetryRefusal::WaitBudget)
        );
    }

    #[test]
    fn a_non_retryable_failure_outranks_the_deadline() {
        let mut budget = RetryBudget::new();
        budget.begin(0);
        assert_eq!(
            budget.decide(OPERATION_NANOSECONDS, false, None),
            RetryDecision::Stop(RetryRefusal::NonRetryable)
        );
    }

    #[test]
    fn a_completion_after_the_deadline_is_refused() {
        let mut budget = RetryBudget::new();
        budget.begin(0);
        assert_eq!(
            budget.complete(OPERATION_NANOSECONDS),
            Err(RetryRefusal::Deadline)
        );
        let mut budget = RetryBudget::new();
        budget.begin(0);
        assert_eq!(budget.complete(SECOND), Ok(()));
    }
}
