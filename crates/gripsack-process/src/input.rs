//! Bounded serializer bytes and allocation requests; allocator granularity is external.

use super::InputByteLimit;
use gripsack_policy::process_budget::admit_input_append;
use std::io::{self, Write};

pub struct InputBuffer {
    bytes: Vec<u8>,
    limit: InputByteLimit,
}

impl InputBuffer {
    pub fn new(limit: InputByteLimit) -> Self {
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
        let Some(append) = admit_input_append(
            self.limit,
            self.bytes.len(),
            self.bytes.capacity(),
            bytes.len(),
        ) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("request exceeds the {} byte cap", self.limit.bytes()),
            ));
        };
        if append.reserve_additional() != 0 {
            self.bytes.reserve_exact(append.reserve_additional());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_append_preserves_the_admitted_request() {
        let mut input = InputBuffer::new(InputByteLimit::new(17));
        input.write_all(b"abcd").unwrap();
        input.write_all(b"efghijklmnop").unwrap();
        let error = input
            .write_all(b"QR")
            .expect_err("serializer_excess_was_admitted");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            input.as_bytes(),
            b"abcdefghijklmnop",
            "rejected_serializer_append_changed_request",
        );
        input.write_all(b"q").unwrap();
        assert_eq!(input.as_bytes(), b"abcdefghijklmnopq");
    }
}
