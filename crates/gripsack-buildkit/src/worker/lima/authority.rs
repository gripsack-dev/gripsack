//! Durable control authority is outside the disposable short socket namespace.
use super::*;
use config::VmLifecycle;

impl Provider<'_> {
    pub(super) fn intent(&self, record: &InstanceRecord) -> Result<VmIntent, WorkerError> {
        self.read_intent(INTENT, record)?
            .ok_or(WorkerError::Corrupt("Lima instance intent is missing"))
    }
    pub(super) fn read_intent(
        &self,
        name: &str,
        record: &InstanceRecord,
    ) -> Result<Option<VmIntent>, WorkerError> {
        let bytes = match bounded_read(&self.home.dir, Path::new(name)) {
            Ok(bytes) => bytes,
            Err(WorkerError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let intent: VmIntent = serde_json::from_slice(&bytes)?;
        intent.verify_owner(&self.home, record)?;
        Ok(Some(intent))
    }
    pub(super) fn store_intent(&self, name: &str, intent: &VmIntent) -> Result<(), WorkerError> {
        gripsack_fs::atomic_write(
            &self.home.dir,
            Path::new(name),
            &serde_json::to_vec(intent)?,
        )?;
        Ok(())
    }
    pub(super) fn configuration(
        &self,
        lima: &gripsack_fs::Dir,
        intent: &VmIntent,
    ) -> Result<(), WorkerError> {
        self.configuration_named(lima, intent, VM_NAME)
    }
    pub(super) fn configuration_named(
        &self,
        lima: &gripsack_fs::Dir,
        intent: &VmIntent,
        name: &str,
    ) -> Result<(), WorkerError> {
        let bytes = bounded_read(lima, &Path::new(name).join("lima.yaml"))?;
        let observed: serde_json::Value = serde_yaml::from_slice(&bytes)
            .map_err(|_| WorkerError::Corrupt("Lima persisted configuration is invalid YAML"))?;
        if observed != intent.configuration {
            return Err(WorkerError::Foreign(
                "Lima persisted configuration differs from owned intent".into(),
            ));
        }
        Ok(())
    }
    fn admit_missing_runtime(
        &self,
        record: &InstanceRecord,
        intent: Option<&VmIntent>,
    ) -> Result<(), WorkerError> {
        match intent {
            Some(intent)
                if intent.lifecycle == VmLifecycle::MayBeRunning
                    && intent.boot_session.as_str() == capabilities::boot_session_id()? =>
            {
                Err(WorkerError::Corrupt(
                    "same-boot Lima runtime authority disappeared; VM quiescence is unconfirmed",
                ))
            }
            None if record.container_id.is_some() => Err(WorkerError::Corrupt(
                "recorded Lima instance lost durable authority",
            )),
            _ => Ok(()),
        }
    }
    pub(super) fn inspect_named(
        &self,
        record: &InstanceRecord,
        name: &str,
        intent_name: &str,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Option<ContainerObservation>, WorkerError> {
        let mut intent = self.read_intent(intent_name, record)?;
        let Some(lima) = self.owned_home(record)? else {
            self.admit_missing_runtime(record, intent.as_ref())?;
            return Ok(None);
        };
        match gripsack_fs::open_dir_nofollow(&lima, Path::new(name)) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.admit_missing_runtime(record, intent.as_ref())?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        }
        let intent = intent
            .as_mut()
            .ok_or(WorkerError::Foreign("unmarked Lima instance".into()))?;
        self.configuration_named(&lima, intent, name)?;
        let bytes = self.run(&args(&["list", "--format", INSPECT_FORMAT, name]), lock)?;
        let observed: VmObservation = serde_json::from_slice(&bytes)?;
        let observed = observed.admit(intent, &self.lima_home().join(name))?;
        let machine_path = Path::new(name).join("vz-identifier");
        match bounded_read(&lima, &machine_path) {
            Ok(bytes) if !bytes.is_empty() => {
                let identity = Sha256Digest::of(&bytes);
                match intent.machine_identity {
                    Some(expected) if expected != identity => {
                        return Err(WorkerError::Foreign(
                            "Lima native VM identifier changed".into(),
                        ));
                    }
                    None if observed.running
                        && record.state == super::super::home::RecordState::Starting =>
                    {
                        intent.machine_identity = Some(identity);
                        self.store_intent(intent_name, intent)?;
                    }
                    None if observed.running => {
                        return Err(WorkerError::Corrupt(
                            "running VM has no admitted native identity",
                        ));
                    }
                    _ => {}
                }
            }
            Err(WorkerError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound
                    && !observed.running
                    && intent.machine_identity.is_none() => {}
            Ok(_) if !observed.running && intent.machine_identity.is_none() => {}
            Err(error) => return Err(error),
            _ => return Err(WorkerError::Corrupt("Lima native VM identifier is absent")),
        }
        if !observed.running && intent.lifecycle == VmLifecycle::MayBeRunning {
            intent.lifecycle = VmLifecycle::Stopped;
            self.store_intent(intent_name, intent)?;
        }
        Ok(Some(observed))
    }
}
