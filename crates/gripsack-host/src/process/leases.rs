//! Explicit coordination handles, not ambient inherited descriptors. The
//! build bridge retains worker and artifact-root exclusion if its parent dies.
//! The owning lock implementation must release at last close, not LOCK_UN.
use std::{
    fs::File,
    io,
    os::fd::{AsRawFd, RawFd},
};

#[derive(Default)]
pub struct ProcessLeases {
    pub worker: Option<File>,
    pub retention: Option<File>,
}
impl ProcessLeases {
    pub(crate) fn admit(self) -> io::Result<Self> {
        Ok(Self {
            worker: self
                .worker
                .map(super::descriptors::retain_above_stdio)
                .transpose()?,
            retention: self
                .retention
                .map(super::descriptors::retain_above_stdio)
                .transpose()?,
        })
    }
    pub(crate) fn descriptors(&self) -> [Option<RawFd>; 2] {
        [
            self.worker.as_ref().map(AsRawFd::as_raw_fd),
            self.retention.as_ref().map(AsRawFd::as_raw_fd),
        ]
    }
}
