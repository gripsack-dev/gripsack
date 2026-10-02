//! Keep the exact suffix of old stderr plus the next read without overflowing.
use super::RetainedStderrLimit;
use vstd::prelude::*;

verus! {

pub struct TailAppend { discard: usize, skip: usize }
impl TailAppend {
    pub closed spec fn discard(&self) -> usize { self.discard }
    pub closed spec fn skip(&self) -> usize { self.skip }
    pub fn discard_bytes(&self) -> (bytes: usize) ensures bytes == self.discard() { self.discard }
    pub fn skip_bytes(&self) -> (bytes: usize) ensures bytes == self.skip() { self.skip }
}

pub fn retain_tail(limit: RetainedStderrLimit, retained: usize, incoming: usize) -> (append: TailAppend)
    requires retained <= limit.view(),
    ensures
        append.discard() <= retained, append.skip() <= incoming,
        retained - append.discard() + incoming - append.skip()
            == if retained + incoming > limit.view() { limit.view() as int } else { retained + incoming },
        append.skip() > 0 ==> append.discard() == retained,
        append.skip() == if incoming > limit.view() { incoming - limit.view() } else { 0int },
        append.discard() == if retained + incoming > limit.view() {
            if incoming >= limit.view() { retained as int } else { retained + incoming - limit.view() }
        } else { 0int },
    no_unwind
{
    let capacity = limit.bytes();
    if incoming >= capacity {
        TailAppend { discard: retained, skip: incoming - capacity }
    } else {
        let old_room = capacity - incoming;
        let discard = retained.saturating_sub(old_room);
        TailAppend { discard, skip: 0 }
    }
}

}
