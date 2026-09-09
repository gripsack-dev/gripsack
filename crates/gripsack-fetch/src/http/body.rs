use super::failure::{HttpFailureKind, io_kind};
use std::io::{self, Read};
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
#[error("HTTP response read failed: {kind}")]
pub(super) struct BodyReadFailure {
    pub kind: HttpFailureKind,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid JSON metadata at line {line}, column {column}")]
pub(super) struct MetadataDecodeFailure {
    pub line: usize,
    pub column: usize,
}

pub(super) struct TransferBudget {
    pub remaining: u64,
    pub limit: u64,
}
impl TransferBudget {
    pub fn new(limit: u64) -> Self {
        Self {
            remaining: limit,
            limit,
        }
    }
}

pub(super) struct ResponseReader<'a> {
    pub inner: Box<dyn Read>,
    pub budget: &'a mut TransferBudget,
    pub deadline: Instant,
    pub expected_remaining: Option<u64>,
}

impl ResponseReader<'_> {
    fn account_length(&mut self, read: usize) -> io::Result<()> {
        if read == 0
            && self
                .expected_remaining
                .is_some_and(|remaining| remaining != 0)
        {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                BodyReadFailure {
                    kind: HttpFailureKind::InterruptedBody,
                },
            ));
        }
        if let Some(remaining) = &mut self.expected_remaining {
            *remaining = remaining.saturating_sub(read as u64);
        }
        Ok(())
    }
}
impl Read for ResponseReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                BodyReadFailure {
                    kind: HttpFailureKind::Timeout,
                },
            ));
        }
        if self.budget.remaining == 0 {
            let read = self.inner.read(&mut [0]).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    BodyReadFailure {
                        kind: io_kind(&error, true),
                    },
                )
            })?;
            self.account_length(read)?;
            return if read == 0 {
                Ok(0)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    crate::spool::LimitExceeded {
                        what: "HTTP transfer across attempts",
                        limit: self.budget.limit,
                    },
                ))
            };
        }
        let maximum = buffer
            .len()
            .min(self.budget.remaining.try_into().unwrap_or(usize::MAX));
        let read = self.inner.read(&mut buffer[..maximum]).map_err(|error| {
            io::Error::new(
                error.kind(),
                BodyReadFailure {
                    kind: io_kind(&error, true),
                },
            )
        })?;
        self.account_length(read)?;
        self.budget.remaining -= read as u64;
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_new_attempt_cannot_reset_consumed_transfer_bytes() {
        let mut budget = TransferBudget::new(4);
        let deadline = Instant::now() + std::time::Duration::from_secs(1);
        let mut first = ResponseReader {
            inner: Box::new(io::Cursor::new(b"abc")),
            budget: &mut budget,
            deadline,
            expected_remaining: Some(3),
        };
        let mut bytes = Vec::new();
        first.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"abc");
        let mut second = ResponseReader {
            inner: Box::new(io::Cursor::new(b"xy")),
            budget: &mut budget,
            deadline,
            expected_remaining: Some(2),
        };
        let error = second.read_to_end(&mut Vec::new()).unwrap_err();
        assert!(error.get_ref().unwrap().is::<crate::spool::LimitExceeded>());
    }
}
