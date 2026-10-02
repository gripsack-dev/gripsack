//! The one qualified local Linux effect boundary. No ambient Docker context,
//! host network entitlement, broad HOME mount or process-sandbox workaround.
mod previous;
use super::{
    WorkerError,
    home::{InstanceRecord, WorkerHome},
    observation::{ContainerId, ContainerObservation, nano_cpus},
};
use gripsack_process::{
    Control, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessLeases, ProcessRole,
    SelectedProgram, StdoutByteLimit,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Instant,
};

pub const PINNED_WORKER_IMAGE: &str =
    "moby/buildkit:v0.33.0@sha256:6c2fa84a6b61ccd72899dde4239f8d5717f05f9a8ca6f3cad185fb1a95a94de3";
const DOCKER_SOCKET: &str = "unix:///var/run/docker.sock";
const MAX_DOCKER_OUTPUT: usize = 1024 * 1024;
const OWNER_LABEL: &str = "dev.gripsack.owner";
const HOME_LABEL: &str = "dev.gripsack.home";
const PROFILE_LABEL: &str = "dev.gripsack.profile";

pub(super) fn preflight(
    _options: &super::WorkerOptions,
    deadline: Instant,
) -> Result<(), WorkerError> {
    // CPU/memory value domains were admitted at construction. Preserve the
    // existing Linux local-Docker capability boundary at acquisition.
    if Instant::now() >= deadline {
        return Err(WorkerError::Deadline);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeObservation {
    name: String,
    /// Docker emits null when a volume has no labels; absent labels are
    /// NEVER owned.
    labels: Option<BTreeMap<String, String>>,
}

pub(super) struct Docker<'a> {
    environment: &'a OperatorEnvironment,
    program: std::sync::OnceLock<SelectedProgram>,
    config: PathBuf,
    directory: PathBuf,
    deadline: Instant,
}
pub(super) type Provider<'a> = Docker<'a>;
impl<'a> Docker<'a> {
    pub(super) fn new(
        environment: &'a OperatorEnvironment,
        home: &WorkerHome,
        deadline: Instant,
    ) -> Result<Self, WorkerError> {
        Ok(Self {
            environment,
            program: std::sync::OnceLock::new(),
            config: home.root.join("docker-config"),
            directory: home.root.clone(),
            deadline,
        })
    }
    fn invoke(
        &self,
        arguments: &[OsString],
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(gripsack_process::NativeOutcome, Vec<u8>), WorkerError> {
        if self.program.get().is_none() {
            let selected = SelectedProgram::select(
                self.environment,
                Path::new("docker"),
                None,
                self.deadline,
            )?;
            // Invocations hold the cross-process lifecycle lock. If another
            // caller initialized first, retain that already-selected program.
            let _ = self.program.set(selected);
        }
        let program = self.program.get().ok_or(WorkerError::Corrupt(
            "Docker executable selection disappeared",
        ))?;
        let limits = Limits {
            timeout: self.deadline.saturating_duration_since(Instant::now()),
            operation_deadline: Some(self.deadline),
            stdout_bytes: StdoutByteLimit::new(MAX_DOCKER_OUTPUT as u64),
            ..Limits::default()
        };
        let invocation = Invocation::admit(
            self.environment,
            ProcessRole::Build,
            program,
            &self.directory,
            limits,
        )?
        .retain_leases(ProcessLeases {
            worker: Some(lock.duplicate_handle()?),
            retention: None,
        })?;
        let mut argv = Vec::with_capacity(arguments.len() + 4);
        argv.extend([
            OsStr::new("--host"),
            OsStr::new(DOCKER_SOCKET),
            OsStr::new("--config"),
            self.config.as_os_str(),
        ]);
        argv.extend(arguments.iter().map(OsString::as_os_str));
        let mut stdout = Vec::new();
        let outcome = invocation.run(&argv, NativeInput::Bytes(b""), None, |bytes| {
            if bytes.len() <= MAX_DOCKER_OUTPUT.saturating_sub(stdout.len()) {
                stdout.extend_from_slice(bytes);
            }
            Control::Continue
        })?;
        Ok((outcome, stdout))
    }
    fn run(
        &self,
        arguments: &[OsString],
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Vec<u8>, WorkerError> {
        let (outcome, stdout) = self.invoke(arguments, lock)?;
        if !outcome.success {
            return Err(WorkerError::Effect(
                String::from_utf8_lossy(&outcome.stderr).into_owned(),
            ));
        }
        Ok(stdout)
    }
    pub(super) fn provision_image(
        &self,
        offline: bool,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let (outcome, _) = self.invoke(
            &args(&[
                "image",
                "inspect",
                "--format",
                "{{.Id}}",
                PINNED_WORKER_IMAGE,
            ]),
            lock,
        )?;
        if outcome.success {
            return Ok(());
        }
        let missing = outcome.receipt.exit_code == Some(1)
            && outcome.receipt.disposition == gripsack_process::ProcessDisposition::Exited
            && String::from_utf8_lossy(&outcome.stderr)
                .trim_start()
                .starts_with("Error: No such image:");
        if !missing {
            return Err(WorkerError::Effect(
                String::from_utf8_lossy(&outcome.stderr).into_owned(),
            ));
        }
        if offline {
            return Err(WorkerError::OfflineInput(PINNED_WORKER_IMAGE));
        }
        self.run(&args(&["pull", PINNED_WORKER_IMAGE]), lock)?;
        Ok(())
    }
    pub(super) fn inspect(
        &self,
        home: &WorkerHome,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Option<ContainerObservation>, WorkerError> {
        self.inspect_named(&home.resource_name, lock)
    }
    fn inspect_named(
        &self,
        name: &str,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Option<ContainerObservation>, WorkerError> {
        let names = self.run(
            &args(&[
                "container",
                "ls",
                "--all",
                "--no-trunc",
                "--filter",
                &format!("name=^/{name}$"),
                "--format",
                "{{.ID}}",
            ]),
            lock,
        )?;
        let names = std::str::from_utf8(&names)
            .map_err(|_| WorkerError::Corrupt("Docker emitted non-UTF-8 identity"))?
            .trim();
        if names.is_empty() {
            return Ok(None);
        }
        let id = ContainerId::try_from(names.to_owned())?;
        let format = r#"{"id":{{json .Id}},"running":{{json .State.Running}},"image":{{json .Config.Image}},"nano_cpus":{{json .HostConfig.NanoCpus}},"memory":{{json .HostConfig.Memory}},"labels":{{json .Config.Labels}}}"#;
        let bytes = self.run(
            &args(&["container", "inspect", "--format", format, id.as_str()]),
            lock,
        )?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }
    pub(super) fn configured_details(
        &self,
        home: &WorkerHome,
        record: Option<&InstanceRecord>,
        options: &super::WorkerOptions,
    ) -> super::inspection::WorkerInspection {
        super::inspection::WorkerInspection::configured(
            record,
            options,
            "linux/amd64",
            home.cache_volume(),
        )
    }

    fn daemon_details(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(String, String), WorkerError> {
        let bytes = self.run(
            &args(&[
                "exec",
                id.as_str(),
                "buildctl",
                "--addr",
                "unix:///run/gripsack/buildkitd.sock",
                "debug",
                "workers",
                "--format",
                "{{json .}}",
            ]),
            lock,
        )?;
        super::inspection::daemon_facts(&bytes, "linux/amd64")
    }

    pub(super) fn validate_ready(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let observed = self.inspect(home, lock)?.ok_or(WorkerError::Corrupt(
            "owned worker disappeared before readiness",
        ))?;
        self.verify_container(home, record, &observed)?;
        if record.container_id.as_ref() != Some(&observed.id) || !observed.running {
            return Err(WorkerError::Corrupt(
                "owned worker identity/liveness changed before readiness",
            ));
        }
        let (version, _) = self.daemon_details(&observed.id, lock)?;
        if record.image != PINNED_WORKER_IMAGE
            || version != crate::protocol::EXPECTED_DAEMON_VERSION
        {
            return Err(WorkerError::Corrupt(
                "owned daemon version differs from the supported pinned image",
            ));
        }
        Ok(())
    }

    pub(super) fn status_details(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        observed: Option<&ContainerObservation>,
        details: &mut super::inspection::WorkerInspection,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        use super::inspection::{WorkerDiskUsage, WorkerObservation as Fact};
        let cache = self.verify_volume(home, record, lock)?;
        details.cache_present = Fact::measured(cache, "owned Docker volume labels");
        let mut cache_bytes = if cache {
            Fact::unavailable("worker stopped; cache volume is not mounted for inspection")
        } else {
            Fact::Absent
        };
        if let Some(observed) = observed {
            self.verify_container(home, record, observed)?;
            details.instance(
                observed,
                "Docker container inspect: configured image and resource limits",
            )?;
            let helper = self.run(&args(&["version", "--format", "{{.Client.Version}}"]), lock);
            details.helper_version.observed = match helper {
                Ok(bytes) => Fact::measured(
                    String::from_utf8(bytes)
                        .map_err(|_| WorkerError::Corrupt("Docker version is not UTF-8"))?
                        .trim()
                        .to_owned(),
                    "Docker client version",
                ),
                Err(error) => Fact::Unavailable(error.to_string()),
            };
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Disk {
                id: ContainerId,
                image: String,
                size: Option<u64>,
                mounts: Vec<serde_json::Value>,
            }
            let disk: Disk = serde_json::from_slice(&self.run(&args(&[
                "container", "inspect", "--size", "--format",
                r#"{"id":{{json .Id}},"image":{{json .Image}},"size":{{json .SizeRw}},"mounts":{{json .Mounts}}}"#, observed.id.as_str(),
            ]), lock)?)?;
            if disk.id != observed.id {
                return Err(WorkerError::Foreign(disk.id.as_str().to_owned()));
            }
            if cache
                && !disk.mounts.iter().any(|mount| {
                    mount["Type"] == "volume"
                        && mount["Name"] == home.cache_volume()
                        && mount["Destination"] == "/var/lib/buildkit"
                })
            {
                return Err(WorkerError::Foreign(
                    "owned cache volume is not the container cache mount".to_owned(),
                ));
            }
            details.image_id = Fact::measured(disk.image, "Docker container immutable image ID");
            details.disk.push(WorkerDiskUsage {
                scope: observed.id.as_str().to_owned(),
                accounting: "Docker writable-layer bytes; excludes image and cache volume",
                bytes: disk.size.map_or_else(
                    || Fact::unavailable("Docker omitted writable-layer accounting"),
                    |bytes| Fact::measured(bytes, "Docker scoped container inspect --size"),
                ),
            });
            if observed.running {
                match self.daemon_details(&observed.id, lock) {
                    Ok((version, target)) => {
                        details.daemon_version.observed =
                            Fact::measured(version, "BuildKit debug workers");
                        details.target.observed =
                            Fact::measured(target, "BuildKit advertised worker platforms");
                    }
                    Err(error) => {
                        details.daemon_version.observed = Fact::Unavailable(error.to_string());
                        details.target.observed = Fact::unavailable("daemon platform query failed");
                    }
                }
                if cache {
                    cache_bytes = match self.run(
                        &args(&[
                            "exec",
                            observed.id.as_str(),
                            "du",
                            "-sk",
                            "/var/lib/buildkit",
                        ]),
                        lock,
                    ) {
                        Ok(bytes) => Fact::measured(
                            super::inspection::du_bytes(&bytes)?,
                            "du -sk inside the existing owned container",
                        ),
                        Err(error) => Fact::Unavailable(error.to_string()),
                    };
                }
            } else {
                details.target.observed =
                    Fact::unavailable("worker stopped; daemon platforms not queried");
            }
        }
        details.disk.push(WorkerDiskUsage {
            scope: home.cache_volume(),
            accounting: "allocated cache bytes (1-KiB du units); live snapshot may change",
            bytes: cache_bytes,
        });
        Ok(())
    }
    /// The recorded durable binding is the authority: image, owned labels
    /// AND the resource limits. An unbound (legacy) record skips the
    /// resource comparison here and is re-bound from this verified
    /// observation by the caller.
    pub(super) fn verify_container(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        observed: &ContainerObservation,
    ) -> Result<(), WorkerError> {
        let resources_match = match (record.cpus, record.memory) {
            (Some(cpus), Some(memory)) => {
                observed.nano_cpus == nano_cpus(cpus) && observed.memory == memory
            }
            (None, None) => true,
            _ => return Err(WorkerError::Corrupt("worker resource binding is partial")),
        };
        if observed.image != record.image
            || !resources_match
            || !observed
                .labels
                .as_ref()
                .is_some_and(|labels| owned_labels(labels, home, record))
        {
            return Err(WorkerError::Foreign(observed.id.as_str().to_owned()));
        }
        Ok(())
    }
    pub(super) fn ensure_volume(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let name = home.cache_volume();
        if self.verify_volume(home, record, lock)? {
            return Ok(());
        }
        let mut arguments = args(&["volume", "create"]);
        add_labels(&mut arguments, home, record);
        arguments.push(name.into());
        self.run(&arguments, lock)?;
        if !self.verify_volume(home, record, lock)? {
            return Err(WorkerError::Corrupt("created worker volume disappeared"));
        }
        Ok(())
    }
    pub(super) fn verify_volume(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<bool, WorkerError> {
        let name = home.cache_volume();
        let found = self.run(
            &args(&[
                "volume",
                "ls",
                "--filter",
                &format!("name=^{name}$"),
                "--format",
                "{{.Name}}",
            ]),
            lock,
        )?;
        let found = std::str::from_utf8(&found)
            .map_err(|_| WorkerError::Corrupt("Docker emitted non-UTF-8 volume name"))?
            .trim();
        if found.is_empty() {
            return Ok(false);
        }
        if found != name {
            return Err(WorkerError::Foreign(found.to_owned()));
        }
        let bytes = self.run(
            &args(&[
                "volume",
                "inspect",
                "--format",
                r#"{"name":{{json .Name}},"labels":{{json .Labels}}}"#,
                &name,
            ]),
            lock,
        )?;
        let volume: VolumeObservation = serde_json::from_slice(&bytes)?;
        let owned = volume
            .labels
            .as_ref()
            .is_some_and(|labels| owned_labels(labels, home, record));
        if volume.name != name || !owned {
            return Err(WorkerError::Foreign(name));
        }
        Ok(true)
    }
    pub(super) fn create(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        options: &super::WorkerOptions,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<ContainerId, WorkerError> {
        let mut arguments = args(&[
            "container",
            "create",
            "--pull=never",
            "--name",
            &home.resource_name,
            "--privileged",
            "--cpus",
            &options.cpus.get().to_string(),
            "--memory",
            &options.memory.bytes().to_string(),
            "--mount",
            &format!(
                "type=volume,source={},target=/var/lib/buildkit",
                home.cache_volume()
            ),
            "--mount",
            &format!(
                "type=bind,source={},target=/run/gripsack",
                home.run_dir().display()
            ),
        ]);
        add_labels(&mut arguments, home, record);
        // SAFETY: getegid has no pointer/memory preconditions.
        let group = unsafe { libc::getegid() }.to_string();
        arguments.extend(args(&[
            PINNED_WORKER_IMAGE,
            "--addr",
            "unix:///run/gripsack/buildkitd.sock",
            "--group",
            &group,
            "--containerd-worker=false",
            "--oci-worker-platform",
            "linux/amd64",
            "--oci-worker-net=bridge",
            "--cdi-disabled",
        ]));
        let output = self.run(&arguments, lock)?;
        let id = std::str::from_utf8(&output)
            .map_err(|_| WorkerError::Corrupt("Docker emitted non-UTF-8 container ID"))?
            .trim();
        Ok(ContainerId::try_from(id.to_owned())?)
    }
    pub(super) fn start(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        self.run(&args(&["container", "start", id.as_str()]), lock)?;
        Ok(())
    }
    pub(super) fn stop(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        self.run(
            &args(&["container", "stop", "--timeout", "10", id.as_str()]),
            lock,
        )?;
        Ok(())
    }
    pub(super) fn remove_container(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        self.run(&args(&["container", "rm", id.as_str()]), lock)?;
        Ok(())
    }
    pub(super) fn remove_volume(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if !self.verify_volume(home, record, lock)? {
            return Ok(());
        }
        self.run(&args(&["volume", "rm", &home.cache_volume()]), lock)?;
        Ok(())
    }
}
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn owned_labels(
    labels: &BTreeMap<String, String>,
    home: &WorkerHome,
    record: &InstanceRecord,
) -> bool {
    labels
        .get(OWNER_LABEL)
        .is_some_and(|value| value == &record.owner.to_string())
        && labels
            .get(HOME_LABEL)
            .is_some_and(|value| value == &home.identity.to_string())
        && labels
            .get(PROFILE_LABEL)
            .is_some_and(|value| value == home.profile.as_str())
}
fn add_labels(arguments: &mut Vec<OsString>, home: &WorkerHome, record: &InstanceRecord) {
    for (key, value) in [
        (OWNER_LABEL, record.owner.to_string()),
        (HOME_LABEL, home.identity.to_string()),
        (PROFILE_LABEL, home.profile.as_str().to_owned()),
    ] {
        arguments.push("--label".into());
        arguments.push(format!("{key}={value}").into());
    }
}
