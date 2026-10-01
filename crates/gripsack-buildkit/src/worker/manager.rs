use super::{
    CleanupConfirmation, WorkerAddress, WorkerError, WorkerPhase, WorkerProfile, WorkerScope,
    home::{InstanceRecord, RecordState, WorkerHome},
    lease::{Inventory, LeaseId, WorkerLease},
    observation::{self, ContainerObservation},
    provider::{self, Provider},
};
use crate::identity::FenceEpoch;
use gripsack_policy::worker_lease as policy;
use gripsack_process::OperatorEnvironment;
use std::{
    num::{NonZeroU16, NonZeroU64},
    os::unix::fs::FileTypeExt,
    path::Path,
    time::{Duration, Instant},
};

const DEFAULT_WORKER_CPUS: NonZeroU16 = NonZeroU16::new(2).unwrap();
const DEFAULT_MEMORY_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub(super) const MINIMUM_MEMORY_BYTES: u64 = 6 * 1024 * 1024; // Docker's minimum memory limit.
const READY_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBytes(u64);
impl MemoryBytes {
    pub fn new(bytes: u64) -> Result<Self, WorkerError> {
        if bytes < MINIMUM_MEMORY_BYTES {
            return Err(WorkerError::Invalid(
                "worker memory is below Docker's six-MiB minimum",
            ));
        }
        Ok(Self(bytes))
    }
    pub fn bytes(self) -> u64 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerOptions {
    pub offline: bool,
    pub cpus: NonZeroU16,
    pub memory: MemoryBytes,
}
impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            offline: false,
            cpus: DEFAULT_WORKER_CPUS,
            memory: MemoryBytes(DEFAULT_MEMORY_BYTES),
        }
    }
}
#[derive(Debug)]
pub struct WorkerStatus {
    pub phase: WorkerPhase,
    pub live_leases: usize,
    pub uncertain_leases: usize,
    pub running: bool,
    pub instance_id: Option<String>,
}

/// Constructible only after the owned daemon was observed stopped under its
/// lifecycle lock. Later starts use larger durable epochs and are not covered.
#[derive(Debug)]
pub struct WorkerQuiescence {
    scope: WorkerScope,
    next_epoch: NonZeroU64,
}
impl WorkerQuiescence {
    pub fn retires(&self, scope: WorkerScope, epoch: FenceEpoch) -> bool {
        policy::retired_epoch(self.scope == scope, epoch.get(), self.next_epoch.get())
    }
}

