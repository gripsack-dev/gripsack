//! Keep the exact suffix of old stderr plus the next read without overflowing.
use super::RetainedStderrLimit;

pub struct TailAppend {
    discard: usize,
    skip: usize,
}
impl TailAppend {
    pub fn discard_bytes(&self) -> usize {
        self.discard
    }
    pub fn skip_bytes(&self) -> usize {
        self.skip
    }
}

/// How much of the `retained` tail (≤ the limit) to discard and how much of
/// the `incoming` read to skip so that exactly the last `limit` bytes remain.
pub fn retain_tail(limit: RetainedStderrLimit, retained: usize, incoming: usize) -> TailAppend {
    debug_assert!(retained <= limit.bytes());
    let capacity = limit.bytes();
    if incoming >= capacity {
        TailAppend {
            discard: retained,
            skip: incoming - capacity,
        }
    } else {
        let old_room = capacity - incoming;
        let discard = retained.saturating_sub(old_room);
        TailAppend { discard, skip: 0 }
    }
}
