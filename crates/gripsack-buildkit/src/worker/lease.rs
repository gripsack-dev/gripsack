use super::{WorkerAddress, WorkerError, WorkerScope, home::WorkerHome, observation::ContainerId};
use crate::identity::FenceEpoch;
use std::{num::NonZeroU64, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseId(NonZeroU64);
impl LeaseId {
    pub fn value(self) -> u64 {
        self.0.get()
    }
    pub(super) fn new(value: NonZeroU64) -> Self {
        Self(value)
    }
    pub(super) fn name(self) -> String {
        format!("lease-{:016x}", self.value())
    }
    fn parse(file: &str) -> Result<Self, WorkerError> {
        let value = file
            .strip_prefix("lease-")
            .and_then(|value| value.strip_suffix(".flock"))
            .ok_or(WorkerError::Corrupt(
                "unknown entry in worker lease directory",
            ))?;
        if value.len() != 16
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(WorkerError::Corrupt("invalid worker lease filename"));
        }
        let value = u64::from_str_radix(value, 16)
            .map_err(|_| WorkerError::Corrupt("invalid lease counter"))?;
        Ok(Self(NonZeroU64::new(value).ok_or(WorkerError::Corrupt(
            "zero worker lease identity",
        ))?))
    }
}

/// Drop deliberately leaves the file as unconfirmed evidence. The lock closes
/// only after all explicitly inherited bridge handles have closed too.
pub struct WorkerLease {
    pub(super) id: LeaseId,
    pub(super) scope: WorkerScope,
    pub(super) container: ContainerId,
    pub(super) epoch: FenceEpoch,
    pub(super) address: WorkerAddress,
    pub(super) guard: gripsack_fs::FlockGuard,
}
impl WorkerLease {
    pub fn id(&self) -> LeaseId {
        self.id
    }
    pub fn scope(&self) -> WorkerScope {
        self.scope
    }
    pub fn epoch(&self) -> FenceEpoch {
        self.epoch
    }
    pub fn address(&self) -> &WorkerAddress {
        &self.address
    }
    pub fn instance_id(&self) -> &str {
        self.container.as_str()
    }
    pub fn duplicate_handle(&self) -> std::io::Result<std::fs::File> {
        self.guard.duplicate_handle()
    }
}

pub(super) struct Inventory {
    pub live: usize,
    pub stale: Vec<(LeaseId, gripsack_fs::FlockGuard)>,
}
impl Inventory {
    pub(super) fn observe(home: &WorkerHome) -> Result<Self, WorkerError> {
        let mut result = Self {
            live: 0,
            stale: Vec::new(),
        };
        for entry in home.leases.entries()? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                return Err(WorkerError::Corrupt("lease evidence is not a regular file"));
            }
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or(WorkerError::Corrupt("lease filename is not UTF-8"))?;
            let id = LeaseId::parse(name)?;
            match gripsack_fs::FlockGuard::try_acquire_in(&home.leases, &id.name())? {
                Some(guard) => result.stale.push((id, guard)),
                None => result.live += 1,
            }
            if result.live + result.stale.len()
                > gripsack_policy::worker_lease::MAX_LIVE_LEASES as usize
            {
                return Err(WorkerError::Corrupt(
                    "lease evidence exceeds its bounded inventory",
                ));
            }
        }
        Ok(result)
    }
    /// Only called after the recorded daemon is confirmed stopped, with the
    /// lifecycle lock and each stale lease's own lock still held.
    pub(super) fn clear_stale(self, home: &WorkerHome) -> Result<(), WorkerError> {
        for (id, _guard) in self.stale {
            home.leases.remove_file(format!("{}.flock", id.name()))?;
        }
        gripsack_fs::fsync_pinned_dir(&home.leases, Path::new("leases"))?;
        Ok(())
    }
}
