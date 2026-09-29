//! Distinct cumulative stdout/stderr budgets with sticky overflow refusal.
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct StdoutByteLimit { bytes: u64 }
impl StdoutByteLimit {
    pub closed spec fn view(self) -> u64 { self.bytes }
    pub fn new(bytes: u64) -> (limit: Self) ensures limit.view() == bytes { Self { bytes } }
    pub fn bytes(self) -> (bytes: u64) ensures bytes == self.view() { self.bytes }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct StderrByteLimit { bytes: u64 }
impl StderrByteLimit {
    pub closed spec fn view(self) -> u64 { self.bytes }
    pub fn new(bytes: u64) -> (limit: Self) ensures limit.view() == bytes { Self { bytes } }
    pub fn bytes(self) -> (bytes: u64) ensures bytes == self.view() { self.bytes }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct RetainedStderrLimit { bytes: usize }
impl RetainedStderrLimit {
    pub closed spec fn view(self) -> usize { self.bytes }
    pub fn new(bytes: usize) -> (limit: Self) ensures limit.view() == bytes { Self { bytes } }
    pub fn bytes(self) -> (bytes: usize) ensures bytes == self.view(), no_unwind { self.bytes }
}

struct ObservedBytes { total: u64, overflowed: bool }
impl ObservedBytes {
    fn zero() -> (total: Self) ensures total.total == 0, !total.overflowed {
        Self { total: 0, overflowed: false }
    }

    fn record(&mut self, bytes: u64, limit: u64) -> (admitted: bool)
        ensures
            final(self).overflowed <==> old(self).overflowed || (old(self).total as nat) + (bytes as nat) > u64::MAX,
            final(self).total == if old(self).overflowed || (old(self).total as nat) + (bytes as nat) > u64::MAX {
                u64::MAX
            } else { (old(self).total + bytes) as u64 },
            admitted <==> !old(self).overflowed && (old(self).total as nat) + (bytes as nat) <= limit,
        no_unwind
    {
        if self.overflowed || bytes > u64::MAX - self.total {
            self.total = u64::MAX;
            self.overflowed = true;
            return false;
        }
        self.total += bytes;
        self.total <= limit
    }
}

pub struct StdoutBudget { limit: StdoutByteLimit, observed: ObservedBytes }
impl StdoutBudget {
    pub closed spec fn total(&self) -> u64 { self.observed.total }
    pub closed spec fn overflowed(&self) -> bool { self.observed.overflowed }
    pub closed spec fn limit(&self) -> u64 { self.limit.view() }
    pub fn new(limit: StdoutByteLimit) -> (budget: Self)
        ensures budget.limit() == limit.view(), budget.total() == 0, !budget.overflowed(),
    { Self { limit, observed: ObservedBytes::zero() } }
    pub fn observe(&mut self, bytes: u64) -> (admitted: bool)
        ensures final(self).limit() == old(self).limit(),
            admitted <==> !old(self).overflowed() && (old(self).total() as nat) + (bytes as nat) <= old(self).limit(),
            final(self).total() >= old(self).total(),
            old(self).overflowed() ==> final(self).overflowed(),
            admitted ==> !final(self).overflowed() && final(self).total() == old(self).total() + bytes,
            !admitted ==> final(self).overflowed() || final(self).total() > final(self).limit(),
    { self.observed.record(bytes, self.limit.bytes) }
}

pub struct StderrBudget { limit: StderrByteLimit, observed: ObservedBytes }
impl StderrBudget {
    pub closed spec fn total(&self) -> u64 { self.observed.total }
    pub closed spec fn overflowed(&self) -> bool { self.observed.overflowed }
    pub closed spec fn limit(&self) -> u64 { self.limit.view() }
    pub fn new(limit: StderrByteLimit) -> (budget: Self)
        ensures budget.limit() == limit.view(), budget.total() == 0, !budget.overflowed(),
    { Self { limit, observed: ObservedBytes::zero() } }
    pub fn observe(&mut self, bytes: u64) -> (admitted: bool)
        ensures final(self).limit() == old(self).limit(),
            admitted <==> !old(self).overflowed() && (old(self).total() as nat) + (bytes as nat) <= old(self).limit(),
            final(self).total() >= old(self).total(),
            old(self).overflowed() ==> final(self).overflowed(),
            admitted ==> !final(self).overflowed() && final(self).total() == old(self).total() + bytes,
            !admitted ==> final(self).overflowed() || final(self).total() > final(self).limit(),
    { self.observed.record(bytes, self.limit.bytes) }
    pub fn truncated(&self, retained: RetainedStderrLimit) -> (truncated: bool)
        ensures truncated <==> self.overflowed() || self.total() > retained.view(),
    { self.observed.overflowed || (self.observed.total as u128) > (retained.bytes as u128) }
}

}
