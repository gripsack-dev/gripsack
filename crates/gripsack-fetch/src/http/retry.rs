//! Pure retry decisions; callers supply the monotonic clock (HttpRetry.tla bridge).
use super::failure::HttpFailureKind;
use std::time::{Duration, Instant};

pub(crate) const OPERATION_TIMEOUT: Duration = Duration::from_secs(600);
const WAIT_LIMIT: Duration = Duration::from_secs(30);
const ATTEMPT_LIMIT: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryStopReason {
    NonRetryable,
    AttemptLimit,
    Deadline,
    WaitBudget,
    ServerCooldown,
    UnknownServerDelay,
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
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    RetryAfter(Duration),
    Stop(RetryStopReason),
}

pub(crate) struct RequestBudget {
    pub started: Instant,
    pub deadline: Instant,
    pub attempts: u8,
    pub waited: Duration,
}
impl RequestBudget {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            deadline: now + OPERATION_TIMEOUT,
            attempts: 0,
            waited: Duration::ZERO,
        }
    }
    pub fn begin(&mut self, now: Instant) -> Result<Duration, RetryStopReason> {
        if now >= self.deadline {
            return Err(RetryStopReason::Deadline);
        }
        if self.attempts >= ATTEMPT_LIMIT {
            return Err(RetryStopReason::AttemptLimit);
        }
        self.attempts += 1;
        Ok(self.deadline.duration_since(now))
    }
    pub fn decide(
        &self,
        now: Instant,
        kind: HttpFailureKind,
        server_wait: Option<Duration>,
    ) -> RetryDecision {
        if !kind.retryable() {
            return RetryDecision::Stop(RetryStopReason::NonRetryable);
        }
        if now >= self.deadline {
            return RetryDecision::Stop(RetryStopReason::Deadline);
        }
        if self.attempts >= ATTEMPT_LIMIT {
            return RetryDecision::Stop(RetryStopReason::AttemptLimit);
        }
        let delay = Duration::from_secs(1 << self.attempts.saturating_sub(1));
        let delay = server_wait.map_or(delay, |server| server.max(delay));
        if delay > WAIT_LIMIT.saturating_sub(self.waited) {
            return RetryDecision::Stop(RetryStopReason::WaitBudget);
        }
        if delay >= self.deadline.saturating_duration_since(now) {
            return RetryDecision::Stop(RetryStopReason::Deadline);
        }
        RetryDecision::RetryAfter(delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_retry_policy_has_fixed_deadline_and_bounded_attempts() {
        let start = Instant::now();
        let mut budget = RequestBudget::new(start);
        let original_deadline = budget.deadline;
        let mut now = start;
        for attempt in 1..=3 {
            budget.begin(now).unwrap();
            assert_eq!(budget.attempts, attempt);
            match budget.decide(now, HttpFailureKind::Status(500), None) {
                RetryDecision::RetryAfter(delay) => {
                    now += delay;
                    budget.waited += delay;
                }
                RetryDecision::Stop(reason) => {
                    assert_eq!(attempt, 3);
                    assert_eq!(reason, RetryStopReason::AttemptLimit);
                }
            }
            assert_eq!(budget.deadline, original_deadline);
        }
        assert_eq!(budget.begin(now), Err(RetryStopReason::AttemptLimit));
    }
    #[test]
    fn terminal_classes_and_upstream_cooldowns_never_get_fast_replays() {
        let now = Instant::now();
        let mut budget = RequestBudget::new(now);
        budget.begin(now).unwrap();
        for kind in [
            HttpFailureKind::Status(401),
            HttpFailureKind::Status(403),
            HttpFailureKind::Status(404),
            HttpFailureKind::Tls,
            HttpFailureKind::InvalidMetadata,
            HttpFailureKind::LoginPage,
        ] {
            assert_eq!(
                budget.decide(now, kind, None),
                RetryDecision::Stop(RetryStopReason::NonRetryable)
            );
        }
        assert_eq!(
            budget.decide(
                now,
                HttpFailureKind::RateLimited(429),
                Some(Duration::from_secs(60))
            ),
            RetryDecision::Stop(RetryStopReason::WaitBudget)
        );
        assert_eq!(
            budget.decide(budget.deadline, HttpFailureKind::Timeout, None),
            RetryDecision::Stop(RetryStopReason::Deadline)
        );
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
