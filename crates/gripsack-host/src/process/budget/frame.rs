//! Incremental LF framing; delimiters do not consume content allowance.
//! A cap failure remains terminal even if a later delimiter arrives.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameByteLimit {
    bytes: usize,
}

impl FrameByteLimit {
    pub fn new(bytes: usize) -> Self {
        Self { bytes }
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameAction {
    Append,
    Boundary,
    Limit,
}

pub struct FrameBudget {
    limit: FrameByteLimit,
    length: usize,
    failed: bool,
}

impl FrameBudget {
    pub fn new(limit: FrameByteLimit) -> Self {
        Self {
            limit,
            length: 0,
            failed: false,
        }
    }

    pub fn observe(&mut self, byte: u8) -> FrameAction {
        if self.failed {
            return FrameAction::Limit;
        }
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

    pub fn finish(&mut self) -> bool {
        let fragment = !self.failed && self.length != 0;
        self.length = 0;
        fragment
    }
}
