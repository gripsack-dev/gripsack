//! Clock observations for the verified operation-budget state machine.
//! The original end is immutable; observations may only reduce admission time.
use gripsack_policy::operation_budget::{OperationBudget, RemainingTime};
use std::time::{Duration, Instant};

const NANOS_PER_SECOND: u128 = 1_000_000_000;

#[derive(Debug)]
pub struct OperationDeadline {
    started: Instant,
    end: Instant,
    budget: OperationBudget,
}

impl OperationDeadline {
    /// Admit the remaining portion of an already selected absolute deadline.
    /// This never starts a fresh timeout for a later phase of an operation.
    pub fn at(end: Instant) -> Self {
        let started = Instant::now();
        Self {
            started,
            end,
            budget: OperationBudget::new(end.saturating_duration_since(started).as_nanos()),
        }
    }

    /// The immutable deadline passed to timed OS/library waits.
    pub fn instant(&self) -> Instant {
        self.end
    }

    pub fn remaining(&mut self) -> Option<Duration> {
        self.remaining_at(Instant::now())
    }

    fn remaining_at(&mut self, now: Instant) -> Option<Duration> {
        self.budget
            .observe(now.saturating_duration_since(self.started).as_nanos())
            .map(duration)
    }

    /// A wait ending exactly at the deadline cannot authorize more work.
    /// Refusal is terminal even if a later clock observation goes backwards.
    pub fn admit_wait(&mut self, wait: Duration) -> bool {
        self.budget.admit_wait(
            Instant::now()
                .saturating_duration_since(self.started)
                .as_nanos(),
            wait.as_nanos(),
        )
    }

    pub fn stop(&mut self) {
        self.budget.stop();
    }
}

fn duration(remaining: RemainingTime) -> Duration {
    let nanos = remaining.nanoseconds();
    // The allowance originated in Duration::as_nanos; the verified kernel
    // cannot enlarge it. Keep the conversion checked at the std/Verus seam.
    let seconds = u64::try_from(nanos / NANOS_PER_SECOND)
        .expect("remaining operation time exceeded its admitted Duration");
    Duration::new(seconds, (nanos % NANOS_PER_SECOND) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_observations_cannot_extend_or_revive_admission() {
        let start = Instant::now();
        let mut deadline = OperationDeadline {
            started: start,
            end: start + Duration::from_secs(10),
            budget: OperationBudget::new(Duration::from_secs(10).as_nanos()),
        };
        assert_eq!(
            deadline.remaining_at(start + Duration::from_secs(4)),
            Some(Duration::from_secs(6))
        );
        assert_eq!(
            deadline.remaining_at(start + Duration::from_secs(2)),
            Some(Duration::from_secs(6))
        );
        assert_eq!(deadline.remaining_at(start + Duration::from_secs(10)), None);
        assert_eq!(deadline.remaining_at(start), None);
    }

    #[test]
    fn an_unfillable_wait_stops_later_admission() {
        let mut deadline = OperationDeadline::at(Instant::now() + Duration::from_secs(1));
        assert!(!deadline.admit_wait(Duration::from_secs(1)));
        assert_eq!(deadline.remaining(), None);
    }
}
