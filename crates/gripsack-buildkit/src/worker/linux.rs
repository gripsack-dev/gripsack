//! Linux worker provision/stop prototype for the B1-01 Linux lane.
//! It is not the `grip` build backend: the bridge, capability
//! handshake, private socket, Mac VM and B2 lowering are still required.
//! Docker effects use the bounded process supervisor; lease transitions
//! use [`super::WorkerLeases`]. Stop and cache deletion require both the
//! lease decision and Docker's recorded resource identity/ownership.

use super::{CleanupSet, LeaseError, WorkerLeases};
use std::num::NonZeroU16;
use std::time::Duration;

/// The pinned worker image B0 qualified on this lane (recorded in
/// `verification/buildkit-qualification/pins.env`); updates are
/// deliberate, through that harness. Rotated 2026-09-26 to the
/// current v0.33.0 manifest list whose amd64 leaf is byte-identical
/// to the originally qualified image.
pub const PINNED_WORKER_IMAGE: &str =
    "moby/buildkit:v0.33.0@sha256:6c2fa84a6b61ccd72899dde4239f8d5717f05f9a8ca6f3cad185fb1a95a94de3";

/// Admit worker names before they become Docker container/volume names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerId(String);

#[derive(Debug, thiserror::Error)]
#[error(
    "worker id must start with an ASCII alphanumeric and contain 1..=48 ASCII alphanumerics, '-' or '_'"
)]
pub struct InvalidWorkerId;

impl WorkerId {
    pub fn parse(value: &str) -> Result<Self, InvalidWorkerId> {
        let mut bytes = value.bytes();
        if value.len() > 48
            || !bytes
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(InvalidWorkerId);
        }
        Ok(Self(value.to_owned()))
    }
}

/// Docker's full immutable identity, captured from `docker run -d`.
/// Never delete a container merely because its mutable name matches.
#[derive(Debug)]
struct DockerContainerId([u8; 64]);

impl DockerContainerId {
    fn from_run_output(output: &[u8]) -> Option<Self> {
        let bytes: [u8; 64] = output.trim_ascii_end().try_into().ok()?;
        bytes
            .iter()
            .all(u8::is_ascii_hexdigit)
            .then_some(Self(bytes))
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("admitted ASCII hex Docker ID")
    }
}

/// Everything the provider needs; effects run through the injected
/// runner so the orchestration is testable without Docker.
#[derive(Debug)]
pub struct LinuxWorker<S> {
    id: WorkerId,
    port: NonZeroU16,
    supervisor: S,
    leases: WorkerLeases,
    container_id: Option<DockerContainerId>,
    cleanup: CleanupSet,
}

/// How effects are executed. Production uses the bounded process
/// supervisor (`gripsack_process::run` against `docker`); tests use a
/// recording fake.
pub trait RunDocker {
    type Error: std::fmt::Debug;

