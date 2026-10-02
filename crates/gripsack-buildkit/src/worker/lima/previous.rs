//! Parking a stopped VM is an atomic capability-relative rename. Lima's `rename`
//! command moves files individually and resets vz-identifier, so it is not a
//! crash-atomic preservation mechanism for our admitted native VM identity.
use super::*;
const PREVIOUS_VM: &str = "previous";
const PREVIOUS_INTENT: &str = "lima-previous.json";

impl Provider<'_> {
    fn previous_observation(
        &self,
        previous: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Option<ContainerObservation>, WorkerError> {
        // A completed retirement may have removed the provider record before
        // the shared manager removed its replacement transaction record.
        if self.read_intent(PREVIOUS_INTENT, previous)?.is_none() {
            if let Some(lima) = self.owned_home(previous)? {
                match lima.symlink_metadata(PREVIOUS_VM) {
                    Ok(_) => return Err(WorkerError::Foreign("unmarked previous Lima VM".into())),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            return Ok(None);
        }
        let observed = self.inspect_named(previous, PREVIOUS_VM, PREVIOUS_INTENT, lock)?;
        if let Some(instance) = &observed {
            self.verify_container(&self.home, previous, instance)?;
            if previous.container_id.as_ref() != Some(&instance.id) || instance.running {
                return Err(WorkerError::Corrupt(
                    "previous Lima VM identity/liveness changed",
                ));
            }
        }
        Ok(observed)
    }
    pub(in crate::worker) fn park_previous(
        &self,
        home: &WorkerHome,
        previous: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if self.previous_observation(previous, lock)?.is_some() {
            if self.inspect(home, lock)?.is_some() {
                return Err(WorkerError::Corrupt(
                    "both Lima VM names exist before parking",
                ));
            }
            return Ok(());
        }
        let current = self
            .inspect(home, lock)?
            .ok_or(WorkerError::Corrupt("Lima VM disappeared before parking"))?;
        self.verify_container(home, previous, &current)?;
        if current.running || previous.container_id.as_ref() != Some(&current.id) {
            return Err(WorkerError::Corrupt(
                "only the identified stopped Lima VM can be parked",
            ));
        }
        let intent = self.intent(previous)?;
        if let Some(existing) = self.read_intent(PREVIOUS_INTENT, previous)? {
            if existing.id != intent.id {
                return Err(WorkerError::Foreign(
                    "another protected Lima configuration exists".into(),
                ));
            }
        }
        self.store_intent(PREVIOUS_INTENT, &intent)?;
        let lima = self
            .owned_home(previous)?
            .ok_or(WorkerError::Corrupt("Lima namespace disappeared"))?;
        gripsack_fs::rename_noreplace(&lima, Path::new(VM_NAME), Path::new(PREVIOUS_VM))?;
        gripsack_fs::fsync_pinned_dir(&lima, Path::new(PREVIOUS_VM))?;
        self.previous_observation(previous, lock)?
            .ok_or(WorkerError::Corrupt("parked Lima VM was not observed"))?;
        Ok(())
    }
    pub(in crate::worker) fn restore_previous(
        &self,
        home: &WorkerHome,
        previous: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if let Some(current) = self.inspect_named(previous, VM_NAME, INTENT, lock)? {
            self.verify_container(home, previous, &current)?;
            if current.running
                || previous.container_id.as_ref() != Some(&current.id)
                || self.previous_observation(previous, lock)?.is_some()
            {
                return Err(WorkerError::Corrupt(
                    "restoration cannot overwrite another Lima VM",
                ));
            }
            return Ok(());
        }
        self.previous_observation(previous, lock)?
            .ok_or(WorkerError::Corrupt(
                "recoverable previous Lima VM is missing",
            ))?;
        let intent = self
            .read_intent(PREVIOUS_INTENT, previous)?
            .ok_or(WorkerError::Corrupt(
                "previous Lima configuration is missing",
            ))?;
        let lima = self
            .owned_home(previous)?
            .ok_or(WorkerError::Corrupt("Lima namespace disappeared"))?;
        self.store_intent(INTENT, &intent)?;
        gripsack_fs::rename_noreplace(&lima, Path::new(PREVIOUS_VM), Path::new(VM_NAME))?;
        gripsack_fs::fsync_pinned_dir(&lima, Path::new(VM_NAME))?;
        let restored = self
            .inspect_named(previous, VM_NAME, INTENT, lock)?
            .ok_or(WorkerError::Corrupt("restored Lima VM disappeared"))?;
        self.verify_container(home, previous, &restored)?;
        Ok(())
    }
    pub(in crate::worker) fn finish_previous(
        &self,
        _home: &WorkerHome,
        previous: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if self.previous_observation(previous, lock)?.is_some() {
            let mut intent = self
                .read_intent(PREVIOUS_INTENT, previous)?
                .ok_or(WorkerError::Corrupt("previous Lima intent disappeared"))?;
            intent.lifecycle = config::VmLifecycle::Removing;
            self.store_intent(PREVIOUS_INTENT, &intent)?;
            self.run(&args(&["delete", "--force", PREVIOUS_VM]), lock)?;
            if self.previous_observation(previous, lock)?.is_some() {
                return Err(WorkerError::Corrupt(
                    "previous Lima VM deletion was not observed",
                ));
            }
        }
        if self.read_intent(PREVIOUS_INTENT, previous)?.is_some() {
            gripsack_fs::remove_file(&self.home.dir, Path::new(PREVIOUS_INTENT))?;
            gripsack_fs::fsync_pinned_dir(&self.home.dir, Path::new(PREVIOUS_INTENT))?;
        }
        Ok(())
    }
}
