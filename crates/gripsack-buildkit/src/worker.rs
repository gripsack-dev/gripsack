//! One private owned worker per canonical home/profile. Live kernel leases
//! prevent stop/replacement; uncertain attempts are reconciled by stopping the
//! recorded owned container only after all inherited writer handles have closed.
//! The instance's image and resource binding is durable in the home record and
//! checked on every reuse: a changed request refuses while clients are live and
//! replaces only the idle owned container, never the cache volume or foreign
//! paths. Worker cache is disposable; host artifact publication belongs to
//! exec/store.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
mod home;
mod inspection;
mod lease;
mod observation;
pub use inspection::{WorkerDiskUsage, WorkerFact, WorkerInspection, WorkerObservation};
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod lima;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod linux;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use lima as provider;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
use linux as provider;
mod manager;
pub use gripsack_policy::worker_lease::WorkerPhase;
pub use lease::{LeaseId, WorkerLease};
pub use manager::{MemoryBytes, OwnedWorker, WorkerOptions, WorkerQuiescence, WorkerStatus};

/// Portable metadata for the owned namespace incarnation, not execution authority.
/// A deleted/recreated owner cannot retire attempts from its predecessor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerScope {
    home: crate::identity::WorkerHomeId,
    owner: crate::identity::WorkerOwnerId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerProfile(String);
impl WorkerProfile {
    pub fn parse(value: &str) -> Result<Self, WorkerError> {
        if value.is_empty()
            || value.len() > 48
            || !value.as_bytes()[0].is_ascii_alphanumeric()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(WorkerError::Invalid(
                "worker profile must be 1..=48 ASCII letters/digits/'-'/'_', starting with a letter or digit",
            ));
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerAddress(PathBuf);
impl WorkerAddress {
    pub fn socket_path(&self) -> &Path {
        &self.0
    }
    pub fn connect_arg(&self) -> OsString {
        let mut address = OsString::from("unix://");
        address.push(&self.0);
        address
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupConfirmation {
    Confirmed,
    Uncertain,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("worker I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("worker state cannot be decoded: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("invalid worker configuration: {0}")]
    Invalid(&'static str),
    #[error("owned worker state is inconsistent: {0}")]
    Corrupt(&'static str),
    #[error("refusing a foreign worker resource: {0:?}")]
    Foreign(String),
    #[error("worker operation failed: {0:?}")]
    Effect(String),
    #[error("worker has {0} live lease(s); stop/replacement is refused")]
    LiveLeases(usize),
    #[error("offline builder input is missing: {0}")]
    OfflineInput(&'static str),
    #[error("worker operation deadline expired")]
    Deadline,
    #[error("worker lease or epoch counter is exhausted")]
    CounterExhausted,
    #[error(
        "owned workers require native Linux x86_64 or macOS ARM64 with Virtualization.framework"
    )]
    UnsupportedPlatform,
}