    /// Run `docker <args>` to completion under the given deadline and
    /// return (exit-ok, captured stdout).
    fn run(&mut self, args: &[&str], deadline: Duration) -> Result<(bool, Vec<u8>), Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError<E: std::fmt::Debug> {
    #[error("worker effect failed: {0:?}")]
    Effect(E),
    #[error("worker health or ownership check failed: {0}")]
    Unhealthy(String),
    #[error("lease policy rejected the operation: {0}")]
    Lease(#[from] LeaseError),
    #[error("worker {0:?} exited before its version probe completed")]
    DiedEarly(String),
}

/// The health handshake must report exactly this daemon identity.
pub const EXPECTED_DAEMON: &str = "buildkitd github.com/moby/buildkit";

impl<S: RunDocker> LinuxWorker<S> {
    /// Only admitted worker IDs produce container and volume names.
    pub fn new(id: WorkerId, port: NonZeroU16, supervisor: S) -> Self {
        let container = format!("gripsack-worker-{}", id.0);
        let cache = format!("{container}-cache");
        let mut cleanup = CleanupSet::new();
        cleanup.own(container);
        // Normal stop retains the named disk cache for warm reuse.
        cleanup.retain(cache);
        Self {
            id,
            port,
            supervisor,
            leases: WorkerLeases::provisioning(),
            container_id: None,
            cleanup,
        }
    }

    pub fn lease_state(&self) -> &WorkerLeases {
        &self.leases
    }

    pub fn container_name(&self) -> String {
        format!("gripsack-worker-{}", self.id.0)
    }

    /// Start only from Provisioning. Bootstrap a labelled disk cache
    /// and capture Docker's immutable container ID before probing the
    /// pinned daemon executable. A failed probe cannot become Ready.
    pub fn provision(&mut self) -> Result<(), ProvisionError<S::Error>> {
        if self.leases.state() != super::WorkerState::Provisioning {
            return Err(ProvisionError::Lease(LeaseError::IllegalTransition(
                self.leases.state(),
                super::WorkerState::Ready,
            )));
        }
        let result = self.start_and_probe();
        if let Err(error) = result {
            self.leases.failed().map_err(ProvisionError::Lease)?;
            if self.container_id.is_some() {
                self.remove_owned_container()?;
            }
            return Err(error);
        }
        self.leases.provisioned().map_err(ProvisionError::Lease)
    }

    fn start_and_probe(&mut self) -> Result<(), ProvisionError<S::Error>> {
        let (available, _) = self
            .supervisor
            .run(
                &["image", "inspect", PINNED_WORKER_IMAGE],
                Duration::from_secs(30),
            )
            .map_err(ProvisionError::Effect)?;
        if !available {
            return Err(ProvisionError::Unhealthy(format!(
                "pinned worker image {PINNED_WORKER_IMAGE} is not present locally; qualify the offline image before starting the worker"
            )));
        }
        self.ensure_owned_volume()?;
        let container = self.container_name();
        let publish = format!("127.0.0.1:{}:1234/tcp", self.port);
        let cache_mount = format!("{}:/var/lib/buildkit", self.cleanup_volume());
        let (ok, output) = self
            .supervisor
            .run(
                &[
                    "run",
                    "-d",
                    "--rm",
                    "--pull=never",
                    "--privileged",
                    "--platform",
                    "linux/amd64",
                    "--name",
                    &container,
                    "--label",
                    "dev.gripsack.owned=true",
                    "-p",
                    &publish,
                    "-v",
                    &cache_mount,
                    PINNED_WORKER_IMAGE,
                    "--addr",
                    "tcp://0.0.0.0:1234",
                ],
                Duration::from_secs(120),
            )
            .map_err(ProvisionError::Effect)?;
        if let Some(id) = DockerContainerId::from_run_output(&output) {
            self.container_id = Some(id);
        } else if ok {
            return Err(ProvisionError::Unhealthy(
                "docker run returned no valid full container ID; inspect the worker manually"
                    .to_string(),
            ));
        }
        if !ok {
            return Err(ProvisionError::DiedEarly(container));
        }
        self.probe_version()
    }

    fn cleanup_volume(&self) -> String {
        format!("gripsack-worker-{}-cache", self.id.0)
    }

    fn ensure_owned_volume(&mut self) -> Result<(), ProvisionError<S::Error>> {
        let volume = self.cleanup_volume();
        let label = format!("dev.gripsack.worker={}", self.id.0);
        let (exists, _) = self
            .supervisor
            .run(&["volume", "inspect", &volume], Duration::from_secs(30))
            .map_err(ProvisionError::Effect)?;
        if !exists {
            let (created, _) = self
                .supervisor
                .run(
                    &[
                        "volume",
                        "create",
                        "--label",
                        "dev.gripsack.owned=true",
                        "--label",
                        &label,
                        &volume,
                    ],
                    Duration::from_secs(60),
                )
                .map_err(ProvisionError::Effect)?;
            if !created {
                return Err(ProvisionError::Unhealthy(format!(
                    "cannot create owned cache volume {volume}"
                )));
            }
        }
        self.check_volume_owner(&volume)
    }

    fn check_volume_owner(&mut self, volume: &str) -> Result<(), ProvisionError<S::Error>> {
        let (ok, label) = self
            .supervisor
            .run(
                &[
                    "volume",
                    "inspect",
                    "--format",
                    r#"{{index .Labels "dev.gripsack.owned"}}:{{index .Labels "dev.gripsack.worker"}}"#,
                    volume,
                ],
                Duration::from_secs(30),
            )
            .map_err(ProvisionError::Effect)?;
        let expected = format!("true:{}", self.id.0);
        if !ok || label.trim_ascii_end() != expected.as_bytes() {
            return Err(ProvisionError::Unhealthy(format!(
                "cache volume {volume} is not owned by worker {}",
                self.id.0
            )));
        }
        Ok(())
    }

    /// Executable-version probe only. A daemon capability handshake
    /// remains part of the unimplemented B1-01 backend contract.
    fn probe_version(&mut self) -> Result<(), ProvisionError<S::Error>> {
        let id = self
            .container_id
            .as_ref()
            .expect("provision captured the Docker ID");
        let (ok, stdout) = self
            .supervisor
            .run(
                &["exec", id.as_str(), "buildkitd", "--version"],
                Duration::from_secs(30),
            )
            .map_err(ProvisionError::Effect)?;
        if !ok || !stdout.starts_with(EXPECTED_DAEMON.as_bytes()) {
            return Err(ProvisionError::Unhealthy(
                String::from_utf8_lossy(&stdout).trim().to_string(),
            ));
        }
        Ok(())
    }

    /// Idle-only stop through the lease kernel. The cache volume is
    /// deliberately retained. A failed docker removal cannot be
    /// reported as a successful stop: the worker writer may still live.
    pub fn stop(&mut self) -> Result<(), ProvisionError<S::Error>> {
        if self.leases.state() != super::WorkerState::Stopping {
            self.leases.request_stop().map_err(ProvisionError::Lease)?;
        }
        self.remove_owned_container()?;
        self.leases.stopped().map_err(ProvisionError::Lease)
    }

    /// Explicit teardown of the retained disk cache after an idle
    /// stop. Never remove the cache while a worker may still use it.
    pub fn teardown_with_cache(&mut self) -> Result<(), ProvisionError<S::Error>> {
        if self.leases.state() != super::WorkerState::Stopped {
            self.stop()?;
        }
        let volume = self.cleanup_volume();
        if !matches!(
            self.cleanup.may_delete(&volume),
            Err(super::CleanupError::Retained(ref name)) if name == &volume
        ) {
            return Err(ProvisionError::Unhealthy(format!(
                "cache volume {volume} is not a retained owned resource"
            )));
        }
        self.check_volume_owner(&volume)?;
        let (ok, _) = self
            .supervisor
            .run(&["volume", "rm", &volume], Duration::from_secs(60))
            .map_err(ProvisionError::Effect)?;
        if !ok {
            return Err(ProvisionError::Unhealthy(format!(
                "cache volume {volume} removal failed"
            )));
        }
        Ok(())
    }

    fn remove_owned_container(&mut self) -> Result<(), ProvisionError<S::Error>> {
        let container = self.container_name();
        if self.cleanup.may_delete(&container) != Ok(true) {
            return Err(ProvisionError::Unhealthy(format!(
                "container {container} is not owned by this worker"
            )));
        }
        let id = self.container_id.as_ref().ok_or_else(|| {
            ProvisionError::Unhealthy("no Docker container ID was captured".to_string())
        })?;
        let (inspected, label) = self
            .supervisor
            .run(
                &[
                    "inspect",
                    "--type",
                    "container",
                    "--format",
                    r#"{{index .Config.Labels "dev.gripsack.owned"}}"#,
                    id.as_str(),
                ],
                Duration::from_secs(30),
            )
            .map_err(ProvisionError::Effect)?;
        if !inspected || label.trim_ascii_end() != b"true" {
            return Err(ProvisionError::Unhealthy(format!(
                "container {container} is absent or its ownership label differs"
            )));
        }
        let (removed, _) = self
            .supervisor
            .run(&["rm", "-f", id.as_str()], Duration::from_secs(60))
            .map_err(ProvisionError::Effect)?;
        if !removed {
            return Err(ProvisionError::Unhealthy(format!(
                "owned container {container} removal failed"
            )));
        }
        self.container_id = None;
        Ok(())
    }
}

/// Production runner: every effect goes through the bounded process
/// supervisor against the `docker` CLI. Output is captured to a
/// bounded buffer; the supervisor's deadline includes cleanup.
#[derive(Debug, Default)]
pub struct DockerCli;

impl RunDocker for DockerCli {
    type Error = std::io::Error;

    fn run(&mut self, args: &[&str], deadline: Duration) -> Result<(bool, Vec<u8>), Self::Error> {
        let mut command = std::process::Command::new("/usr/bin/docker");
        // The managed Linux lane never follows an ambient remote
        // DOCKER_HOST/DOCKER_CONTEXT into a privileged remote daemon.
        command
            .env_remove("DOCKER_HOST")
            .env_remove("DOCKER_CONTEXT")
            .arg("--host")
            .arg("unix:///var/run/docker.sock")
            .args(args);
        let mut stdout: Vec<u8> = Vec::new();
        let limits = gripsack_process::Limits {
            timeout: deadline,
            line_bytes: 4096,
            stdout_bytes: 64 * 1024,
            stderr_bytes: 64 * 1024,
            retained_stderr_bytes: 4096,
            ..Default::default()
        };
        let outcome = gripsack_process::run(&mut command, &[], limits, |line| {
            stdout.extend_from_slice(line);
            stdout.push(b'\n');
            gripsack_process::Control::Continue
        })?;
        let ok = matches!(outcome.reason, gripsack_process::StopReason::Exited)
            && outcome.status.is_some_and(|status| status.success());
        Ok((ok, stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Simulate foreign resources and failed effects, not successful
    /// wire echoes; the ignored Docker case exercises the real runner.
    #[derive(Default)]
    struct FakeDocker {
        calls: RefCell<Vec<Vec<String>>>,
        healthy: bool,
        fail_run: bool,
        foreign_volume: bool,
        foreign_container: bool,
        fail_remove: bool,
    }

    impl RunDocker for FakeDocker {
        type Error = String;

        fn run(&mut self, args: &[&str], _deadline: Duration) -> Result<(bool, Vec<u8>), String> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|arg| (*arg).to_owned()).collect());
            let answer = match args {
                ["volume", "inspect", "--format", ..] if self.foreign_volume => {
                    (true, b"false:external\n".to_vec())
                }
                ["volume", "inspect", "--format", ..] => (true, b"true:test-1\n".to_vec()),
                ["volume", "inspect", ..] => (self.foreign_volume, Vec::new()),
                ["run", ..] if self.fail_run => (false, Vec::new()),
                ["run", ..] => (true, vec![b'a'; 64]),
                ["exec", ..] if !self.healthy => (false, Vec::new()),
                ["exec", ..] => (true, format!("{EXPECTED_DAEMON} v0.33.0").into_bytes()),
                ["inspect", ..] if self.foreign_container => (true, b"false\n".to_vec()),
                ["inspect", ..] => (true, b"true\n".to_vec()),
                ["rm", ..] if self.fail_remove => (false, Vec::new()),
                _ => (true, Vec::new()),
            };
            Ok(answer)
        }
    }

    fn make(fake: FakeDocker) -> LinuxWorker<FakeDocker> {
        LinuxWorker::new(
            WorkerId::parse("test-1").unwrap(),
            NonZeroU16::new(41234).unwrap(),
            fake,
        )
    }

    /// Real daemon, not a fake BuildKit worker. Intentionally ignored
    /// in container gates, which have no Docker daemon.
    #[test]
    #[ignore = "requires a real docker engine with the pinned buildkitd image"]
    fn the_real_docker_worker_provisions_leases_and_stops() {
        let id = WorkerId::parse(&format!("it-{}", std::process::id())).unwrap();
        let mut worker = LinuxWorker::new(id, NonZeroU16::new(47123).unwrap(), DockerCli);
        let cache = worker.cleanup_volume();
        worker.provision().expect("provision + version probe");
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Ready
        );
        let lease = worker.leases.acquire(super::super::ClientId(1)).unwrap();
        assert!(worker.stop().is_err(), "a live lease blocks the stop");
        worker.leases.release(lease).unwrap();
        worker.stop().expect("idle stop");
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Stopped
        );
        let (cache_exists, _) = DockerCli
            .run(&["volume", "inspect", &cache], Duration::from_secs(30))
            .unwrap();
        assert!(
            cache_exists,
            "the disk cache is retained after an idle stop"
        );
        let (container_exists, _) = DockerCli
            .run(
                &["inspect", &worker.container_name()],
                Duration::from_secs(30),
            )
            .unwrap();
        assert!(!container_exists, "the owned container is removed");
        worker
            .teardown_with_cache()
            .expect("explicit owner teardown");
        let (cache_exists, _) = DockerCli
            .run(&["volume", "inspect", &cache], Duration::from_secs(30))
            .unwrap();
        assert!(!cache_exists, "explicit teardown removes the cache");
    }

