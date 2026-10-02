//! A stopped previous instance remains owned recovery state until the new
//! daemon's handshake succeeds. Counters never roll back with configuration.
use super::*;

impl OwnedWorker<'_> {
    fn previous_record(
        &self,
        current: &InstanceRecord,
    ) -> Result<Option<InstanceRecord>, WorkerError> {
        let Some(previous) = self.home.load_previous()? else {
            return Ok(None);
        };
        if previous.owner != current.owner
            || previous.state != RecordState::Stopped
            || previous.container_id.is_none()
        {
            return Err(WorkerError::Corrupt(
                "previous instance ownership/state is invalid",
            ));
        }
        Ok(Some(previous))
    }

    pub(super) fn reconcile_previous_for_acquire(
        &self,
        current: &mut InstanceRecord,
        inventory: &Inventory,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let Some(previous) = self.previous_record(current)? else {
            return Ok(());
        };
        if inventory.live != 0 {
            return Err(WorkerError::LiveLeases(inventory.live));
        }
        if current.state == RecordState::Ready
            && current.container_id != previous.container_id
            && inventory.stale.is_empty()
            && self
                .observe(current, lock)?
                .is_some_and(|instance| instance.running)
            && self
                .provider
                .validate_ready(&self.home, current, lock)
                .is_ok()
        {
            self.provider.finish_previous(&self.home, &previous, lock)?;
            self.home.clear_previous(&previous)?;
            return Ok(());
        }
        self.restore_previous_record(current, &previous, lock)?;
        // Keep stale evidence until the caller's explicit retry/stop owns the
        // complete inventory and can consume its retained per-lease locks.
        Err(WorkerError::Corrupt(
            "interrupted replacement restored the previous stopped worker; retry the requested build explicitly",
        ))
    }

    pub(super) fn finish_previous_after_handshake(
        &self,
        current: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let Some(previous) = self.previous_record(current)? else {
            return Ok(());
        };
        if current.state != RecordState::Ready || current.container_id == previous.container_id {
            return Err(WorkerError::Corrupt(
                "replacement cannot retire its own previous identity",
            ));
        }
        self.provider.finish_previous(&self.home, &previous, lock)?;
        self.home.clear_previous(&previous)
    }

    pub(super) fn restore_previous_if_present(
        &self,
        current: &mut InstanceRecord,
        inventory: &Inventory,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if let Some(previous) = self.previous_record(current)? {
            if inventory.live != 0 {
                return Err(WorkerError::LiveLeases(inventory.live));
            }
            self.restore_previous_record(current, &previous, lock)?;
        }
        Ok(())
    }

    fn restore_previous_record(
        &self,
        current: &mut InstanceRecord,
        previous: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        // A crash can occur after the provider restored the old name but before
        // the manager published its record. Both durable identities are checked;
        // neither a familiar name nor an unrecorded arbitrary instance suffices.
        if let Some(observed) = self.provider.inspect(&self.home, lock)? {
            let already_restored = previous.container_id.as_ref() == Some(&observed.id);
            let authority = if already_restored {
                previous
            } else {
                &*current
            };
            self.provider
                .verify_container(&self.home, authority, &observed)?;
            let identity = policy::identity_decision(
                authority
                    .container_id
                    .as_ref()
                    .map(|id| id.as_str().as_bytes()),
                Some(observed.id.as_str().as_bytes()),
            );
            if identity != policy::IdentityDecision::Current {
                if identity == policy::IdentityDecision::Unrecorded
                    && current.state == RecordState::Provisioning
                {
                    current.container_id = Some(observed.id.clone());
                    self.home.store(current)?;
                } else {
                    return Err(WorkerError::Foreign(observed.id.as_str().to_owned()));
                }
            }
            if observed.running {
                current.state = RecordState::Stopping;
                self.home.store(current)?;
                self.provider.stop(&observed.id, lock)?;
                if self
                    .provider
                    .inspect(&self.home, lock)?
                    .is_some_and(|instance| instance.running)
                {
                    return Err(WorkerError::Corrupt(
                        "replacement stop did not confirm termination",
                    ));
                }
            }
            current.state = RecordState::Stopped;
            self.home.store(current)?;
            if !already_restored {
                let identity = policy::identity_decision(
                    current
                        .container_id
                        .as_ref()
                        .map(|id| id.as_str().as_bytes()),
                    Some(observed.id.as_str().as_bytes()),
                );
                policy::admit_removal(identity)
                    .map_err(|_| WorkerError::Foreign(observed.id.as_str().to_owned()))?;
                self.provider.remove_container(&observed.id, lock)?;
                current.container_id = None;
                self.home.store(current)?;
            }
        }
        self.provider.restore_previous(&self.home, previous, lock)?;
        let next_epoch = current.next_epoch.max(previous.next_epoch);
        let next_lease = current.next_lease.max(previous.next_lease);
        *current = previous.clone();
        current.next_epoch = next_epoch;
        current.next_lease = next_lease;
        current.state = RecordState::Stopped;
        self.home.store(current)?;
        self.provider.finish_previous(&self.home, previous, lock)?;
        self.home.clear_previous(previous)
    }
}
