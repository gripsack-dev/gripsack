//! Incremental LF framing; delimiters do not consume content allowance.
//! A cap failure remains terminal even if a later delimiter arrives.
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct FrameByteLimit { bytes: usize }

impl FrameByteLimit {
    pub closed spec fn view(self) -> usize { self.bytes }
    pub fn new(bytes: usize) -> (limit: Self)
        ensures limit.view() == bytes,
    { Self { bytes } }
    pub fn bytes(self) -> (bytes: usize)
        ensures bytes == self.view(),
    { self.bytes }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum FrameAction { Append, Boundary, Limit }

pub struct FrameBudget { limit: FrameByteLimit, length: usize, failed: bool }

impl FrameBudget {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool { self.length <= self.limit.view() }

    pub closed spec fn length(&self) -> usize { self.length }
    pub closed spec fn limit(&self) -> usize { self.limit.view() }
    pub closed spec fn failed(&self) -> bool { self.failed }

    pub fn new(limit: FrameByteLimit) -> (frame: Self)
        ensures frame.limit() == limit.view(), frame.length() == 0, !frame.failed(),
    { Self { limit, length: 0, failed: false } }

    pub fn observe(&mut self, byte: u8) -> (action: FrameAction)
        ensures
            final(self).limit() == old(self).limit(),
            action == if old(self).failed() { FrameAction::Limit }
                else if byte == 10 { FrameAction::Boundary }
                else if old(self).length() == old(self).limit() { FrameAction::Limit }
                else { FrameAction::Append },
            final(self).failed() <==> action == FrameAction::Limit,
            final(self).length() == match action {
                FrameAction::Append => old(self).length() + 1,
                FrameAction::Boundary => 0int,
                FrameAction::Limit => old(self).length() as int,
            },
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        if self.failed { return FrameAction::Limit; }
        if byte == b'\n' {
            self.length = 0;
            return FrameAction::Boundary;
        }
        if self.length == self.limit.bytes {
            self.failed = true;
            return FrameAction::Limit;
        }
        self.length += 1;
        FrameAction::Append
    }

    pub fn finish(&mut self) -> (fragment: bool)
        ensures
            fragment <==> !old(self).failed() && old(self).length() > 0,
            final(self).length() == 0,
            final(self).failed() == old(self).failed(),
            final(self).limit() == old(self).limit(),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        let fragment = !self.failed && self.length != 0;
        self.length = 0;
        fragment
    }
}

}
