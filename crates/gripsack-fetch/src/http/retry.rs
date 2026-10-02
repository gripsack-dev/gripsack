//! HTTP clock/classification adapter for the shipped retry transition kernel.
use super::failure::HttpFailureKind;
use gripsack_policy::retry_budget as policy;
use std::io;
use std::time::{Duration, Instant};

pub(crate) const OPERATION_TIMEOUT: Duration =
    Duration::from_nanos(policy::OPERATION_NANOSECONDS as u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryStopReason {
    NonRetryable,
    AttemptLimit,
    Deadline,
    WaitBudget,
    ServerCooldown,
    UnknownServerDelay,
    ProtocolOrder,
}
impl std::fmt::Display for RetryStopReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NonRetryable => "non-retryable failure",
            Self::AttemptLimit => "attempt limit reached",
            Self::Deadline => "operation deadline exhausted",
            Self::WaitBudget => "server delay exceeds retry-wait budget",
            Self::ServerCooldown => "host cooldown active; request not attempted",
            Self::UnknownServerDelay => "server cooldown/reset unavailable or invalid; not retried",
            Self::ProtocolOrder => "HTTP attempt protocol order was violated",
        })
    }
}
impl From<policy::RetryRefusal> for RetryStopReason {
    fn from(reason: policy::RetryRefusal) -> Self {
        match reason {
            policy::RetryRefusal::NonRetryable => Self::NonRetryable,
            policy::RetryRefusal::AttemptLimit => Self::AttemptLimit,
            policy::RetryRefusal::Deadline => Self::Deadline,
            policy::RetryRefusal::WaitBudget => Self::WaitBudget,
            policy::RetryRefusal::ServerCooldown => Self::ServerCooldown,
            policy::RetryRefusal::UnknownServerDelay => Self::UnknownServerDelay,
            policy::RetryRefusal::ProtocolOrder => Self::ProtocolOrder,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    RetryAfter(Duration),
    Stop(RetryStopReason),
}

pub(crate) struct RequestBudget {
    started: Instant,
    deadline: Instant,
    state: policy::RetryBudget,
}
impl RequestBudget {
    pub fn new(now: Instant) -> io::Result<Self> {
        let deadline = now.checked_add(OPERATION_TIMEOUT).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "HTTP operation timeout exceeds Instant range",
            )
        })?;
        Ok(Self {
            started: now,
            deadline,
            state: policy::RetryBudget::new(),
        })
    }

    pub fn until(now: Instant, deadline: Instant) -> io::Result<Self> {
        let mut budget = Self::new(now)?;
        budget.deadline = budget.deadline.min(deadline);
        Ok(budget)
    }

    pub fn started(&self) -> Instant {
        self.started
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn attempts(&self) -> u8 {
        self.state.attempt_count()
    }
    pub fn waited(&self) -> Duration {
        Duration::from_nanos(self.state.waited_nanoseconds())
    }

    pub fn begin(&mut self, mut now: Instant) -> Result<Duration, RetryStopReason> {
        loop {
            if now >= self.deadline {
                return Err(self.stop(policy::RetryRefusal::Deadline));
            }
            match self
                .state
                .begin(now.saturating_duration_since(self.started).as_nanos())
            {
                policy::AttemptAdmission::Start { remaining_ns, .. } => {
                    return Ok(Duration::from_nanos(remaining_ns).min(self.deadline - now));
                }
                policy::AttemptAdmission::Wait { nanoseconds } => {
                    if Duration::from_nanos(nanoseconds) >= self.deadline - now {
                        return Err(self.stop(policy::RetryRefusal::Deadline));
                    }
                    std::thread::sleep(Duration::from_nanos(nanoseconds));
                    now = Instant::now();
                }
                policy::AttemptAdmission::Stop(reason) => return Err(reason.into()),
            }
        }
    }

    pub fn decide(
        &mut self,
        now: Instant,
        kind: HttpFailureKind,
        server_wait: Option<Duration>,
    ) -> RetryDecision {
        if now >= self.deadline {
            return RetryDecision::Stop(self.stop(policy::RetryRefusal::Deadline));
        }
        match self.state.decide(
            now.saturating_duration_since(self.started).as_nanos(),
            kind.retryable(),
            server_wait.map(|wait| wait.as_nanos()),
        ) {
            policy::RetryDecision::RetryAfter { nanoseconds } => {
                if Duration::from_nanos(nanoseconds) >= self.deadline - now {
                    return RetryDecision::Stop(self.stop(policy::RetryRefusal::Deadline));
                }
                RetryDecision::RetryAfter(Duration::from_nanos(nanoseconds))
            }
            policy::RetryDecision::Stop(reason) => RetryDecision::Stop(reason.into()),
        }
    }

    pub fn complete(&mut self, now: Instant) -> Result<(), RetryStopReason> {
        if now >= self.deadline {
            return Err(self.stop(policy::RetryRefusal::Deadline));
        }
        self.state
            .complete(now.saturating_duration_since(self.started).as_nanos())
            .map_err(Into::into)
    }

    pub fn stop(&mut self, reason: policy::RetryRefusal) -> RetryStopReason {
        self.state.stop(reason).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_deadline_bounds_attempt_completion_and_retry_wait() {
        let now = Instant::now();
        let end = now + Duration::from_millis(1);
        let mut budget = RequestBudget::until(now, end).unwrap();
        assert_eq!(budget.begin(now), Ok(Duration::from_millis(1)));
        assert_eq!(
            budget.decide(
                now,
                HttpFailureKind::Status(503),
                Some(Duration::from_secs(1))
            ),
            RetryDecision::Stop(RetryStopReason::Deadline),
        );
        let mut budget = RequestBudget::until(now, end).unwrap();
        budget.begin(now).unwrap();
        assert_eq!(budget.complete(end), Err(RetryStopReason::Deadline));
        let mut expired = RequestBudget::until(now, now).unwrap();
        assert_eq!(expired.begin(now), Err(RetryStopReason::Deadline));
        let extended = RequestBudget::until(now, now + OPERATION_TIMEOUT * 2).unwrap();
        assert_eq!(extended.deadline(), now + OPERATION_TIMEOUT);
    }

    #[test]
    fn an_expired_clock_observation_cannot_admit_an_attempt() {
        let started = Instant::now();
        let mut budget = RequestBudget::new(started).unwrap();
        assert_eq!(
            budget.begin(started + OPERATION_TIMEOUT),
            Err(RetryStopReason::Deadline),
            "expired_http_attempt_admitted",
        );
    }
    #[test]
    fn real_retry_policy_has_fixed_deadline_and_bounded_attempts() {
        let start = Instant::now();
        let mut budget = RequestBudget::new(start).unwrap();
        let original_deadline = budget.deadline();
        let mut now = start;
        for attempt in 1..=3 {
            budget.begin(now).unwrap();
            assert_eq!(budget.attempts(), attempt);
            match budget.decide(now, HttpFailureKind::Status(500), None) {
                RetryDecision::RetryAfter(delay) => {
                    now += delay;
                }
                RetryDecision::Stop(reason) => {
                    assert_eq!(attempt, 3);
                    assert_eq!(reason, RetryStopReason::AttemptLimit);
                }
            }
            assert_eq!(budget.deadline(), original_deadline);
        }
        assert_eq!(budget.begin(now), Err(RetryStopReason::AttemptLimit));
    }
    #[test]
    fn terminal_classes_and_upstream_cooldowns_never_get_fast_replays() {
        let now = Instant::now();
        for kind in [
            HttpFailureKind::Status(401),
            HttpFailureKind::Status(403),
            HttpFailureKind::Status(404),
            HttpFailureKind::Tls,
            HttpFailureKind::InvalidMetadata,
            HttpFailureKind::LoginPage,
        ] {
            let mut budget = RequestBudget::new(now).unwrap();
            budget.begin(now).unwrap();
            assert_eq!(
                budget.decide(now, kind, None),
                RetryDecision::Stop(RetryStopReason::NonRetryable),
                "nonretryable_http_failure_replayed",
            );
            assert_eq!(budget.complete(now), Err(RetryStopReason::NonRetryable));
            assert_eq!(budget.begin(now), Err(RetryStopReason::NonRetryable));
        }
        let mut budget = RequestBudget::new(now).unwrap();
        budget.begin(now).unwrap();
        assert_eq!(
            budget.decide(
                now,
                HttpFailureKind::RateLimited(429),
                Some(Duration::from_secs(60))
            ),
            RetryDecision::Stop(RetryStopReason::WaitBudget)
        );
        let mut budget = RequestBudget::new(now).unwrap();
        budget.begin(now).unwrap();
        assert_eq!(
            budget.decide(budget.deadline(), HttpFailureKind::Timeout, None),
            RetryDecision::Stop(RetryStopReason::Deadline)
        );
        let mut budget = RequestBudget::new(now).unwrap();
        budget.begin(now).unwrap();
        assert_eq!(
            budget.decide(
                now,
                HttpFailureKind::Status(503),
                Some(Duration::from_secs(4))
            ),
            RetryDecision::RetryAfter(Duration::from_secs(4))
        );
    }
}