    #[test]
    #[ignore = "requires a real docker engine with the pinned buildkitd image"]
    fn the_real_docker_worker_refuses_a_foreign_cache_volume() {
        struct RemoveFixtureVolume(String);
        impl Drop for RemoveFixtureVolume {
            fn drop(&mut self) {
                let _ = DockerCli.run(&["volume", "rm", &self.0], Duration::from_secs(30));
            }
        }

        let id = WorkerId::parse(&format!("foreign-{}", std::process::id())).unwrap();
        let mut worker = LinuxWorker::new(id, NonZeroU16::new(47124).unwrap(), DockerCli);
        let cache = worker.cleanup_volume();
        let (created, _) = DockerCli
            .run(&["volume", "create", &cache], Duration::from_secs(30))
            .unwrap();
        assert!(
            created,
            "create a user-owned volume with no Gripsack labels"
        );
        let _fixture = RemoveFixtureVolume(cache.clone());
        assert!(matches!(
            worker.provision(),
            Err(ProvisionError::Unhealthy(_))
        ));
        let (present, _) = DockerCli
            .run(&["volume", "inspect", &cache], Duration::from_secs(30))
            .unwrap();
        assert!(present, "a foreign volume remains untouched");
        let (container_exists, _) = DockerCli
            .run(
                &["inspect", &worker.container_name()],
                Duration::from_secs(30),
            )
            .unwrap();
        assert!(
            !container_exists,
            "no worker starts with foreign cache data"
        );
    }

