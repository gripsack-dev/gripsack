//! Bounded acquisition independent of executor worker count.

use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Condvar, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchLimits {
    pub concurrent: NonZeroUsize,
    pub download_bytes: NonZeroU64,
    pub expanded_bytes: NonZeroU64,
    pub archive_entries: NonZeroUsize,
    pub decoder_bytes: NonZeroU64,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            concurrent: NonZeroUsize::new(2).unwrap(),
            download_bytes: NonZeroU64::new(512 * 1024 * 1024).unwrap(),
            expanded_bytes: NonZeroU64::new(4 * 1024 * 1024 * 1024).unwrap(),
            archive_entries: NonZeroUsize::new(100_000).unwrap(),
            decoder_bytes: NonZeroU64::new(128 * 1024 * 1024).unwrap(),
        }
    }
}

pub(crate) struct AcquisitionGate {
    active: Mutex<usize>,
    changed: Condvar,
    limit: usize,
}

impl AcquisitionGate {
    pub(crate) fn new(limit: NonZeroUsize) -> Self {
        Self {
            active: Mutex::new(0),
            changed: Condvar::new(),
            limit: limit.get(),
        }
    }

    pub(crate) fn acquire(&self) -> Permit<'_> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while *active == self.limit {
            active = self
                .changed
                .wait(active)
                .unwrap_or_else(|error| error.into_inner());
        }
        *active += 1;
        Permit(self)
    }
}

pub(crate) struct Permit<'a>(&'a AcquisitionGate);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut active = self
            .0
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *active -= 1;
        self.0.changed.notify_one();
    }
}
