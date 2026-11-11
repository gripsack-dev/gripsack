//! Monotone elapsed-time admission with irreversible terminal failure.
//! Callers own the original clock origin/deadline and supply observations.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemainingTime {
    nanoseconds: u128,
}

impl RemainingTime {
    pub fn nanoseconds(self) -> u128 {
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
    pub fn new(allowance_ns: u128) -> Self {
        Self {
            allowance: allowance_ns,
            observed: 0,
            terminal: allowance_ns == 0,
        }
    }

    pub fn observe(&mut self, elapsed_ns: u128) -> Option<RemainingTime> {
        if elapsed_ns >= self.allowance {
            self.terminal = true;
            self.observed = self.allowance;
        } else if elapsed_ns > self.observed {
            self.observed = elapsed_ns;
        }
        if self.terminal {
            None
        } else {
            Some(RemainingTime {
                nanoseconds: self.allowance - self.observed,
            })
        }
    }

    pub fn admit_wait(&mut self, elapsed_ns: u128, wait_ns: u128) -> bool {
        match self.observe(elapsed_ns) {
            None => false,
            Some(remaining) => {
                if wait_ns < remaining.nanoseconds() {
                    true
                } else {
                    self.terminal = true;
                    false
                }
            }
        }
    }

    pub fn stop(&mut self) {
        self.terminal = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rewound_clock_never_returns_spent_time() {
        let mut budget = OperationBudget::new(100);
        assert_eq!(budget.observe(40).unwrap().nanoseconds(), 60);
        assert_eq!(budget.observe(10).unwrap().nanoseconds(), 60);
        assert!(budget.observe(100).is_none());
        assert!(budget.observe(0).is_none(), "expiry is terminal");
    }

    #[test]
    fn a_wait_reaching_the_deadline_is_refused_for_good() {
        let mut budget = OperationBudget::new(100);
        assert!(budget.admit_wait(0, 99));
        assert!(!budget.admit_wait(0, 100));
        assert!(budget.observe(0).is_none());
    }

    #[test]
    fn a_stopped_or_empty_budget_never_revives() {
        let mut budget = OperationBudget::new(100);
        budget.stop();
        assert!(budget.observe(0).is_none());
        assert!(!budget.admit_wait(0, 1));
        assert!(OperationBudget::new(0).observe(0).is_none());
    }
}
