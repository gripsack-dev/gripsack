//! Distinct cumulative stdout/stderr budgets with sticky overflow refusal.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdoutByteLimit {
    bytes: u64,
}
impl StdoutByteLimit {
    pub fn new(bytes: u64) -> Self {
        Self { bytes }
    }
    pub fn bytes(self) -> u64 {
        self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StderrByteLimit {
    bytes: u64,
}
impl StderrByteLimit {
    pub fn new(bytes: u64) -> Self {
        Self { bytes }
    }
    pub fn bytes(self) -> u64 {
        self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedStderrLimit {
    bytes: usize,
}
impl RetainedStderrLimit {
    pub fn new(bytes: usize) -> Self {
        Self { bytes }
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
}

struct ObservedBytes {
    total: u64,
    overflowed: bool,
}
impl ObservedBytes {
    fn zero() -> Self {
        Self {
            total: 0,
            overflowed: false,
        }
    }

    fn record(&mut self, bytes: u64, limit: u64) -> bool {
        if self.overflowed || bytes > u64::MAX - self.total {
            self.total = u64::MAX;
            self.overflowed = true;
            return false;
        }
        self.total += bytes;
        self.total <= limit
    }
}

pub struct StdoutBudget {
    limit: StdoutByteLimit,
    observed: ObservedBytes,
}
impl StdoutBudget {
    pub fn new(limit: StdoutByteLimit) -> Self {
        Self {
            limit,
            observed: ObservedBytes::zero(),
        }
    }
    pub fn observe(&mut self, bytes: u64) -> bool {
        self.observed.record(bytes, self.limit.bytes)
    }
}

pub struct StderrBudget {
    limit: StderrByteLimit,
    observed: ObservedBytes,
}
impl StderrBudget {
    pub fn new(limit: StderrByteLimit) -> Self {
        Self {
            limit,
            observed: ObservedBytes::zero(),
        }
    }
    pub fn observe(&mut self, bytes: u64) -> bool {
        self.observed.record(bytes, self.limit.bytes)
    }
    pub fn truncated(&self, retained: RetainedStderrLimit) -> bool {
        self.observed.overflowed || (self.observed.total as u128) > (retained.bytes as u128)
    }
}