pub struct OwnedWorker<'a> {
    home: WorkerHome,
    provider: Provider<'a>,
    options: WorkerOptions,
    deadline: Instant,
}
impl<'a> OwnedWorker<'a> {
    pub fn open(
        home: &Path,
        profile: WorkerProfile,
        environment: &'a OperatorEnvironment,
        options: WorkerOptions,
        deadline: Instant,
    ) -> Result<Self, WorkerError> {
        if !cfg!(any(
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64")
        )) {
            return Err(WorkerError::UnsupportedPlatform);
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        provider::validate_options(&options)?;
        let home = WorkerHome::open(home, profile)?;
        let provider = Provider::new(environment, &home, deadline)?;
        Ok(Self {
            home,
            provider,
            options,
            deadline,
        })
    }
    fn record(&self) -> Result<InstanceRecord, WorkerError> {
        match self.home.load()? {
            Some(record) => Ok(record),
            None => {
                let record = InstanceRecord::fresh(&self.home)?;
                self.home.store(&record)?;
                Ok(record)
            }
        }
    }
    fn observe(
        &self,
        record: &mut InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Option<ContainerObservation>, WorkerError> {
        let observed = self.provider.inspect(&self.home, lock)?;
        if let Some(container) = &observed {
            self.provider
                .verify_container(&self.home, record, container)?;
            let identity = policy::identity_decision(
                record
                    .container_id
                    .as_ref()
                    .map(|id| id.as_str().as_bytes()),
                Some(container.id.as_str().as_bytes()),
            );
            match identity {
                policy::IdentityDecision::Current => {
                    // A record written before resources were durable re-binds
                    // them from THIS verified owned observation, once.
                    if record.cpus.is_none() && record.memory.is_none() {
                        record.cpus = Some(observation::cpus_from_nano(container.nano_cpus).ok_or(
                            WorkerError::Corrupt("owned worker has a fractional CPU binding"),
                        )?);
                        record.memory = Some(container.memory);
                        self.home.store(record)?;
                    }
                }
                policy::IdentityDecision::Unrecorded
                    if record.state == RecordState::Provisioning =>
                {
                    // Creation intent and its private owner labels were durable
                    // before Docker ran. Reconcile a crash before ID publication.
                    record.container_id = Some(container.id.clone());
                    self.home.store(record)?;
                }
                _ => return Err(WorkerError::Foreign(container.id.as_str().to_owned())),
            }
        }
        Ok(observed)
    }

    pub fn acquire(&self) -> Result<WorkerLease, WorkerError> {
        let lock = self.home.lock(self.deadline)?;
        let mut record = self.record()?;
        let inventory = Inventory::observe(&self.home)?;
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        self.provider.prepare_control(self.options.offline, &lock)?;
        let mut observed = self.observe(&mut record, &lock)?;
        let inconsistent = !inventory.stale.is_empty()
            || matches!(
                record.state,
                RecordState::Provisioning
                    | RecordState::Starting
                    | RecordState::Stopping
                    | RecordState::Quarantined
            )
            || record.state == RecordState::Ready
                && !observed.as_ref().is_some_and(|container| container.running);
        let mut live = inventory.live;
        if inconsistent {
            record.state = RecordState::Quarantined;
            self.home.store(&record)?;
            if inventory.live != 0 {
                return Err(WorkerError::LiveLeases(inventory.live));
            }
            self.reconcile_stop(&mut record, &inventory, &lock)?;
            inventory.clear_stale(&self.home)?;
            observed = self.observe(&mut record, &lock)?;
            live = 0;
        } else if record.state != RecordState::Ready && inventory.live != 0 {
            return Err(WorkerError::LiveLeases(inventory.live));
        }
        // Resource/version binding is durable and checked on every reuse.
        // A mismatch with live clients refuses; idle, it replaces the owned
        // instance (the disposable container, never the cache volume).
        if observed.is_some() {
            match policy::admit_replacement(
                binding_matches(&record, &self.options),
                record.state.phase(),
                live as u64,
                0, // stale evidence was reconciled or absent above
            ) {
                Ok(policy::ReuseDecision::Reuse) => {}
                Ok(policy::ReuseDecision::Replace) => {
                    let idle = Inventory {
                        live: 0,
                        stale: Vec::new(),
                    };
                    self.reconcile_stop(&mut record, &idle, &lock)?;
                    if let Some(container) = self.observe(&mut record, &lock)? {
                        let identity = policy::identity_decision(
                            record
                                .container_id
                                .as_ref()
                                .map(|id| id.as_str().as_bytes()),
                            Some(container.id.as_str().as_bytes()),
                        );
                        policy::admit_removal(identity)
                            .map_err(|_| WorkerError::Foreign(container.id.as_str().to_owned()))?;
                        self.provider.remove_container(&container.id, &lock)?;
                    }
                    record.container_id = None;
                    self.home.store(&record)?;
                    observed = None;
                }
                Err(policy::ReplacementRefusal::LiveLeases) => {
                    return Err(WorkerError::LiveLeases(live));
                }
                Err(_) => {
                    return Err(WorkerError::Corrupt(
                        "replacement refused for a settled idle worker",
                    ));
                }
            }
        }
        if record.state != RecordState::Ready {
            self.home.prepare_runtime(&record)?;
            self.provider.provision_image(self.options.offline, &lock)?;
            self.provider.ensure_volume(&self.home, &record, &lock)?;
            if observed.is_none() {
                record.state = RecordState::Provisioning;
                record.image = provider::PINNED_WORKER_IMAGE.to_owned();
                record.cpus = Some(self.options.cpus);
                record.memory = Some(self.options.memory.bytes());
                self.home.store(&record)?;
                record.container_id =
                    Some(
                        self.provider
                            .create(&self.home, &record, &self.options, &lock)?,
                    );
                self.home.store(&record)?;
            }
            let epoch = record.next_epoch;
            record.next_epoch = increment(epoch)?;
            record.epoch = Some(FenceEpoch::new(epoch.get())?);
            record.state = RecordState::Starting;
            self.home.store(&record)?;
            let id = record
                .container_id
                .as_ref()
                .ok_or(WorkerError::Corrupt("starting worker lacks identity"))?;
            self.provider.start(id, &lock)?;
            self.await_socket(&mut record, &lock)?;
            record.state = RecordState::Ready;
            self.home.store(&record)?;
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        if let Err(error) = self.provider.validate_ready(&self.home, &record, &lock) {
            record.state = RecordState::Quarantined;
            self.home.store(&record)?;
            return Err(error);
        }
        // Re-observe after reconciliation; the old inventory may held locks for
        // stale evidence that has now been removed and released.
        let inventory = Inventory::observe(&self.home)?;
        if !inventory.stale.is_empty() {
            return Err(WorkerError::Corrupt(
                "uncertain leases remain after reconciliation",
            ));
        }
        policy::admit_acquire(record.state.phase(), inventory.live as u64)
            .map_err(|_| WorkerError::LiveLeases(inventory.live))?;
        let id = LeaseId::new(record.next_lease);
        record.next_lease = increment(record.next_lease)?;
        self.home.store(&record)?;
        let guard = gripsack_fs::FlockGuard::try_acquire_in(&self.home.leases, &id.name())?.ok_or(
            WorkerError::Corrupt("new lease identity already belongs to a writer"),
        )?;
        gripsack_fs::fsync_pinned_dir(&self.home.leases, Path::new("leases"))?;
        Ok(WorkerLease {
            id,
            scope: WorkerScope {
                home: self.home.identity,
                owner: record.owner,
            },
            container: record
                .container_id
                .ok_or(WorkerError::Corrupt("ready worker lost its container"))?,
            epoch: record
                .epoch
                .ok_or(WorkerError::Corrupt("ready worker lost its epoch"))?,
            address: WorkerAddress(self.home.socket()),
            guard,
        })
    }

    pub fn release(
        &self,
        lease: WorkerLease,
        cleanup: CleanupConfirmation,
    ) -> Result<(), WorkerError> {
        if lease.scope.home != self.home.identity {
            return Err(WorkerError::Corrupt(
                "lease belongs to another home/profile",
            ));
        }
        let lock = self.home.lock(self.deadline)?;
        let mut record = self.record()?;
        if lease.scope.owner != record.owner
            || record.container_id.as_ref() != Some(&lease.container)
            || record.epoch != Some(lease.epoch)
        {
            return Err(WorkerError::Corrupt("lease instance/epoch is stale"));
        }
        let inventory = Inventory::observe(&self.home)?;
        let other_clients = inventory.live.checked_sub(1).ok_or(WorkerError::Corrupt(
            "held lease disappeared from inventory",
        ))?;
        if policy::release_disposition(cleanup == CleanupConfirmation::Confirmed)
            == policy::ReleaseDisposition::Quarantined
        {
            record.state = RecordState::Quarantined;
            self.home.store(&record)?;
            return Ok(()); // File remains; inherited writers still retain their flock.
        }
        self.home
            .leases
            .remove_file(format!("{}.flock", lease.id.name()))?;
        gripsack_fs::fsync_pinned_dir(&self.home.leases, Path::new("leases"))?;
        drop(lease);
        if other_clients == 0 {
            if inventory.stale.is_empty() && record.state == RecordState::Ready {
                policy::admit_stop(record.state.phase(), 0, 0)
                    .map_err(|_| WorkerError::Corrupt("idle stop was refused"))?;
            }
            let idle = Inventory {
                live: 0,
                stale: inventory.stale,
            };
            self.reconcile_stop(&mut record, &idle, &lock)?;
            idle.clear_stale(&self.home)?;
        }
        Ok(())
    }

    fn reconcile_stop(
        &self,
        record: &mut InstanceRecord,
        inventory: &Inventory,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if inventory.live != 0 {
            return Err(WorkerError::LiveLeases(inventory.live));
        }
        if let Some(container) = self.observe(record, lock)? {
            if container.running {
                record.state = RecordState::Stopping;
                self.home.store(record)?;
                self.provider.stop(&container.id, lock)?;
                if self
                    .observe(record, lock)?
                    .is_some_and(|current| current.running)
                {
                    return Err(WorkerError::Corrupt(
                        "stop did not confirm daemon termination",
                    ));
                }
            }
        } else {
            record.container_id = None;
        }
        record.state = RecordState::Stopped;
        self.home.store(record)?;
        Ok(())
    }

    fn await_socket(
        &self,
        record: &mut InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        loop {
            let running = self
                .observe(record, lock)?
                .is_some_and(|container| container.running);
            if !running {
                return Err(WorkerError::Corrupt(
                    "worker exited before its private socket was ready",
                ));
            }
            match std::fs::symlink_metadata(self.home.socket()) {
                Ok(metadata) if metadata.file_type().is_socket() => return Ok(()),
                Ok(_) => return Err(WorkerError::Corrupt("worker endpoint is not a Unix socket")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(WorkerError::Deadline);
            }
            std::thread::sleep(remaining.min(READY_INTERVAL));
        }
    }

    pub fn status(&self) -> Result<WorkerStatus, WorkerError> {
        let lock = self.home.lock(self.deadline)?;
        let inventory = Inventory::observe(&self.home)?;
        let Some(mut record) = self.home.load()? else {
            return Ok(WorkerStatus {
                phase: WorkerPhase::Stopped,
                live_leases: inventory.live,
                uncertain_leases: inventory.stale.len(),
                running: false,
                instance_id: None,
            });
        };
        let observed = self.observe(&mut record, &lock)?;
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        if record.state == RecordState::Ready
            && observed.as_ref().is_some_and(|instance| instance.running)
        {
            self.provider.validate_ready(&self.home, &record, &lock)?;
        }
        Ok(WorkerStatus {
            phase: record.state.phase(),
            live_leases: inventory.live,
            uncertain_leases: inventory.stale.len(),
            running: observed.as_ref().is_some_and(|container| container.running),
            instance_id: observed.map(|container| container.id.as_str().to_owned()),
        })
    }
    pub fn stop(&self) -> Result<WorkerQuiescence, WorkerError> {
        let lock = self.home.lock(self.deadline)?;
        let mut record = self.record()?;
        let inventory = Inventory::observe(&self.home)?;
        self.reconcile_stop(&mut record, &inventory, &lock)?;
        inventory.clear_stale(&self.home)?;
        Ok(WorkerQuiescence {
            scope: WorkerScope {
                home: self.home.identity,
                owner: record.owner,
            },
            next_epoch: record.next_epoch,
        })
    }
    pub fn cache_cleanup(&self) -> Result<(), WorkerError> {
        let lock = self.home.lock(self.deadline)?;
        let mut record = self.record()?;
        let inventory = Inventory::observe(&self.home)?;
        if inventory.live != 0 {
            return Err(WorkerError::LiveLeases(inventory.live));
        }
        if !inventory.stale.is_empty() || record.state != RecordState::Stopped {
            return Err(WorkerError::Corrupt(
                "stop/reconcile the worker before deleting cache",
            ));
        }
        if self.provider.verify_volume(&self.home, &record, &lock)? {
            policy::admit_cache_cleanup(record.state.phase(), 0, 0, true)
                .map_err(|_| WorkerError::Corrupt("cache removal policy refused"))?;
        }
        if let Some(container) = self.observe(&mut record, &lock)? {
            if container.running {
                return Err(WorkerError::Corrupt("stopped record has a live container"));
            }
            let identity = policy::identity_decision(
                record
                    .container_id
                    .as_ref()
                    .map(|id| id.as_str().as_bytes()),
                Some(container.id.as_str().as_bytes()),
            );
            policy::admit_removal(identity)
                .map_err(|_| WorkerError::Foreign(container.id.as_str().to_owned()))?;
            self.provider.remove_container(&container.id, &lock)?;
        }
        self.provider.remove_volume(&self.home, &record, &lock)?;
        record.container_id = None;
        self.home.store(&record)?;
        self.home.remove_runtime(&record)
    }
}
fn increment(value: NonZeroU64) -> Result<NonZeroU64, WorkerError> {
    value
        .get()
        .checked_add(1)
        .and_then(NonZeroU64::new)
        .ok_or(WorkerError::CounterExhausted)
}

/// Whether the recorded durable instance binding (image + resources) matches
/// this acquisition's request. Unbound legacy records never match silently —
/// they are re-bound from a verified observation before this is consulted.
fn binding_matches(record: &InstanceRecord, options: &WorkerOptions) -> bool {
    record.image == provider::PINNED_WORKER_IMAGE
        && record.cpus == Some(options.cpus)
        && record.memory == Some(options.memory.bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::WorkerHomeId;
    use crate::worker::home;

    /// A durable ready record whose binding matches `options` exactly.
    fn bound_record(options: &WorkerOptions) -> InstanceRecord {
        InstanceRecord {
            version: home::RECORD_VERSION,
            home_identity: WorkerHomeId::of(b"manager-test-home"),
            owner: crate::identity::WorkerOwnerId::from_bytes([7; 32]),
            profile: "default".to_owned(),
            image: provider::PINNED_WORKER_IMAGE.to_owned(),
            cpus: Some(options.cpus),
            memory: Some(options.memory.bytes()),
            state: RecordState::Ready,
            container_id: Some(
                observation::ContainerId::try_from("a".repeat(64)).expect("valid container identity"),
            ),
            epoch: Some(FenceEpoch::new(1).expect("nonzero epoch")),
            next_epoch: NonZeroU64::new(2).unwrap(),
            next_lease: NonZeroU64::MIN,
        }
    }

    #[test]
    fn matching_binding_reuses_warm_worker() {
        let options = WorkerOptions::default();
        let record = bound_record(&options);
        assert!(binding_matches(&record, &options));
        assert_eq!(
            policy::admit_replacement(true, record.state.phase(), 2, 0),
            Ok(policy::ReuseDecision::Reuse),
            "matching_binding_was_not_reused"
        );
    }

    #[test]
    fn mismatched_options_refuse_replacement_with_live_clients() {
        let options = WorkerOptions {
            cpus: NonZeroU16::new(8).unwrap(),
            ..WorkerOptions::default()
        };
        let record = bound_record(&WorkerOptions::default());
        assert!(!binding_matches(&record, &options));
        assert_eq!(
            policy::admit_replacement(false, record.state.phase(), 1, 0),
            Err(policy::ReplacementRefusal::LiveLeases),
            "resource_change_proceeded_with_live_clients"
        );
    }

    #[test]
    fn mismatched_options_replace_only_when_idle_and_settled() {
        let options = WorkerOptions {
            memory: MemoryBytes::new(8 * 1024 * 1024 * 1024).unwrap(),
            ..WorkerOptions::default()
        };
        let record = bound_record(&WorkerOptions::default());
        assert!(!binding_matches(&record, &options));
        assert_eq!(
            policy::admit_replacement(false, record.state.phase(), 0, 0),
            Ok(policy::ReuseDecision::Replace),
            "idle_replacement_was_refused"
        );
        assert_eq!(
            policy::admit_replacement(false, WorkerPhase::Provisioning, 0, 0),
            Err(policy::ReplacementRefusal::MidTransition),
            "replacement_proceeded_mid_transition"
        );
        assert_eq!(
            policy::admit_replacement(false, record.state.phase(), 0, 1),
            Err(policy::ReplacementRefusal::UncertainWriters),
            "replacement_proceeded_with_uncertain_writers"
        );
    }

    #[test]
    fn changed_pin_is_a_replacement_decision_not_corruption() {
        let mut record = bound_record(&WorkerOptions::default());
        record.image = "moby/buildkit:v0.33.0@sha256:previous".to_owned();
        assert!(
            !binding_matches(&record, &WorkerOptions::default()),
            "previous_pin_matched_current_request"
        );
        assert_eq!(
            policy::admit_replacement(false, record.state.phase(), 0, 0),
            Ok(policy::ReuseDecision::Replace),
            "owned_previous_image_was_not_replaceable"
        );
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod docker_tests {
    //! Real-daemon runtime scenarios (Epic B B1-02): ignored by default and
    //! in container gates, which have no Docker engine. Run explicitly on a
    //! Linux x86_64 host with a local Docker daemon:
    //! `cargo test -p gripsack-buildkit --lib -- --ignored docker_tests`.
    use super::*;
    use crate::worker::linux;
    use std::{process::Command, time::Duration};

    const PREVIOUS_PIN: &str = "gripsack-test-previous-pin:v1";

    fn docker(arguments: &[&str]) -> Result<std::process::Output, WorkerError> {
        Ok(Command::new("docker").args(arguments).output()?)
    }
    fn docker_available() -> bool {
        docker(&["info"]).is_ok_and(|output| output.status.success())
    }

    struct RuntimeFixture {
        _root: tempfile::TempDir,
        home_path: std::path::PathBuf,
        home: WorkerHome,
        environment: OperatorEnvironment,
    }
    impl RuntimeFixture {
        fn new() -> Option<Self> {
            if !docker_available() {
                return None;
            }
            let root = tempfile::tempdir().unwrap();
            let home_path = root.path().join("home");
            std::fs::create_dir(&home_path).unwrap();
            let home =
                WorkerHome::open(&home_path, WorkerProfile::parse("default").unwrap()).unwrap();
            let environment = OperatorEnvironment::capture().unwrap();
            Some(Self {
                _root: root,
                home_path,
                home,
                environment,
            })
        }
        fn worker(&self, options: WorkerOptions) -> OwnedWorker<'_> {
            OwnedWorker::open(
                &self.home_path,
                WorkerProfile::parse("default").unwrap(),
                &self.environment,
                options,
                Instant::now() + Duration::from_secs(600),
            )
            .unwrap()
        }
        fn lease_scope(&self) -> WorkerScope {
            let record = self.home.load().unwrap().expect("record persisted");
            WorkerScope {
                home: self.home.identity,
                owner: record.owner,
            }
        }
        fn container_exists(&self) -> bool {
            docker(&[
                "container",
                "inspect",
                "--format",
                "{{.Id}}",
                &self.home.resource_name,
            ])
            .is_ok_and(|output| output.status.success())
        }
        fn volume_exists(&self) -> bool {
            docker(&["volume", "inspect", &self.home.cache_volume()])
                .is_ok_and(|output| output.status.success())
        }
    }
    impl Drop for RuntimeFixture {
        fn drop(&mut self) {
            let _ = docker(&["container", "rm", "-f", &self.home.resource_name]);
            let _ = docker(&["volume", "rm", "-f", &self.home.cache_volume()]);
            let _ = docker(&["image", "rm", PREVIOUS_PIN]);
            let _ = std::fs::remove_dir_all(&self.home.run);
        }
    }

    /// Warm reuse checks the durable resource binding; an idle options change
    /// replaces the owned container; the same change with a live lease is
    /// refused; the stop receipt retires only earlier epochs; cache cleanup
    /// then removes exactly the owned resources.
    #[test]
    #[ignore = "requires a real docker engine with the pinned buildkitd image"]
    fn warm_reuse_idle_replacement_and_live_refusal_with_real_daemon() {
        let Some(fixture) = RuntimeFixture::new() else {
            eprintln!("docker unavailable; scenario skipped");
            return;
        };
        let options = WorkerOptions::default();
        let worker = fixture.worker(options);
        let lease = worker.acquire().expect("first acquire");
        let first_instance = lease.instance_id().to_owned();
        worker
            .release(lease, CleanupConfirmation::Confirmed)
            .expect("release");
        // Warm reuse: identical options keep the same recorded instance.
        let lease = worker.acquire().expect("warm reuse");
        assert_eq!(
            lease.instance_id(),
            first_instance,
            "warm_reuse_replaced_the_instance"
        );
        // The same resource change with a live client refuses.
        let upgraded = WorkerOptions {
            cpus: NonZeroU16::new(3).unwrap(),
            ..options
        };
        let second = fixture.worker(upgraded);
        assert!(
            matches!(second.acquire(), Err(WorkerError::LiveLeases(1))),
            "resource_change_proceeded_with_live_client"
        );
        drop(second);
        worker
            .release(lease, CleanupConfirmation::Confirmed)
            .expect("release");
        // Idle: replacement proceeds and the instance identity changes.
        let upgraded_worker = fixture.worker(upgraded);
        let lease = upgraded_worker.acquire().expect("idle replacement");
        assert_ne!(
            lease.instance_id(),
            first_instance,
            "idle_replacement_reused_the_old_instance"
        );
        let epoch = lease.epoch();
        upgraded_worker
            .release(lease, CleanupConfirmation::Confirmed)
            .expect("release");
        let quiescence = upgraded_worker.stop().expect("idle stop");
        assert!(
            quiescence.retires(fixture.lease_scope(), epoch),
            "current_receipt_did_not_retire_earlier_epoch"
        );
        let later = FenceEpoch::new(epoch.get() + 1).unwrap();
        assert!(
            !quiescence.retires(fixture.lease_scope(), later),
            "current_receipt_retired_a_new_epoch"
        );
        assert!(fixture.container_exists(), "stop_removed_the_container");
        assert!(fixture.volume_exists(), "stop_removed_the_cache");
        upgraded_worker.cache_cleanup().expect("owned cleanup");
        assert!(
            !fixture.container_exists(),
            "owned_container_survived_cleanup"
        );
        assert!(!fixture.volume_exists(), "owned_cache_survived_cleanup");
        assert!(
            !fixture.home.run_dir().exists(),
            "run_child_survived_cleanup"
        );
        // The marker-bearing namespace is persistent control state (like
        // instance.json) and survives cache cleanup.
        assert!(
            fixture.home.run.join("owner").exists(),
            "namespace_marker_lost"
        );
    }

    /// A worker recorded under a PREVIOUS image spelling stays owned and
    /// cleanable after the pin moved: the record loads, the owned container
    /// and cache are removed, and no foreign path is touched.
    #[test]
    #[ignore = "requires a real docker engine with the pinned buildkitd image"]
    fn previous_image_cleanup_is_not_blocked_by_pin_change_with_real_daemon() {
        let Some(fixture) = RuntimeFixture::new() else {
            eprintln!("docker unavailable; scenario skipped");
            return;
        };
        let options = WorkerOptions::default();
        let worker = fixture.worker(options);
        let lease = worker.acquire().expect("acquire");
        worker
            .release(lease, CleanupConfirmation::Confirmed)
            .expect("release");
        worker.stop().expect("stop");
        // Simulate a pin change: the same image bytes under the previous
        // spelling, with the container and record genuinely bound to it.
        assert!(
            docker(&["image", "tag", linux::PINNED_WORKER_IMAGE, PREVIOUS_PIN])
                .unwrap()
                .status
                .success(),
            "failed to tag the previous-pin fixture image"
        );
        let mut record = fixture.home.load().unwrap().expect("record persisted");
        let old_container = record.container_id.clone().expect("container recorded");
        let lock = fixture
            .home
            .lock(Instant::now() + Duration::from_secs(60))
            .unwrap();
        worker
            .provider
            .remove_container(&old_container, &lock)
            .expect("remove current-pin container for fixture reset");
        drop(lock);
        let created = docker(&[
            "container",
            "create",
            "--pull=never",
            "--name",
            &fixture.home.resource_name,
            "--privileged",
            "--cpus",
            &options.cpus.get().to_string(),
            "--memory",
            &options.memory.bytes().to_string(),
            "--label",
            &format!("dev.gripsack.owner={}", record.owner),
            "--label",
            &format!("dev.gripsack.home={}", fixture.home.identity),
            "--label",
            "dev.gripsack.profile=default",
            PREVIOUS_PIN,
        ])
        .expect("create previous-pin fixture container");
        assert!(created.status.success(), "fixture container create failed");
        let id = String::from_utf8(created.stdout).unwrap().trim().to_owned();
        record.image = PREVIOUS_PIN.to_owned();
        record.container_id = Some(observation::ContainerId::try_from(id).unwrap());
        fixture
            .home
            .store(&record)
            .expect("store previous-pin record");
        // The current pin no longer matches the record; cleanup must proceed.
        worker.cache_cleanup().expect("previous-image cleanup");
        assert!(
            !fixture.container_exists(),
            "previous_image_container_survived"
        );
        assert!(!fixture.volume_exists(), "previous_image_cache_survived");
    }

    /// A cache volume or container under the owned NAME but without the
    /// private owner labels is foreign: acquisition refuses and stop/cache
    /// cleanup never touch it.
    #[test]
    #[ignore = "requires a real docker engine with the pinned buildkitd image"]
    fn foreign_resources_are_refused_and_untouched_with_real_daemon() {
        let Some(fixture) = RuntimeFixture::new() else {
            eprintln!("docker unavailable; scenario skipped");
            return;
        };
        // Foreign volume under the owned cache name.
        assert!(
            docker(&["volume", "create", &fixture.home.cache_volume()])
                .unwrap()
                .status
                .success(),
            "failed to create the foreign fixture volume"
        );
        let worker = fixture.worker(WorkerOptions::default());
        assert!(
            matches!(worker.acquire(), Err(WorkerError::Foreign(_))),
            "foreign_volume_was_adopted"
        );
        assert!(fixture.volume_exists(), "foreign_volume_was_removed");
        assert!(
            !fixture.container_exists(),
            "worker_started_over_foreign_cache"
        );
        assert!(
            docker(&["volume", "rm", &fixture.home.cache_volume()])
                .unwrap()
                .status
                .success(),
            "failed to reset the foreign fixture volume"
        );
        // Foreign container under the owned resource name.
        assert!(
            docker(&[
                "container",
                "create",
                "--pull=never",
                "--name",
                &fixture.home.resource_name,
                linux::PINNED_WORKER_IMAGE,
            ])
            .unwrap()
            .status
            .success(),
            "failed to create the foreign fixture container"
        );
        assert!(
            matches!(worker.acquire(), Err(WorkerError::Foreign(_))),
            "foreign_container_was_adopted"
        );
        assert!(fixture.container_exists(), "foreign_container_was_removed");
        assert!(
            matches!(worker.cache_cleanup(), Err(WorkerError::Foreign(_))),
            "cache_cleanup_touched_a_foreign_container"
        );
        assert!(
            fixture.container_exists(),
            "foreign_container_erased_by_cleanup"
        );
    }
}
