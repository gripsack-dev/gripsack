//! A serializer sink whose allocation cannot grow beyond the request budget.

use std::io::{self, Write};

pub struct InputBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl InputBuffer {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl Write for InputBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("request exceeds the {} byte cap", self.limit),
            ));
        }
        // Vec's geometric growth can exceed a non-power-of-two configured cap.
        // Reserve exactly the bytes needed once that next growth would overshoot.
        let needed = self.bytes.len() + bytes.len();
        if needed > self.bytes.capacity() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(needed)
                .min(self.limit);
            self.bytes.reserve_exact(capacity - self.bytes.len());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