    #[test]
    fn worker_identity_rejects_unsafe_or_unbounded_docker_names() {
        for value in ["", "-relative", "../outside", "contains space", "é", "x/y"] {
            assert!(WorkerId::parse(value).is_err(), "{value:?}");
        }
        assert!(WorkerId::parse(&"x".repeat(49)).is_err());
    }

    #[test]
    fn foreign_cache_volume_blocks_provision_without_launch_or_deletion() {
        let mut worker = make(FakeDocker {
            healthy: true,
            foreign_volume: true,
            ..Default::default()
        });
        assert!(matches!(
            worker.provision(),
            Err(ProvisionError::Unhealthy(_))
        ));
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Failed
        );
        let calls = worker.supervisor.calls.borrow();
        assert!(!calls.iter().any(|call| {
            matches!(call.first().map(String::as_str), Some("run") | Some("rm"))
                || call
                    .as_slice()
                    .starts_with(&["volume".to_owned(), "rm".to_owned()])
        }));
    }

    #[test]
    fn failed_version_probe_cannot_authorize_a_lease() {
        let mut worker = make(FakeDocker::default());
        assert!(matches!(
            worker.provision(),
            Err(ProvisionError::Unhealthy(_))
        ));
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Failed
        );
        assert!(worker.leases.acquire(super::super::ClientId(1)).is_err());
    }

    #[test]
    fn foreign_container_identity_blocks_deletion_and_stop() {
        let mut worker = make(FakeDocker {
            healthy: true,
            ..Default::default()
        });
        worker.provision().unwrap();
        worker.supervisor.foreign_container = true;
        assert!(matches!(worker.stop(), Err(ProvisionError::Unhealthy(_))));
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Stopping
        );
        assert!(
            !worker
                .supervisor
                .calls
                .borrow()
                .iter()
                .any(|call| { call.first().is_some_and(|arg| arg == "rm") })
        );
        worker.supervisor.foreign_container = false;
        worker.stop().unwrap();
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Stopped
        );
    }

    #[test]
    fn failed_removal_does_not_claim_stopped_and_can_retry() {
        let mut worker = make(FakeDocker {
            healthy: true,
            fail_remove: true,
            ..Default::default()
        });
        worker.provision().unwrap();
        assert!(matches!(worker.stop(), Err(ProvisionError::Unhealthy(_))));
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Stopping
        );
        worker.supervisor.fail_remove = false;
        worker.stop().unwrap();
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Stopped
        );
    }

    #[test]
    fn live_lease_blocks_worker_deletion_until_drained() {
        let mut worker = make(FakeDocker {
            healthy: true,
            ..Default::default()
        });
        worker.provision().unwrap();
        let lease = worker.leases.acquire(super::super::ClientId(1)).unwrap();
        assert!(matches!(worker.stop(), Err(ProvisionError::Lease(_))));
        assert_eq!(worker.lease_state().live_leases(), 1);
        worker.leases.release(lease).unwrap();
        worker.stop().unwrap();
        worker.teardown_with_cache().unwrap();
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Stopped
        );
    }

    #[test]
    fn failed_launch_with_no_container_id_never_deletes_by_name() {
        let mut worker = make(FakeDocker {
            fail_run: true,
            ..Default::default()
        });
        assert!(matches!(
            worker.provision(),
            Err(ProvisionError::DiedEarly(_))
        ));
        assert_eq!(
            worker.lease_state().state(),
            super::super::WorkerState::Failed
        );
        assert!(
            !worker
                .supervisor
                .calls
                .borrow()
                .iter()
                .any(|call| { call.first().is_some_and(|arg| arg == "rm") })
        );
    }
}
