//! One outstanding bounded pipe write, with monotone acknowledged input.
use super::InputByteLimit;

pub const IO_CHUNK_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferPhase {
    Ready,
    Offered(usize),
    Closed,
}

pub struct InputChunk {
    start: usize,
    end: usize,
}
impl InputChunk {
    pub fn range(self) -> std::ops::Range<usize> {
        self.start..self.end
    }
}

pub struct InputTransfer {
    total: usize,
    sent: usize,
    phase: TransferPhase,
}
impl InputTransfer {
    pub fn admit(length: usize, limit: InputByteLimit) -> Option<Self> {
        if length > limit.bytes() {
            return None;
        }
        Some(Self {
            total: length,
            sent: 0,
            phase: if length == 0 {
                TransferPhase::Closed
            } else {
                TransferPhase::Ready
            },
        })
    }

    pub fn begin(&mut self) -> Option<InputChunk> {
        if !matches!(self.phase, TransferPhase::Ready) {
            return None;
        }
        let remaining = self.total - self.sent;
        let bytes = if remaining < IO_CHUNK_BYTES {
            remaining
        } else {
            IO_CHUNK_BYTES
        };
        let end = self.sent + bytes;
        self.phase = TransferPhase::Offered(bytes);
        Some(InputChunk {
            start: self.sent,
            end,
        })
    }

    pub fn complete(&mut self, written: usize) -> bool {
        let bytes = match self.phase {
            TransferPhase::Offered(bytes) => bytes,
            _ => {
                self.phase = TransferPhase::Closed;
                return false;
            }
        };
        if written == 0 || written > bytes {
            self.phase = TransferPhase::Closed;
            return false;
        }
        self.phase = TransferPhase::Closed;
        self.sent += written;
        if self.sent < self.total {
            self.phase = TransferPhase::Ready;
        }
        true
    }

    pub fn retry(&mut self) {
        if matches!(self.phase, TransferPhase::Offered(_)) {
            self.phase = TransferPhase::Ready;
        }
    }

    pub fn close(&mut self) {
        self.phase = TransferPhase::Closed;
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.phase, TransferPhase::Closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_advance_only_by_acknowledged_bytes() {
        let mut transfer = InputTransfer::admit(10_000, InputByteLimit::new(10_000)).unwrap();
        assert_eq!(transfer.begin().unwrap().range(), 0..IO_CHUNK_BYTES);
        assert!(transfer.complete(100));
        assert_eq!(transfer.begin().unwrap().range(), 100..100 + IO_CHUNK_BYTES);
        transfer.retry();
        assert_eq!(
            transfer.begin().unwrap().range(),
            100..100 + IO_CHUNK_BYTES,
            "a retry keeps progress"
        );
        assert!(transfer.complete(IO_CHUNK_BYTES));
        assert_eq!(
            transfer.begin().unwrap().range(),
            100 + IO_CHUNK_BYTES..10_000
        );
        assert!(!transfer.complete(10_000), "over-acknowledgement closes");
        assert!(transfer.is_closed());
        assert!(InputTransfer::admit(10_001, InputByteLimit::new(10_000)).is_none());
    }
}
