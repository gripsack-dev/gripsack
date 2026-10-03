//! Retain the stopped old container until the replacement's actual handshake.
//! Docker's immutable ID, not the parking name, authorizes every effect.
use super::*;

fn previous_name(home: &WorkerHome) -> String {
    format!("{}-previous", home.resource_name)
}

impl Docker<'_> {
    fn previous_observation(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Option<ContainerObservation>, WorkerError> {
        let observed = self.inspect_named(&previous_name(home), lock)?;
        if let Some(instance) = &observed {
            self.verify_container(home, record, instance)?;
            if record.container_id.as_ref() != Some(&instance.id) {
                return Err(WorkerError::Foreign(instance.id.as_str().to_owned()));
            }
            if instance.running {
                return Err(WorkerError::Corrupt(
                    "previous owned container unexpectedly runs",
                ));
            }
        }
        Ok(observed)
    }

    pub(in crate::worker) fn park_previous(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if self.previous_observation(home, record, lock)?.is_some() {
            if self.inspect(home, lock)?.is_some() {
                return Err(WorkerError::Corrupt(
                    "both current and previous containers exist before parking",
                ));
            }
            return Ok(());
        }
        let instance = self
            .inspect(home, lock)?
            .ok_or(WorkerError::Corrupt("container disappeared before parking"))?;
        self.verify_container(home, record, &instance)?;
        if record.container_id.as_ref() != Some(&instance.id) || instance.running {
            return Err(WorkerError::Corrupt(
                "only the identified stopped container can be parked",
            ));
        }
        self.run(
            &args(&[
                "container",
                "rename",
                instance.id.as_str(),
                &previous_name(home),
            ]),
            lock,
        )?;
        if self.previous_observation(home, record, lock)?.is_none() {
            return Err(WorkerError::Corrupt("container parking was not observed"));
        }
        Ok(())
    }

    pub(in crate::worker) fn restore_previous(
        &self,
        home: &WorkerHome,
        previous: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let parked = self.previous_observation(home, previous, lock)?;
        if let Some(current) = self.inspect(home, lock)? {
            self.verify_container(home, previous, &current)?;
            if current.running
                || previous.container_id.as_ref() != Some(&current.id)
                || parked.is_some()
            {
                return Err(WorkerError::Corrupt(
                    "previous container cannot overwrite an existing instance",
                ));
            }
            return Ok(()); // Crash before the parking rename.
        }
        let parked = parked.ok_or(WorkerError::Corrupt(
            "recoverable previous container is missing",
        ))?;
        self.run(
            &args(&[
                "container",
                "rename",
                parked.id.as_str(),
                &home.resource_name,
            ]),
            lock,
        )?;
        let current = self
            .inspect(home, lock)?
            .ok_or(WorkerError::Corrupt("restored container is missing"))?;
        self.verify_container(home, previous, &current)?;
        if previous.container_id.as_ref() != Some(&current.id) || current.running {
            return Err(WorkerError::Corrupt(
                "restored container identity/liveness changed",
            ));
        }
        Ok(())
    }

    pub(in crate::worker) fn finish_previous(
        &self,
        home: &WorkerHome,
        previous: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if let Some(parked) = self.previous_observation(home, previous, lock)? {
            self.remove_container(&parked.id, lock)?;
            if self.previous_observation(home, previous, lock)?.is_some() {
                return Err(WorkerError::Corrupt(
                    "previous container retirement was not observed",
                ));
            }
        }
        Ok(())
    }
}
